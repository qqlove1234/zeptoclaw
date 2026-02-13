//! Container-based agent proxy that spawns containers for each request
//!
//! This module provides the `ContainerAgentProxy` which runs agents in isolated
//! containers (Docker or Apple Container), enabling multi-user scenarios with
//! proper isolation.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::config::{Config, ContainerAgentBackend, ContainerAgentConfig};
use crate::error::{Result, ZeptoError};
use crate::session::SessionManager;

use super::ipc::{parse_marked_response, AgentRequest, AgentResponse, AgentResult};

const CONTAINER_WORKSPACE_DIR: &str = "/data/.zeptoclaw/workspace";
const CONTAINER_SESSIONS_DIR: &str = "/data/.zeptoclaw/sessions";
const CONTAINER_CONFIG_PATH: &str = "/data/.zeptoclaw/config.json";

/// Path inside the container where the env file is mounted (Apple Container only).
const CONTAINER_ENV_DIR: &str = "/tmp/zeptoclaw-env";

/// Resolved backend after auto-detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackend {
    Docker,
    #[cfg(target_os = "macos")]
    Apple,
}

impl std::fmt::Display for ResolvedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedBackend::Docker => write!(f, "docker"),
            #[cfg(target_os = "macos")]
            ResolvedBackend::Apple => write!(f, "apple-container"),
        }
    }
}

#[derive(Debug, Clone)]
struct ContainerInvocation {
    binary: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// Temp directory to clean up after container exits (Apple Container env file).
    temp_dir: Option<std::path::PathBuf>,
}

/// Proxy that spawns containers to process agent requests.
///
/// Each inbound message is processed in an isolated container, providing
/// security isolation for multi-user scenarios.
pub struct ContainerAgentProxy {
    config: Config,
    container_config: ContainerAgentConfig,
    bus: Arc<MessageBus>,
    session_manager: Option<SessionManager>,
    running: AtomicBool,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    resolved_backend: ResolvedBackend,
}

impl ContainerAgentProxy {
    /// Create a new container agent proxy with explicit resolved backend.
    pub fn new(config: Config, bus: Arc<MessageBus>, backend: ResolvedBackend) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let container_config = config.container_agent.clone();
        let session_manager = match SessionManager::new() {
            Ok(manager) => Some(manager),
            Err(e) => {
                warn!(
                    "Failed to initialize session manager for container agent proxy: {}",
                    e
                );
                None
            }
        };

