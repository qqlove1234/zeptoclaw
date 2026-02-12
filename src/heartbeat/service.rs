//! Heartbeat service implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::bus::{InboundMessage, MessageBus};
use crate::error::Result;

/// Prompt sent to the agent when heartbeat is triggered.
pub const HEARTBEAT_PROMPT: &str = r#"Read HEARTBEAT.md in your workspace (if it exists).
Follow any actionable items listed there.
If nothing needs attention, reply with: HEARTBEAT_OK"#;

/// Background service that periodically enqueues heartbeat prompts.
pub struct HeartbeatService {
    file_path: PathBuf,
    interval: Duration,
    bus: Arc<MessageBus>,
    running: Arc<RwLock<bool>>,
    chat_id: String,
}

impl HeartbeatService {
    /// Create a new heartbeat service.
    pub fn new(
        file_path: PathBuf,
        interval_secs: u64,
        bus: Arc<MessageBus>,
        chat_id: &str,
    ) -> Self {
        Self {
            file_path,
            interval: Duration::from_secs(interval_secs.max(30)),
            bus,
            running: Arc::new(RwLock::new(false)),
            chat_id: chat_id.to_string(),
        }
    }

    /// Start heartbeat loop in the background.
    pub async fn start(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            if *running {
                warn!("Heartbeat service already running");
                return Ok(());
            }
            *running = true;
        }

        let file_path = self.file_path.clone();
        let interval_duration = self.interval;
        let bus = Arc::clone(&self.bus);
        let running = Arc::clone(&self.running);
        let chat_id = self.chat_id.clone();

        info!(
            "Heartbeat service started (interval={}s, file={:?})",
            interval_duration.as_secs(),
            file_path
        );

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval_duration);
            ticker.tick().await;

            loop {
                ticker.tick().await;

                if !*running.read().await {
                    info!("Heartbeat service stopped");
                    break;
                }

                if let Err(e) = Self::tick(&file_path, &bus, &chat_id).await {
                    error!("Heartbeat tick failed: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Stop heartbeat loop.
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    /// Trigger heartbeat immediately.
    pub async fn trigger_now(&self) -> Result<()> {
        Self::tick(&self.file_path, &self.bus, &self.chat_id).await
    }

    /// Returns whether service is running.
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Whether heartbeat content is actionable.
    pub fn is_empty(content: &str) -> bool {
        for raw in content.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("<!--") {
                continue;
            }
            if line == "- [ ]" || line == "* [ ]" {
                continue;
            }
            return false;
        }
        true
    }

    async fn tick(file_path: &PathBuf, bus: &MessageBus, chat_id: &str) -> Result<()> {
        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("Heartbeat file missing at {:?}, skipping tick", file_path);
                return Ok(());
            }
            Err(e) => {
                warn!("Failed to read heartbeat file {:?}: {}", file_path, e);
                return Ok(());
            }
        };

        if Self::is_empty(&content) {
            debug!("Heartbeat file has no actionable content");
            return Ok(());
        }

        let message = InboundMessage::new("heartbeat", "system", chat_id, HEARTBEAT_PROMPT);
        bus.publish_inbound(message).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_empty_true() {
        assert!(HeartbeatService::is_empty(""));
        assert!(HeartbeatService::is_empty("# Header\n## Tasks"));
        assert!(HeartbeatService::is_empty("<!-- comment -->\n\n- [ ]"));
    }

    #[test]
    fn test_is_empty_false() {
        assert!(!HeartbeatService::is_empty("Check orders"));
        assert!(!HeartbeatService::is_empty("- [x] Done"));
        assert!(!HeartbeatService::is_empty("# Header\n- Send alert"));
    }
}
