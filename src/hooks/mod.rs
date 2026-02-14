//! Hook system for ZeptoClaw agent loop.
//!
//! Config-driven hooks that fire at specific points in the agent loop:
//!
//! - `before_tool` — before tool execution (can log or block)
//! - `after_tool` — after tool execution (can log)
//! - `on_error` — when a tool fails (can log)
//!
//! # Configuration
//!
//! ```json
//! {
//!     "hooks": {
//!         "enabled": true,
//!         "before_tool": [
//!             { "action": "log", "tools": ["shell"], "level": "warn" },
//!             { "action": "block", "tools": ["shell"], "channels": ["telegram"], "message": "Shell disabled on Telegram" }
//!         ],
//!         "after_tool": [
//!             { "action": "log", "tools": ["*"], "level": "info" }
//!         ],
//!         "on_error": [
//!             { "action": "log", "level": "error" }
//!         ]
//!     }
//! }
//! ```
//!
//! # Example
//!
//! ```rust
//! use zeptoclaw::hooks::{HooksConfig, HookEngine, HookResult, HookAction, HookRule};
//!
//! let config = HooksConfig {
//!     enabled: true,
//!     before_tool: vec![HookRule {
//!         action: HookAction::Block,
//!         tools: vec!["shell".to_string()],
//!         channels: vec!["telegram".to_string()],
//!         message: Some("Shell disabled on Telegram".to_string()),
//!         ..Default::default()
//!     }],
//!     ..Default::default()
//! };
//! let engine = HookEngine::new(config);
//! let result = engine.before_tool("shell", &serde_json::json!({}), "telegram", "chat-1");
//! assert!(matches!(result, HookResult::Block(_)));
//! ```

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::bus::{MessageBus, OutboundMessage};

// ---------------------------------------------------------------------------
// Hook action enum
// ---------------------------------------------------------------------------

/// What a hook rule does when triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookAction {
    /// Log the event via tracing.
    Log,
    /// Block the tool from executing (before_tool only).
    Block,
    /// Send a notification message via the message bus.
    Notify,
}

// ---------------------------------------------------------------------------
// Hook rule
// ---------------------------------------------------------------------------

/// A single hook rule that matches tool calls and performs an action.
///
/// Rules are evaluated in order. For `before_tool`, the first `Block` rule
/// that matches wins. `Log` rules always execute (no short-circuit).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HookRule {
    /// Action to perform.
    pub action: HookAction,
    /// Tool names to match. `["*"]` matches all tools. Empty = match none.
    pub tools: Vec<String>,
    /// Channel names to match. Empty = match all channels.
    pub channels: Vec<String>,
    /// Log level for `Log` action (trace/debug/info/warn/error).
    pub level: Option<String>,
    /// Custom message for `Block` action.
    pub message: Option<String>,
    /// Optional target channel name for `Notify` action.
    /// Falls back to current tool call channel when unset.
    pub channel: Option<String>,
    /// Optional target chat ID for `Notify` action.
    /// Falls back to current tool call chat_id when unset.
    pub chat_id: Option<String>,
}

impl Default for HookRule {
    fn default() -> Self {
        Self {
            action: HookAction::Log,
            tools: vec![],
            channels: vec![],
            level: None,
            message: None,
            channel: None,
            chat_id: None,
        }
    }
}

impl HookRule {
    /// Check if this rule matches the given tool name.
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        self.tools.iter().any(|t| t == "*" || t == tool_name)
    }

    /// Check if this rule matches the given channel name.
    /// Empty channels list means match all.
    pub fn matches_channel(&self, channel_name: &str) -> bool {
        self.channels.is_empty() || self.channels.iter().any(|c| c == "*" || c == channel_name)
    }
}

// ---------------------------------------------------------------------------
// Hooks config
// ---------------------------------------------------------------------------

/// Hooks configuration for `config.json`.
///
/// Controls the hook system that fires at specific points in the agent loop.
///
/// # Defaults
///
/// - `enabled`: `false`
/// - All rule lists: empty
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    /// Master switch for hooks.
    pub enabled: bool,
    /// Rules evaluated before each tool execution.
    pub before_tool: Vec<HookRule>,
    /// Rules evaluated after each tool execution.
    pub after_tool: Vec<HookRule>,
    /// Rules evaluated when a tool returns an error.
    pub on_error: Vec<HookRule>,
}

// ---------------------------------------------------------------------------
// Hook result
// ---------------------------------------------------------------------------