        Self {
            config,
            container_config,
            bus,
            session_manager,
            running: AtomicBool::new(false),
            shutdown_tx,
            shutdown_rx,
            resolved_backend: backend,
        }
    }

    /// Return the resolved backend.
    pub fn backend(&self) -> ResolvedBackend {
        self.resolved_backend
    }

    /// Start the proxy loop, processing messages from the bus.
    pub async fn start(&self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(ZeptoError::Config(
                "Container agent proxy already running".into(),
            ));
        }

        info!(
            "Starting containerized agent proxy (backend={})",
            self.resolved_backend
        );

        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Container agent proxy shutting down");
                        break;
                    }
                }
                msg = self.bus.consume_inbound() => {
                    match msg {
                        Some(inbound) => {
                            let response = self.process_in_container(&inbound).await;
                            if let Err(e) = self.bus.publish_outbound(response).await {
                                error!("Failed to publish response: {}", e);
                            }
                        }
                        None => {
                            error!("Inbound channel closed");
                            break;
                        }
                    }
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Stop the proxy loop.
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Check if the proxy is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Process a message in a container.
    async fn process_in_container(&self, message: &InboundMessage) -> OutboundMessage {
        let request_id = Uuid::new_v4().to_string();
        let session_snapshot = self.load_session_snapshot(&message.session_key).await;

        let request = AgentRequest {
            request_id: request_id.clone(),
            message: message.clone(),
            agent_config: self.config.agents.defaults.clone(),
            session: session_snapshot,
        };

        match self.spawn_container(&request).await {
            Ok(response) => match response.result {
                AgentResult::Success { content, .. } => {
                    OutboundMessage::new(&message.channel, &message.chat_id, &content)
                }
                AgentResult::Error { message: err, .. } => OutboundMessage::new(
                    &message.channel,
                    &message.chat_id,
                    &format!("Error: {}", err),
                ),
            },
            Err(e) => {
                error!("Container execution failed: {}", e);
                OutboundMessage::new(
                    &message.channel,
                    &message.chat_id,
                    &format!("Container error: {}", e),
                )
            }
        }
    }

    async fn load_session_snapshot(&self, session_key: &str) -> Option<crate::session::Session> {
        let manager = self.session_manager.as_ref()?;

        match manager.get(session_key).await {
            Ok(session) => session,
            Err(e) => {
                warn!("Failed to load session snapshot for {}: {}", session_key, e);
                None
            }
        }
    }

    /// Spawn a container and communicate via stdin/stdout.
    async fn spawn_container(&self, request: &AgentRequest) -> Result<AgentResponse> {
        let config_root = dirs::home_dir().unwrap_or_default().join(".zeptoclaw");
        let workspace_dir = config_root.join("workspace");
        let sessions_dir = config_root.join("sessions");
        let config_path = config_root.join("config.json");

        tokio::fs::create_dir_all(&workspace_dir)
            .await
            .map_err(|e| ZeptoError::Config(format!("Failed to create workspace dir: {}", e)))?;
        tokio::fs::create_dir_all(&sessions_dir)
            .await
            .map_err(|e| ZeptoError::Config(format!("Failed to create sessions dir: {}", e)))?;
        tokio::fs::create_dir_all(&config_root)
            .await
            .map_err(|e| ZeptoError::Config(format!("Failed to create config dir: {}", e)))?;

        let invocation = match self.resolved_backend {
            ResolvedBackend::Docker => {
                self.build_docker_invocation(&workspace_dir, &sessions_dir, &config_path)
            }
            #[cfg(target_os = "macos")]
            ResolvedBackend::Apple => {
                self.build_apple_invocation(&workspace_dir, &sessions_dir, &config_path)
                    .await?
            }
        };

        debug!(
            request_id = %request.request_id,
            backend = %self.resolved_backend,
            image = %self.container_config.image,
            args_len = invocation.args.len(),
            env_len = invocation.env.len(),
            "Spawning containerized agent request"
        );

        let mut command = Command::new(&invocation.binary);
        command.args(&invocation.args);
        for (name, value) in &invocation.env {
            command.env(name, value);
        }

        let result = self.run_container_process(&mut command, request).await;

        // Clean up temp dir (Apple Container env file) regardless of outcome.
        if let Some(ref temp_dir) = invocation.temp_dir {
            if let Err(e) = tokio::fs::remove_dir_all(temp_dir).await {
                warn!("Failed to clean up temp env dir {:?}: {}", temp_dir, e);
            }
        }

        result
    }

    /// Run the container process, write request to stdin, and parse output.
    async fn run_container_process(
        &self,
        command: &mut Command,
        request: &AgentRequest,
    ) -> Result<AgentResponse> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ZeptoError::Config(format!("Failed to spawn container: {}", e)))?;

        // Write request to stdin
        let request_json = serde_json::to_string(request)
            .map_err(|e| ZeptoError::Config(format!("Failed to serialize request: {}", e)))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(request_json.as_bytes())
                .await
                .map_err(|e| ZeptoError::Config(format!("Failed to write to stdin: {}", e)))?;
            stdin.write_all(b"\n").await?;
            stdin.shutdown().await?;
        }

        // Wait for output with timeout
        let timeout = Duration::from_secs(self.container_config.timeout_secs);
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| ZeptoError::Config("Container timeout".into()))?
            .map_err(|e| ZeptoError::Config(format!("Container failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ZeptoError::Config(format!(
                "Container exited with code {:?}: {}",
                output.status.code(),
                stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_marked_response(&stdout)
            .ok_or_else(|| ZeptoError::Config("Failed to parse container response".into()))
    }

    /// Collect env var pairs to pass into the container.
    fn collect_env_vars(&self) -> Vec<(String, String)> {
        let mut env_vars = Vec::new();

        // Provider API keys
        if let Some(ref anthropic) = self.config.providers.anthropic {
            if let Some(ref key) = anthropic.api_key {
                if !key.trim().is_empty() {
                    env_vars.push((
                        "ZEPTOCLAW_PROVIDERS_ANTHROPIC_API_KEY".to_string(),
                        key.clone(),
                    ));
                }
            }
            if let Some(ref base) = anthropic.api_base {
                if !base.trim().is_empty() {
                    env_vars.push((
                        "ZEPTOCLAW_PROVIDERS_ANTHROPIC_API_BASE".to_string(),
                        base.clone(),
                    ));
                }
            }
        }
        if let Some(ref openai) = self.config.providers.openai {
            if let Some(ref key) = openai.api_key {
                if !key.trim().is_empty() {
                    env_vars.push((
                        "ZEPTOCLAW_PROVIDERS_OPENAI_API_KEY".to_string(),
                        key.clone(),
                    ));
                }
            }
            if let Some(ref base) = openai.api_base {
                if !base.trim().is_empty() {
                    env_vars.push((
                        "ZEPTOCLAW_PROVIDERS_OPENAI_API_BASE".to_string(),
                        base.clone(),
                    ));
                }
            }
        }
        if let Some(ref openrouter) = self.config.providers.openrouter {
            if let Some(ref key) = openrouter.api_key {
                if !key.trim().is_empty() {
                    env_vars.push((
                        "ZEPTOCLAW_PROVIDERS_OPENROUTER_API_KEY".to_string(),
                        key.clone(),
                    ));
                }
            }
            if let Some(ref base) = openrouter.api_base {
                if !base.trim().is_empty() {
                    env_vars.push((
                        "ZEPTOCLAW_PROVIDERS_OPENROUTER_API_BASE".to_string(),
                        base.clone(),
                    ));
                }
            }
        }

        // Container-internal paths
        env_vars.push(("HOME".to_string(), "/data".to_string()));
        env_vars.push((
            "ZEPTOCLAW_AGENTS_DEFAULTS_WORKSPACE".to_string(),
            CONTAINER_WORKSPACE_DIR.to_string(),
        ));

        env_vars
    }

    /// Build Docker invocation arguments.
    fn build_docker_invocation(
        &self,
        workspace_dir: &Path,
        sessions_dir: &Path,
        config_path: &Path,
    ) -> ContainerInvocation {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-i".to_string(),
            "--network".to_string(),
            self.container_config.network.clone(),
        ];
        let env_vars = self.collect_env_vars();

        // Resource limits
        if let Some(ref mem) = self.container_config.memory_limit {
            args.push("--memory".to_string());
            args.push(mem.clone());
        }
        if let Some(ref cpu) = self.container_config.cpu_limit {
            args.push("--cpus".to_string());
            args.push(cpu.clone());
        }

        // Volume mounts
        args.push("-v".to_string());
        args.push(format!(
            "{}:{}",
            workspace_dir.display(),
            CONTAINER_WORKSPACE_DIR
        ));
        args.push("-v".to_string());
        args.push(format!(
            "{}:{}",
            sessions_dir.display(),
            CONTAINER_SESSIONS_DIR
        ));
        if config_path.exists() {
            args.push("-v".to_string());
            args.push(format!(
                "{}:{}:ro",
                config_path.display(),
                CONTAINER_CONFIG_PATH
            ));
        }

        // Environment variables — Docker uses `-e NAME` with process env for secrets
        let mut process_env = Vec::new();
        for (name, value) in &env_vars {
            args.push("-e".to_string());
            args.push(name.clone());
            process_env.push((name.clone(), value.clone()));
        }

        // Extra mounts from config
        for mount in &self.container_config.extra_mounts {
            args.push("-v".to_string());
            args.push(mount.clone());
        }

        // Image and command
        args.push(self.container_config.image.clone());
        args.push("zeptoclaw".to_string());
        args.push("agent-stdin".to_string());

        ContainerInvocation {
            binary: self
                .container_config
                .docker_binary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("docker")
                .to_string(),
            args,
            env: process_env,
            temp_dir: None,
        }
    }

    /// Build Apple Container invocation arguments (macOS only).
    ///
    /// Key differences from Docker:
    /// - Binary: `container` not `docker`
    /// - RO mounts: `--mount type=bind,source=X,target=Y,readonly` (not `-v X:Y:ro`)
    /// - Env vars: `-e` flag is broken, use env file mount workaround
    /// - No `--memory`, `--cpus`, or `--network` flags
    /// - Needs explicit `--name` for container naming
    #[cfg(target_os = "macos")]
    async fn build_apple_invocation(
        &self,
        workspace_dir: &Path,
        sessions_dir: &Path,
        config_path: &Path,
    ) -> Result<ContainerInvocation> {
        let container_name = format!("zeptoclaw-{}", Uuid::new_v4());
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-i".to_string(),
            "--name".to_string(),
            container_name,
        ];

        // Volume mounts — RW mounts use -v, RO mounts use --mount with readonly
        args.push("-v".to_string());
        args.push(format!(
            "{}:{}",
            workspace_dir.display(),
            CONTAINER_WORKSPACE_DIR
        ));
        args.push("-v".to_string());
        args.push(format!(
            "{}:{}",
            sessions_dir.display(),
            CONTAINER_SESSIONS_DIR
        ));
        if config_path.exists() {
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target={},readonly",
                config_path.display(),
                CONTAINER_CONFIG_PATH
            ));
        }

        // Extra mounts from config
        for mount in &self.container_config.extra_mounts {
            args.push("-v".to_string());
            args.push(mount.clone());
        }

        // Env file workaround: Apple Container's -e flag is broken, so we write
        // env vars to a shell file, mount it read-only, and source it before exec.
        let env_vars = self.collect_env_vars();
        let temp_dir = tempfile::tempdir()
            .map_err(|e| ZeptoError::Config(format!("Failed to create temp dir for env: {}", e)))?;
        let env_file_path = temp_dir.path().join("env.sh");
        let env_content = generate_env_file_content(&env_vars);
        tokio::fs::write(&env_file_path, &env_content)
            .await
            .map_err(|e| ZeptoError::Config(format!("Failed to write env file: {}", e)))?;

        // Mount env dir read-only
        args.push("--mount".to_string());
        args.push(format!(
            "type=bind,source={},target={},readonly",
            temp_dir.path().display(),
            CONTAINER_ENV_DIR
        ));

        // Image
        args.push(self.container_config.image.clone());

        // Wrap command: source env file then exec zeptoclaw
        args.push("sh".to_string());
        args.push("-c".to_string());
        args.push(format!(
            ". {}/env.sh && exec zeptoclaw agent-stdin",
            CONTAINER_ENV_DIR
        ));

        // Keep temp_dir alive — `keep` prevents automatic cleanup on drop.
        let temp_path = temp_dir.keep();

        Ok(ContainerInvocation {
            binary: "container".to_string(),
            args,
            env: Vec::new(), // Env is passed via file mount, not process env
            temp_dir: Some(temp_path),
        })
    }
}

