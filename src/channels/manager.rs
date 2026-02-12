//! Channel Manager for PicoClaw
//!
//! This module provides the `ChannelManager` which is responsible for:
//! - Registering and managing multiple communication channels
//! - Starting and stopping all channels
//! - Dispatching outbound messages to the appropriate channels

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::bus::{MessageBus, OutboundMessage};
use crate::config::Config;
use crate::error::Result;

use super::Channel;

/// The `ChannelManager` manages the lifecycle of all communication channels.
///
/// It provides methods to:
/// - Register new channels
/// - Start and stop all channels
/// - Route outbound messages to the correct channel
/// - List all registered channels
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────┐
/// │                     ChannelManager                          │
/// │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐       │
/// │  │Telegram │  │ Discord │  │  Slack  │  │WhatsApp │  ...  │
/// │  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘       │
/// │       │            │            │            │              │
/// │       └────────────┴─────┬──────┴────────────┘              │
/// │                          │                                  │
/// │                    ┌─────┴─────┐                           │
/// │                    │MessageBus │                           │
/// │                    └───────────┘                           │
/// └─────────────────────────────────────────────────────────────┘
/// ```
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use picoclaw::bus::MessageBus;
/// use picoclaw::config::Config;
/// use picoclaw::channels::ChannelManager;
///
/// #[tokio::main]
/// async fn main() {
///     let bus = Arc::new(MessageBus::new());
///     let config = Config::default();
///     let manager = ChannelManager::new(bus, config);
///
///     // Register channels
///     // manager.register(Box::new(TelegramChannel::new(...))).await;
///
///     // Start all channels
///     manager.start_all().await.unwrap();
/// }
/// ```
pub struct ChannelManager {
    /// Map of channel name to channel instance
    channels: Arc<RwLock<HashMap<String, Box<dyn Channel>>>>,
    /// Reference to the message bus for routing
    bus: Arc<MessageBus>,
    /// Global configuration
    #[allow(dead_code)]
    config: Config,
}

impl ChannelManager {
    /// Creates a new `ChannelManager` with the given message bus and configuration.
    ///
    /// # Arguments
    ///
    /// * `bus` - The message bus for routing messages
    /// * `config` - The global configuration
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    /// use picoclaw::bus::MessageBus;
    /// use picoclaw::config::Config;
    /// use picoclaw::channels::ChannelManager;
    ///
    /// let bus = Arc::new(MessageBus::new());
    /// let config = Config::default();
    /// let manager = ChannelManager::new(bus, config);
    /// ```
    pub fn new(bus: Arc<MessageBus>, config: Config) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            bus,
            config,
        }
    }

    /// Registers a new channel with the manager.
    ///
    /// The channel is stored by its name and can be started later with `start_all()`.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel to register
    ///
    /// # Example
    ///
    /// ```ignore
    /// let manager = ChannelManager::new(bus, config);
    /// manager.register(Box::new(telegram_channel)).await;
    /// ```
    pub async fn register(&self, channel: Box<dyn Channel>) {
        let name = channel.name().to_string();
        info!("Registering channel: {}", name);
        let mut channels = self.channels.write().await;
        channels.insert(name, channel);
    }

    /// Returns a list of all registered channel names.
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    /// use picoclaw::bus::MessageBus;
    /// use picoclaw::config::Config;
    /// use picoclaw::channels::ChannelManager;
    ///
    /// # tokio_test::block_on(async {
    /// let bus = Arc::new(MessageBus::new());
    /// let config = Config::default();
    /// let manager = ChannelManager::new(bus, config);
    ///
    /// let channels = manager.channels().await;
    /// assert!(channels.is_empty());
    /// # })
    /// ```
    pub async fn channels(&self) -> Vec<String> {
        let channels = self.channels.read().await;
        channels.keys().cloned().collect()
    }

    /// Returns the number of registered channels.
    pub async fn channel_count(&self) -> usize {
        let channels = self.channels.read().await;
        channels.len()
    }

    /// Checks if a channel with the given name is registered.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the channel to check
    pub async fn has_channel(&self, name: &str) -> bool {
        let channels = self.channels.read().await;
        channels.contains_key(name)
    }

    /// Starts all registered channels.
    ///
    /// This method:
    /// 1. Iterates over all registered channels and starts each one
    /// 2. Spawns a background task to dispatch outbound messages
    ///
    /// Errors from individual channels are logged but do not prevent
    /// other channels from starting.
    ///
    /// # Errors
    ///
    /// Returns `Ok(())` even if individual channels fail to start.
    /// Check logs for channel-specific errors.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let manager = ChannelManager::new(bus, config);
    /// manager.register(Box::new(telegram_channel)).await;
    /// manager.start_all().await?;
    /// ```
    pub async fn start_all(&self) -> Result<()> {
        let mut channels = self.channels.write().await;
        for (name, channel) in channels.iter_mut() {
            info!("Starting channel: {}", name);
            if let Err(e) = channel.start().await {
                error!("Failed to start channel {}: {}", name, e);
            }
        }
        drop(channels); // Release the write lock before spawning

        // Start outbound dispatcher
        let bus = self.bus.clone();
        let channels_ref = self.channels.clone();
        tokio::spawn(async move {
            dispatch_outbound(bus, channels_ref).await;
        });

        Ok(())
    }

    /// Stops all registered channels.
    ///
    /// Errors from individual channels are logged but do not prevent
    /// other channels from stopping.
    ///
    /// # Errors
    ///
    /// Returns `Ok(())` even if individual channels fail to stop.
    /// Check logs for channel-specific errors.
    ///
    /// # Example
    ///
    /// ```ignore
    /// manager.stop_all().await?;
    /// ```
    pub async fn stop_all(&self) -> Result<()> {
        let mut channels = self.channels.write().await;
        for (name, channel) in channels.iter_mut() {
            info!("Stopping channel: {}", name);
            if let Err(e) = channel.stop().await {
                error!("Failed to stop channel {}: {}", name, e);
            }
        }
        Ok(())
    }

    /// Sends a message to a specific channel.
    ///
    /// # Arguments
    ///
    /// * `channel_name` - The name of the channel to send to
    /// * `msg` - The outbound message to send
    ///
    /// # Errors
    ///
    /// Returns an error if the channel fails to send the message.
    /// If the channel is not found, a warning is logged and `Ok(())` is returned.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let msg = OutboundMessage::new("telegram", "chat123", "Hello!");
    /// manager.send("telegram", msg).await?;
    /// ```
    pub async fn send(&self, channel_name: &str, msg: OutboundMessage) -> Result<()> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.send(msg).await
        } else {
            warn!("Channel not found: {}", channel_name);
            Ok(())
        }
    }

    /// Returns a reference to the message bus.
    pub fn bus(&self) -> Arc<MessageBus> {
        self.bus.clone()
    }
}