/// Result of evaluating before_tool hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookResult {
    /// Allow the tool to execute.
    Continue,
    /// Block the tool with the given message.
    Block(String),
}

// ---------------------------------------------------------------------------
// Hook engine
// ---------------------------------------------------------------------------

/// Runtime hook engine that evaluates rules from HooksConfig.
///
/// Created once per agent loop iteration and called at 3 points:
/// 1. `before_tool` — before approval gate + tool execution
/// 2. `after_tool` — after successful tool execution
/// 3. `on_error` — after failed tool execution
pub struct HookEngine {
    config: HooksConfig,
    bus: Option<Arc<MessageBus>>,
}

impl HookEngine {
    /// Create a new HookEngine from configuration.
    pub fn new(config: HooksConfig) -> Self {
        Self { config, bus: None }
    }

    /// Attach a message bus for `notify` actions.
    pub fn with_bus(mut self, bus: Arc<MessageBus>) -> Self {
        self.bus = Some(bus);
        self
    }

    fn resolve_notify_target(
        rule: &HookRule,
        current_channel: &str,
        current_chat_id: &str,
    ) -> Option<(String, String)> {
        let target_channel = rule
            .channel
            .as_deref()
            .unwrap_or(current_channel)
            .trim()
            .to_string();
        let target_chat_id = rule
            .chat_id
            .as_deref()
            .unwrap_or(current_chat_id)
            .trim()
            .to_string();

        if target_channel.is_empty() || target_chat_id.is_empty() {
            return None;
        }

        Some((target_channel, target_chat_id))
    }

    fn emit_notify(
        &self,
        hook: &str,
        tool_name: &str,
        rule: &HookRule,
        current_channel: &str,
        current_chat_id: &str,
        message: String,
    ) {
        let Some(bus) = self.bus.as_ref() else {
            tracing::debug!(
                hook = hook,
                tool = tool_name,
                "Hook notify skipped: message bus not configured"
            );
            return;
        };

        let Some((target_channel, target_chat_id)) =
            Self::resolve_notify_target(rule, current_channel, current_chat_id)
        else {
            tracing::warn!(
                hook = hook,
                tool = tool_name,
                channel = current_channel,
                chat_id = current_chat_id,
                "Hook notify skipped: missing channel/chat_id target"
            );
            return;
        };

        let outbound = OutboundMessage::new(&target_channel, &target_chat_id, &message);
        match bus.try_publish_outbound(outbound) {
            Ok(()) => tracing::info!(
                hook = hook,
                tool = tool_name,
                target_channel = %target_channel,
                target_chat_id = %target_chat_id,
                "Hook notify dispatched"
            ),
            Err(error) => tracing::warn!(
                hook = hook,
                tool = tool_name,
                target_channel = %target_channel,
                target_chat_id = %target_chat_id,
                error = %error,
                "Hook notify failed to publish"
            ),
        }
    }

    /// Evaluate before_tool hooks. Returns Block if any matching rule blocks.
    ///
    /// Rules are evaluated in order. `Log` rules execute without stopping.
    /// The first `Block` rule that matches returns immediately.
    pub fn before_tool(
        &self,
        tool_name: &str,
        _args: &serde_json::Value,
        channel: &str,
        chat_id: &str,
    ) -> HookResult {
        if !self.config.enabled {
            return HookResult::Continue;
        }

        for rule in &self.config.before_tool {
            if !rule.matches_tool(tool_name) || !rule.matches_channel(channel) {
                continue;
            }

            match rule.action {
                HookAction::Log => {
                    let level = rule.level.as_deref().unwrap_or("info");
                    match level {
                        "error" => tracing::error!(
                            hook = "before_tool",
                            tool = tool_name,
                            channel = channel,
                            "Hook: tool call"
                        ),
                        "warn" => tracing::warn!(
                            hook = "before_tool",
                            tool = tool_name,
                            channel = channel,
                            "Hook: tool call"
                        ),
                        "debug" => tracing::debug!(
                            hook = "before_tool",
                            tool = tool_name,
                            channel = channel,
                            "Hook: tool call"
                        ),
                        "trace" => tracing::trace!(
                            hook = "before_tool",
                            tool = tool_name,
                            channel = channel,
                            "Hook: tool call"
                        ),
                        _ => tracing::info!(
                            hook = "before_tool",
                            tool = tool_name,
                            channel = channel,
                            "Hook: tool call"
                        ),
                    }
                }
                HookAction::Block => {
                    let msg = rule
                        .message
                        .clone()
                        .unwrap_or_else(|| format!("Tool '{}' blocked by hook", tool_name));
                    tracing::info!(
                        hook = "before_tool",
                        tool = tool_name,
                        channel = channel,
                        "Hook: blocking tool"
                    );
                    return HookResult::Block(msg);
                }
                HookAction::Notify => {
                    let message = rule.message.clone().unwrap_or_else(|| {
                        format!(
                            "Hook notify (before_tool): tool '{}' called in {}:{}",
                            tool_name, channel, chat_id
                        )
                    });
                    self.emit_notify("before_tool", tool_name, rule, channel, chat_id, message);
                }
            }
        }

        HookResult::Continue
    }