/// Generate shell-sourceable env file content.
///
/// Each variable is exported via `export NAME='VALUE'` with single quotes
/// escaped so that values containing special characters work correctly.
pub fn generate_env_file_content(env_vars: &[(String, String)]) -> String {
    let mut lines = Vec::with_capacity(env_vars.len() + 1);
    lines.push("#!/bin/sh".to_string());
    for (name, value) in env_vars {
        // Escape single quotes: replace ' with '\''
        let escaped = value.replace('\'', "'\\''");
        lines.push(format!("export {}='{}'", name, escaped));
    }
    lines.push(String::new()); // trailing newline
    lines.join("\n")
}

/// Resolve the container backend from config, performing auto-detection.
pub async fn resolve_backend(config: &ContainerAgentConfig) -> Result<ResolvedBackend> {
    match config.backend {
        ContainerAgentBackend::Docker => Ok(ResolvedBackend::Docker),
        #[cfg(target_os = "macos")]
        ContainerAgentBackend::Apple => Ok(ResolvedBackend::Apple),
        ContainerAgentBackend::Auto => auto_detect_backend().await,
    }
}

/// Auto-detect: on macOS try Apple Container first, then Docker.
async fn auto_detect_backend() -> Result<ResolvedBackend> {
    #[cfg(target_os = "macos")]
    {
        if is_apple_container_available().await {
            return Ok(ResolvedBackend::Apple);
        }
    }

    if is_docker_available().await {
        return Ok(ResolvedBackend::Docker);
    }

    Err(ZeptoError::Config(
        "No container backend available. Install Docker or Apple Container (macOS 15+).".into(),
    ))
}