/// Background task that dispatches outbound messages from the bus to channels.
///
/// This function runs in a loop, consuming outbound messages from the bus
/// and routing them to the appropriate channel based on the message's
/// `channel` field.
///
/// # Arguments
///
/// * `bus` - The message bus to consume from
/// * `channels` - The shared map of channels
async fn dispatch_outbound(
    bus: Arc<MessageBus>,
    channels: Arc<RwLock<HashMap<String, Box<dyn Channel>>>>,
) {
    loop {
        if let Some(msg) = bus.consume_outbound().await {
            let channel_name = msg.channel.clone();
            let channels = channels.read().await;
            if let Some(channel) = channels.get(&channel_name) {
                if let Err(e) = channel.send(msg.clone()).await {
                    error!("Failed to send message to {}: {}", channel_name, e);
                }
            } else {
                warn!("Unknown channel for outbound message: {}", channel_name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A mock channel for testing
    struct MockChannel {
        name: String,
        running: Arc<AtomicBool>,
        allowlist: Vec<String>,
    }

    impl MockChannel {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                running: Arc::new(AtomicBool::new(false)),
                allowlist: Vec::new(),
            }
        }

        fn with_allowlist(name: &str, allowlist: Vec<String>) -> Self {
            Self {
                name: name.to_string(),
                running: Arc::new(AtomicBool::new(false)),
                allowlist,
            }
        }
    }

    #[async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn start(&mut self) -> Result<()> {
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&mut self) -> Result<()> {
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }

        async fn send(&self, _msg: OutboundMessage) -> Result<()> {
            Ok(())
        }

        fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }

        fn is_allowed(&self, user_id: &str) -> bool {
            self.allowlist.is_empty() || self.allowlist.contains(&user_id.to_string())
        }
    }

    #[tokio::test]
    async fn test_channel_manager_creation() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);
        assert!(manager.channels().await.is_empty());
    }

    #[tokio::test]
    async fn test_register_channel() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);

        let channel = MockChannel::new("test");
        manager.register(Box::new(channel)).await;

        let channels = manager.channels().await;
        assert_eq!(channels.len(), 1);
        assert!(channels.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn test_register_multiple_channels() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);

        manager.register(Box::new(MockChannel::new("telegram"))).await;
        manager.register(Box::new(MockChannel::new("discord"))).await;
        manager.register(Box::new(MockChannel::new("slack"))).await;

        assert_eq!(manager.channel_count().await, 3);
        assert!(manager.has_channel("telegram").await);
        assert!(manager.has_channel("discord").await);
        assert!(manager.has_channel("slack").await);
        assert!(!manager.has_channel("whatsapp").await);
    }

    #[tokio::test]
    async fn test_start_all() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);

        let channel = MockChannel::new("test");
        manager.register(Box::new(channel)).await;

        manager.start_all().await.unwrap();

        // Give the dispatcher task time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_stop_all() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);

        manager.register(Box::new(MockChannel::new("test"))).await;
        manager.start_all().await.unwrap();
        manager.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn test_send_to_unknown_channel() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);

        let msg = OutboundMessage::new("unknown", "chat123", "Hello");
        let result = manager.send("unknown", msg).await;
        assert!(result.is_ok()); // Should not error, just warn
    }

    #[tokio::test]
    async fn test_send_to_registered_channel() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus, config);

        manager.register(Box::new(MockChannel::new("test"))).await;

        let msg = OutboundMessage::new("test", "chat123", "Hello");
        let result = manager.send("test", msg).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_channel_allowlist() {
        let channel = MockChannel::with_allowlist("test", vec!["user1".to_string()]);
        assert!(channel.is_allowed("user1"));
        assert!(!channel.is_allowed("user2"));
    }

    #[tokio::test]
    async fn test_channel_empty_allowlist() {
        let channel = MockChannel::new("test");
        assert!(channel.is_allowed("anyone"));
    }

    #[tokio::test]
    async fn test_bus_reference() {
        let bus = Arc::new(MessageBus::new());
        let config = Config::default();
        let manager = ChannelManager::new(bus.clone(), config);

        // The bus reference should be the same
        assert!(Arc::ptr_eq(&bus, &manager.bus()));
    }
}