    /// Evaluate after_tool hooks (logging only, no blocking).
    pub fn after_tool(
        &self,
        tool_name: &str,
        _result: &str,
        elapsed: std::time::Duration,
        channel: &str,
        chat_id: &str,
    ) {
        if !self.config.enabled {
            return;
        }

        for rule in &self.config.after_tool {
            if !rule.matches_tool(tool_name) || !rule.matches_channel(channel) {
                continue;
            }

            match rule.action {
                HookAction::Log => {
                    let ms = elapsed.as_millis();
                    let level = rule.level.as_deref().unwrap_or("info");
                    match level {
                        "error" => {
                            tracing::error!(hook = "after_tool", tool = tool_name, latency_ms = %ms, "Hook: tool completed")
                        }
                        "warn" => {
                            tracing::warn!(hook = "after_tool", tool = tool_name, latency_ms = %ms, "Hook: tool completed")
                        }
                        "debug" => {
                            tracing::debug!(hook = "after_tool", tool = tool_name, latency_ms = %ms, "Hook: tool completed")
                        }
                        _ => {
                            tracing::info!(hook = "after_tool", tool = tool_name, latency_ms = %ms, "Hook: tool completed")
                        }
                    }
                }
                HookAction::Block => {} // Block is a no-op in after_tool
                HookAction::Notify => {
                    let ms = elapsed.as_millis();
                    let message = rule.message.clone().unwrap_or_else(|| {
                        format!(
                            "Hook notify (after_tool): tool '{}' succeeded in {}ms ({}:{})",
                            tool_name, ms, channel, chat_id
                        )
                    });
                    self.emit_notify("after_tool", tool_name, rule, channel, chat_id, message);
                }
            }
        }
    }

    /// Evaluate on_error hooks (logging only, no blocking).
    pub fn on_error(&self, tool_name: &str, error: &str, channel: &str, chat_id: &str) {
        if !self.config.enabled {
            return;
        }

        for rule in &self.config.on_error {
            if !rule.matches_tool(tool_name) || !rule.matches_channel(channel) {
                continue;
            }

            match rule.action {
                HookAction::Log => {
                    let level = rule.level.as_deref().unwrap_or("error");
                    match level {
                        "warn" => tracing::warn!(
                            hook = "on_error",
                            tool = tool_name,
                            error = error,
                            "Hook: tool error"
                        ),
                        "debug" => tracing::debug!(
                            hook = "on_error",
                            tool = tool_name,
                            error = error,
                            "Hook: tool error"
                        ),
                        _ => tracing::error!(
                            hook = "on_error",
                            tool = tool_name,
                            error = error,
                            "Hook: tool error"
                        ),
                    }
                }
                HookAction::Block => {} // Block is a no-op in on_error
                HookAction::Notify => {
                    let message = rule.message.clone().unwrap_or_else(|| {
                        format!(
                            "Hook notify (on_error): tool '{}' failed: {} ({}:{})",
                            tool_name, error, channel, chat_id
                        )
                    });
                    self.emit_notify("on_error", tool_name, rule, channel, chat_id, message);
                }
            }
        }
    }

    /// Whether hooks are enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- HooksConfig defaults ----

    #[test]
    fn test_hooks_config_default() {
        let config = HooksConfig::default();
        assert!(!config.enabled);
        assert!(config.before_tool.is_empty());
        assert!(config.after_tool.is_empty());
        assert!(config.on_error.is_empty());
    }

