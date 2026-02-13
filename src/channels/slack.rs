//! Slack channel implementation.
//!
//! This channel currently supports outbound messaging through Slack Web API
//! (`chat.postMessage`). Inbound event handling is intentionally left out until
//! Socket Mode / Events API wiring is added.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

use crate::bus::OutboundMessage;
use crate::config::SlackConfig;
use crate::error::{Result, ZeptoError};

use super::{BaseChannelConfig, Channel};

const SLACK_CHAT_POST_MESSAGE_URL: &str = "https://slack.com/api/chat.postMessage";

/// Slack channel implementation backed by Slack Web API.
pub struct SlackChannel {
    config: SlackConfig,
    base_config: BaseChannelConfig,
    running: Arc<AtomicBool>,
    client: reqwest::Client,
}

impl SlackChannel {
    /// Creates a new Slack channel.
    pub fn new(config: SlackConfig) -> Self {
        let base_config = BaseChannelConfig {
            name: "slack".to_string(),
            allowlist: config.allow_from.clone(),
        };

        Self {
            config,
            base_config,
            running: Arc::new(AtomicBool::new(false)),
            client: reqwest::Client::new(),
        }
    }

    /// Returns a reference to the Slack configuration.
    pub fn slack_config(&self) -> &SlackConfig {
        &self.config
    }

    /// Returns whether the channel is enabled in configuration.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    fn build_payload(msg: &OutboundMessage) -> Result<Value> {
        let channel = msg.chat_id.trim();
        if channel.is_empty() {
            return Err(ZeptoError::Channel(
                "Slack channel ID cannot be empty".to_string(),
            ));
        }

        let mut payload = json!({
            "channel": channel,
            "text": msg.content,
        });

        if let Some(ref reply_to) = msg.reply_to {
            if let Some(map) = payload.as_object_mut() {
                map.insert("thread_ts".to_string(), Value::String(reply_to.clone()));
            }
        }

        Ok(payload)
    }
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn start(&mut self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            info!("Slack channel already running");
            return Ok(());
        }

        if !self.config.enabled {
            warn!("Slack channel is disabled in configuration");
            self.running.store(false, Ordering::SeqCst);
            return Ok(());
        }

        if self.config.bot_token.trim().is_empty() {
            self.running.store(false, Ordering::SeqCst);
            return Err(ZeptoError::Config("Slack bot token is empty".to_string()));
        }

        info!("Starting Slack channel (outbound only)");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if !self.running.swap(false, Ordering::SeqCst) {
            info!("Slack channel already stopped");
            return Ok(());
        }

        info!("Slack channel stopped");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(ZeptoError::Channel("Slack channel not running".to_string()));
        }

        if self.config.bot_token.trim().is_empty() {
            return Err(ZeptoError::Config("Slack bot token is empty".to_string()));
        }

        let payload = Self::build_payload(&msg)?;

        let response = self
            .client
            .post(SLACK_CHAT_POST_MESSAGE_URL)
            .bearer_auth(&self.config.bot_token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ZeptoError::Channel(format!("Failed to call Slack API: {}", e)))?;

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            ZeptoError::Channel(format!("Failed to read Slack API response: {}", e))
        })?;

        if !status.is_success() {
            return Err(ZeptoError::Channel(format!(
                "Slack API returned HTTP {}: {}",
                status, body
            )));
        }

        let body_json: Value = serde_json::from_str(&body)
            .map_err(|e| ZeptoError::Channel(format!("Invalid Slack API response JSON: {}", e)))?;

        if !body_json
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let api_error = body_json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown_error");
            return Err(ZeptoError::Channel(format!(
                "Slack API returned error: {}",
                api_error
            )));
        }

        info!("Slack: Message sent successfully");
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn is_allowed(&self, user_id: &str) -> bool {
        self.base_config.is_allowed(user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_channel_creation() {
        let config = SlackConfig {
            enabled: true,
            bot_token: "xoxb-test-token".to_string(),
            app_token: "xapp-test-token".to_string(),
            allow_from: vec!["U123".to_string()],
        };
        let channel = SlackChannel::new(config);

        assert_eq!(channel.name(), "slack");
        assert!(!channel.is_running());
        assert!(channel.is_allowed("U123"));
        assert!(!channel.is_allowed("U999"));
    }

    #[test]
    fn test_slack_empty_allowlist() {
        let config = SlackConfig {
            enabled: true,
            bot_token: "xoxb-test-token".to_string(),
            app_token: String::new(),
            allow_from: vec![],
        };
        let channel = SlackChannel::new(config);

        assert!(channel.is_allowed("anyone"));
    }

    #[test]
    fn test_slack_config_access() {
        let config = SlackConfig {
            enabled: true,
            bot_token: "xoxb-my-token".to_string(),
            app_token: "xapp-token".to_string(),
            allow_from: vec!["UADMIN".to_string()],
        };
        let channel = SlackChannel::new(config);

        assert!(channel.is_enabled());
        assert_eq!(channel.slack_config().bot_token, "xoxb-my-token");
        assert_eq!(channel.slack_config().allow_from, vec!["UADMIN"]);
    }

    #[tokio::test]
    async fn test_slack_start_without_token() {
        let config = SlackConfig {
            enabled: true,
            bot_token: String::new(),
            app_token: String::new(),
            allow_from: vec![],
        };
        let mut channel = SlackChannel::new(config);

        let result = channel.start().await;
        assert!(result.is_err());
        assert!(!channel.is_running());
    }

    #[tokio::test]
    async fn test_slack_start_disabled() {
        let config = SlackConfig {
            enabled: false,
            bot_token: "xoxb-test-token".to_string(),
            app_token: String::new(),
            allow_from: vec![],
        };
        let mut channel = SlackChannel::new(config);

        let result = channel.start().await;
        assert!(result.is_ok());
        assert!(!channel.is_running());
    }

    #[tokio::test]
    async fn test_slack_stop_not_running() {
        let config = SlackConfig {
            enabled: true,
            bot_token: "xoxb-test-token".to_string(),
            app_token: String::new(),
            allow_from: vec![],
        };
        let mut channel = SlackChannel::new(config);

        let result = channel.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_slack_send_not_running() {
        let config = SlackConfig {
            enabled: true,
            bot_token: "xoxb-test-token".to_string(),
            app_token: String::new(),
            allow_from: vec![],
        };
        let channel = SlackChannel::new(config);

        let msg = OutboundMessage::new("slack", "C123456", "Hello");
        let result = channel.send(msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_slack_send_empty_chat_id() {
        let config = SlackConfig {
            enabled: true,
            bot_token: "xoxb-test-token".to_string(),
            app_token: String::new(),
            allow_from: vec![],
        };
        let channel = SlackChannel::new(config);
        channel.running.store(true, Ordering::SeqCst);

        let msg = OutboundMessage::new("slack", "", "Hello");
        let result = channel.send(msg).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_slack_payload_with_reply() {
        let msg = OutboundMessage::new("slack", "C123", "hello").with_reply("173401.000200");
        let payload = SlackChannel::build_payload(&msg).expect("payload should build");

        assert_eq!(payload["channel"], "C123");
        assert_eq!(payload["text"], "hello");
        assert_eq!(payload["thread_ts"], "173401.000200");
    }
}