/// Check if Docker is available and the daemon is running.
pub async fn is_docker_available() -> bool {
    tokio::process::Command::new("docker")
        .args(["info"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if Apple Container CLI is available (macOS only).
#[cfg(target_os = "macos")]
pub async fn is_apple_container_available() -> bool {
    // Check that the `container` binary exists and responds to --version
    let version_ok = tokio::process::Command::new("container")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if !version_ok {
        return false;
    }

    // Also verify that `container run` is available via --help
    tokio::process::Command::new("container")
        .args(["run", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;
    use tokio::time::{sleep, timeout};

    #[test]
    fn test_container_agent_proxy_creation() {
        let config = Config::default();
        let bus = Arc::new(MessageBus::new());
        let proxy = ContainerAgentProxy::new(config, bus, ResolvedBackend::Docker);

        assert!(!proxy.is_running());
        assert_eq!(proxy.backend(), ResolvedBackend::Docker);
    }

    #[test]
    fn test_build_docker_invocation_mounts_expected_paths_and_hides_secrets() {
        let mut config = Config::default();
        config.container_agent.image = "zeptoclaw:test".to_string();
        config.providers.anthropic = Some(ProviderConfig {
            api_key: Some("secret-anthropic-key".to_string()),
            ..Default::default()
        });

        let bus = Arc::new(MessageBus::new());
        let proxy = ContainerAgentProxy::new(config, bus, ResolvedBackend::Docker);

        let temp_root =
            std::env::temp_dir().join(format!("zeptoclaw-proxy-test-{}", Uuid::new_v4()));
        let workspace_dir = temp_root.join("workspace");
        let sessions_dir = temp_root.join("sessions");
        let config_path = temp_root.join("config.json");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(&config_path, "{}").unwrap();

        let invocation = proxy.build_docker_invocation(&workspace_dir, &sessions_dir, &config_path);

        assert_eq!(invocation.binary, "docker");

        let workspace_mount = format!("{}:{}", workspace_dir.display(), CONTAINER_WORKSPACE_DIR);
        let sessions_mount = format!("{}:{}", sessions_dir.display(), CONTAINER_SESSIONS_DIR);
        let config_mount = format!("{}:{}:ro", config_path.display(), CONTAINER_CONFIG_PATH);

        assert!(has_arg_pair(&invocation.args, "-v", &workspace_mount));
        assert!(has_arg_pair(&invocation.args, "-v", &sessions_mount));
        assert!(has_arg_pair(&invocation.args, "-v", &config_mount));
        assert!(has_arg_pair(
            &invocation.args,
            "-e",
            "ZEPTOCLAW_PROVIDERS_ANTHROPIC_API_KEY"
        ));
        assert!(!invocation
            .args
            .iter()
            .any(|arg| arg.contains("secret-anthropic-key")));
        assert!(invocation.env.iter().any(|(name, value)| {
            name == "ZEPTOCLAW_PROVIDERS_ANTHROPIC_API_KEY" && value == "secret-anthropic-key"
        }));

        assert!(invocation.temp_dir.is_none());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[tokio::test]
    async fn test_stop_unblocks_start_loop() {
        let config = Config::default();
        let bus = Arc::new(MessageBus::new());
        let proxy = Arc::new(ContainerAgentProxy::new(
            config,
            bus,
            ResolvedBackend::Docker,
        ));

        let proxy_task = Arc::clone(&proxy);
        let handle = tokio::spawn(async move { proxy_task.start().await });

        for _ in 0..50 {
            if proxy.is_running() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }

        proxy.stop();
        let joined = timeout(Duration::from_secs(2), handle)
            .await
            .expect("proxy task should stop");
        joined
            .expect("proxy task should join")
            .expect("proxy start should exit cleanly");
        assert!(!proxy.is_running());
    }

    #[test]
    fn test_generate_env_file_content_basic() {
        let vars = vec![
            ("FOO".to_string(), "bar".to_string()),
            ("KEY".to_string(), "value with spaces".to_string()),
        ];
        let content = generate_env_file_content(&vars);
        assert!(content.starts_with("#!/bin/sh\n"));
        assert!(content.contains("export FOO='bar'"));
        assert!(content.contains("export KEY='value with spaces'"));
    }

    #[test]
    fn test_generate_env_file_content_special_chars() {
        let vars = vec![
            (
                "QUOTED".to_string(),
                "it's a \"test\" with $var".to_string(),
            ),
            ("EMPTY".to_string(), String::new()),
        ];
        let content = generate_env_file_content(&vars);
        // Single quotes inside value should be escaped
        assert!(content.contains("export QUOTED='it'\\''s a \"test\" with $var'"));
        assert!(content.contains("export EMPTY=''"));
    }

    #[test]
    fn test_collect_env_vars_includes_internal_paths() {
        let config = Config::default();
        let bus = Arc::new(MessageBus::new());
        let proxy = ContainerAgentProxy::new(config, bus, ResolvedBackend::Docker);

        let vars = proxy.collect_env_vars();
        assert!(vars.iter().any(|(k, v)| k == "HOME" && v == "/data"));
        assert!(vars.iter().any(
            |(k, v)| k == "ZEPTOCLAW_AGENTS_DEFAULTS_WORKSPACE" && v == CONTAINER_WORKSPACE_DIR
        ));
    }

    #[test]
    fn test_build_docker_invocation_respects_binary_override() {
        let mut config = Config::default();
        config.container_agent.docker_binary = Some("/tmp/mock-docker".to_string());
        let bus = Arc::new(MessageBus::new());
        let proxy = ContainerAgentProxy::new(config, bus, ResolvedBackend::Docker);

        let temp_root =
            std::env::temp_dir().join(format!("zeptoclaw-binary-test-{}", Uuid::new_v4()));
        let workspace_dir = temp_root.join("workspace");
        let sessions_dir = temp_root.join("sessions");
        let config_path = temp_root.join("config.json");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(&config_path, "{}").unwrap();

        let invocation = proxy.build_docker_invocation(&workspace_dir, &sessions_dir, &config_path);
        assert_eq!(invocation.binary, "/tmp/mock-docker");

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_proxy_end_to_end_with_mocked_docker_binary() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root = tempfile::tempdir().unwrap();
        let script_path = temp_root.path().join("mock-docker.sh");
        let script = r#"#!/bin/sh
cat >/dev/null
cat <<'EOF'
<<<AGENT_RESPONSE_START>>>
{"request_id":"mock-req","result":{"Success":{"content":"mock response","session":null}}}
<<<AGENT_RESPONSE_END>>>
EOF
"#;
        std::fs::write(&script_path, script).unwrap();
        let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions).unwrap();

        let mut config = Config::default();
        config.container_agent.image = "mock-image:latest".to_string();
        config.container_agent.timeout_secs = 5;
        config.container_agent.docker_binary = Some(script_path.to_string_lossy().to_string());

        let bus = Arc::new(MessageBus::new());
        let proxy = Arc::new(ContainerAgentProxy::new(
            config,
            bus.clone(),
            ResolvedBackend::Docker,
        ));

        let proxy_task = Arc::clone(&proxy);
        let handle = tokio::spawn(async move { proxy_task.start().await });

        let inbound = InboundMessage::new("test", "u1", "chat1", "hello");
        bus.publish_inbound(inbound).await.unwrap();

        let outbound = timeout(Duration::from_secs(2), bus.consume_outbound())
            .await
            .expect("should receive outbound within timeout")
            .expect("outbound should be present");
        assert_eq!(outbound.channel, "test");
        assert_eq!(outbound.chat_id, "chat1");
        assert_eq!(outbound.content, "mock response");

        proxy.stop();
        timeout(Duration::from_secs(2), handle)
            .await
            .expect("proxy should stop quickly")
            .expect("proxy task join should succeed")
            .expect("proxy start should return ok");
    }

    #[test]
    fn test_container_agent_backend_serde_roundtrip() {
        // Auto
        let json = serde_json::to_string(&ContainerAgentBackend::Auto).unwrap();
        assert_eq!(json, "\"auto\"");
        let back: ContainerAgentBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ContainerAgentBackend::Auto);

        // Docker
        let json = serde_json::to_string(&ContainerAgentBackend::Docker).unwrap();
        assert_eq!(json, "\"docker\"");
        let back: ContainerAgentBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ContainerAgentBackend::Docker);

        // Apple (macOS only)
        #[cfg(target_os = "macos")]
        {
            let json = serde_json::to_string(&ContainerAgentBackend::Apple).unwrap();
            assert_eq!(json, "\"apple\"");
            let back: ContainerAgentBackend = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ContainerAgentBackend::Apple);
        }
    }

    fn has_arg_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == flag && window[1] == value)
    }
}