    #[test]
    fn test_hooks_config_deserialize() {
        let json = r#"{
            "enabled": true,
            "before_tool": [
                { "action": "log", "tools": ["shell"], "level": "warn" }
            ]
        }"#;
        let config: HooksConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.before_tool.len(), 1);
        assert_eq!(config.before_tool[0].action, HookAction::Log);
    }

    #[test]
    fn test_hooks_config_serialization_roundtrip() {
        let config = HooksConfig {
            enabled: true,
            before_tool: vec![HookRule {
                action: HookAction::Block,
                tools: vec!["shell".to_string()],
                channels: vec!["telegram".to_string()],
                message: Some("blocked".to_string()),
                ..Default::default()
            }],
            after_tool: vec![HookRule {
                action: HookAction::Log,
                tools: vec!["*".to_string()],
                ..Default::default()
            }],
            on_error: vec![],
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: HooksConfig = serde_json::from_str(&json).unwrap();
        assert!(deserialized.enabled);
        assert_eq!(deserialized.before_tool.len(), 1);
        assert_eq!(deserialized.after_tool.len(), 1);
    }

    // ---- HookRule matching ----

    #[test]
    fn test_hook_rule_matches_tool() {
        let rule = HookRule {
            tools: vec!["shell".to_string()],
            ..Default::default()
        };
        assert!(rule.matches_tool("shell"));
        assert!(!rule.matches_tool("echo"));
    }

    #[test]
    fn test_hook_rule_wildcard_matches_all() {
        let rule = HookRule {
            tools: vec!["*".to_string()],
            ..Default::default()
        };
        assert!(rule.matches_tool("shell"));
        assert!(rule.matches_tool("echo"));
        assert!(rule.matches_tool("anything"));
    }

    #[test]
    fn test_hook_rule_empty_tools_matches_none() {
        let rule = HookRule::default();
        assert!(!rule.matches_tool("shell"));
        assert!(!rule.matches_tool("anything"));
    }

    #[test]
    fn test_hook_rule_matches_channel() {
        let rule = HookRule {
            channels: vec!["telegram".to_string()],
            ..Default::default()
        };
        assert!(rule.matches_channel("telegram"));
        assert!(!rule.matches_channel("discord"));
    }

    #[test]
    fn test_hook_rule_empty_channels_matches_all() {
        let rule = HookRule::default();
        assert!(rule.matches_channel("telegram"));
        assert!(rule.matches_channel("discord"));
        assert!(rule.matches_channel("cli"));
    }

    #[test]
    fn test_hook_rule_channel_wildcard() {
        let rule = HookRule {
            channels: vec!["*".to_string()],
            ..Default::default()
        };
        assert!(rule.matches_channel("telegram"));
        assert!(rule.matches_channel("cli"));
    }

    // ---- HookEngine ----

    #[test]
    fn test_hook_engine_disabled_does_nothing() {
        let config = HooksConfig::default();
        let engine = HookEngine::new(config);
        let result = engine.before_tool("shell", &serde_json::json!({}), "telegram", "chat1");
        assert_eq!(result, HookResult::Continue);
    }

    #[test]
    fn test_hook_engine_before_tool_log() {
        let config = HooksConfig {
            enabled: true,
            before_tool: vec![HookRule {
                action: HookAction::Log,
                tools: vec!["shell".to_string()],
                level: Some("warn".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config);
        let result = engine.before_tool("shell", &serde_json::json!({"cmd": "ls"}), "cli", "cli");
        assert_eq!(result, HookResult::Continue);
    }

    #[test]
    fn test_hook_engine_before_tool_block() {
        let config = HooksConfig {
            enabled: true,
            before_tool: vec![HookRule {
                action: HookAction::Block,
                tools: vec!["shell".to_string()],
                channels: vec!["telegram".to_string()],
                message: Some("Shell disabled on Telegram".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config);

        // Should block shell on telegram
        let result = engine.before_tool("shell", &serde_json::json!({}), "telegram", "chat1");
        assert!(matches!(result, HookResult::Block(_)));
        if let HookResult::Block(msg) = result {
            assert_eq!(msg, "Shell disabled on Telegram");
        }

        // Should NOT block shell on CLI
        let result = engine.before_tool("shell", &serde_json::json!({}), "cli", "chat1");
        assert_eq!(result, HookResult::Continue);

        // Should NOT block echo on telegram
        let result = engine.before_tool("echo", &serde_json::json!({}), "telegram", "chat1");
        assert_eq!(result, HookResult::Continue);
    }

    #[test]
    fn test_hook_engine_before_tool_block_default_message() {
        let config = HooksConfig {
            enabled: true,
            before_tool: vec![HookRule {
                action: HookAction::Block,
                tools: vec!["shell".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config);
        let result = engine.before_tool("shell", &serde_json::json!({}), "cli", "chat1");
        if let HookResult::Block(msg) = result {
            assert!(msg.contains("shell"));
            assert!(msg.contains("blocked by hook"));
        } else {
            panic!("Expected Block");
        }
    }

    #[test]
    fn test_hook_engine_multiple_rules_first_block_wins() {
        let config = HooksConfig {
            enabled: true,
            before_tool: vec![
                HookRule {
                    action: HookAction::Log,
                    tools: vec!["*".to_string()],
                    level: Some("info".to_string()),
                    ..Default::default()
                },
                HookRule {
                    action: HookAction::Block,
                    tools: vec!["shell".to_string()],
                    message: Some("blocked".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let engine = HookEngine::new(config);
        let result = engine.before_tool("shell", &serde_json::json!({}), "cli", "chat1");
        assert!(matches!(result, HookResult::Block(_)));
    }

    #[test]
    fn test_hook_engine_after_tool() {
        let config = HooksConfig {
            enabled: true,
            after_tool: vec![HookRule {
                action: HookAction::Log,
                tools: vec!["*".to_string()],
                level: Some("info".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config);
        engine.after_tool(
            "shell",
            "result text",
            std::time::Duration::from_millis(50),
            "cli",
            "chat1",
        );
    }

    #[test]
    fn test_hook_engine_on_error() {
        let config = HooksConfig {
            enabled: true,
            on_error: vec![HookRule {
                action: HookAction::Log,
                tools: vec!["*".to_string()],
                level: Some("error".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config);
        engine.on_error("shell", "command not found", "cli", "chat1");
    }

    #[test]
    fn test_hook_engine_is_enabled() {
        let enabled = HookEngine::new(HooksConfig {
            enabled: true,
            ..Default::default()
        });
        assert!(enabled.is_enabled());

        let disabled = HookEngine::new(HooksConfig::default());
        assert!(!disabled.is_enabled());
    }

    #[tokio::test]
    async fn test_hook_engine_notify_before_tool_publishes_message() {
        use tokio::time::{timeout, Duration};

        let bus = Arc::new(MessageBus::new());
        let config = HooksConfig {
            enabled: true,
            before_tool: vec![HookRule {
                action: HookAction::Notify,
                tools: vec!["shell".to_string()],
                message: Some("manual approval required".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config).with_bus(Arc::clone(&bus));

        let result = engine.before_tool("shell", &serde_json::json!({}), "telegram", "chat77");
        assert_eq!(result, HookResult::Continue);

        let outbound = timeout(Duration::from_millis(300), bus.consume_outbound())
            .await
            .expect("timed out waiting for outbound message")
            .expect("expected outbound message");
        assert_eq!(outbound.channel, "telegram");
        assert_eq!(outbound.chat_id, "chat77");
        assert_eq!(outbound.content, "manual approval required");
    }

    #[tokio::test]
    async fn test_hook_engine_notify_after_tool_uses_override_target() {
        use tokio::time::{timeout, Duration};

        let bus = Arc::new(MessageBus::new());
        let config = HooksConfig {
            enabled: true,
            after_tool: vec![HookRule {
                action: HookAction::Notify,
                tools: vec!["*".to_string()],
                channel: Some("slack".to_string()),
                chat_id: Some("ops-room".to_string()),
                message: Some("tool completed".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config).with_bus(Arc::clone(&bus));

        engine.after_tool(
            "echo",
            "ok",
            std::time::Duration::from_millis(15),
            "telegram",
            "chat77",
        );

        let outbound = timeout(Duration::from_millis(300), bus.consume_outbound())
            .await
            .expect("timed out waiting for outbound message")
            .expect("expected outbound message");
        assert_eq!(outbound.channel, "slack");
        assert_eq!(outbound.chat_id, "ops-room");
        assert_eq!(outbound.content, "tool completed");
    }

    #[tokio::test]
    async fn test_hook_engine_notify_on_error_default_message_contains_error() {
        use tokio::time::{timeout, Duration};

        let bus = Arc::new(MessageBus::new());
        let config = HooksConfig {
            enabled: true,
            on_error: vec![HookRule {
                action: HookAction::Notify,
                tools: vec!["shell".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = HookEngine::new(config).with_bus(Arc::clone(&bus));

        engine.on_error("shell", "permission denied", "telegram", "chat77");

        let outbound = timeout(Duration::from_millis(300), bus.consume_outbound())
            .await
            .expect("timed out waiting for outbound message")
            .expect("expected outbound message");
        assert_eq!(outbound.channel, "telegram");
        assert_eq!(outbound.chat_id, "chat77");
        assert!(outbound.content.contains("permission denied"));
        assert!(outbound.content.contains("shell"));
    }
}
