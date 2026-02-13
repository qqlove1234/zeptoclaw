# Heartbeat Service Implementation Plan

> **For Claude:** Use this plan to implement the heartbeat service task-by-task.

**Goal:** Add a periodic heartbeat service to ZeptoClaw that wakes the agent at configurable intervals to check for and execute background tasks from a `HEARTBEAT.md` file.

**Architecture:** Implement a background Tokio task that periodically reads a heartbeat file and triggers the agent to process any tasks. Integrates with existing agent loop and session management.

**Tech Stack:** Rust, Tokio (async runtime), existing agent infrastructure

---

## Use Cases

| Use Case | Example |
|----------|---------|
| Periodic order checks | "Check Google Sheet for new orders, send WhatsApp confirmations" |
| Low stock alerts | "Alert if any inventory item below minimum stock" |
| Daily summaries | "Generate daily sales report at 6pm" |
| Follow-up reminders | "Send shipping reminder for pending orders > 24h" |
| Health monitoring | "Check if all integrations are working" |

---

## Task 1: Add Heartbeat Config

**Files:**
- Modify: `src/config/types.rs`

**Add heartbeat configuration struct:**

```rust
/// Heartbeat service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Enable/disable heartbeat service
    #[serde(default = "default_heartbeat_enabled")]
    pub enabled: bool,

    /// Interval between heartbeats in seconds (default: 1800 = 30 minutes)
    #[serde(default = "default_heartbeat_interval")]
    pub interval_secs: u64,

    /// Path to heartbeat file (default: ~/.zeptoclaw/HEARTBEAT.md)
    #[serde(default)]
    pub file_path: Option<String>,
}

fn default_heartbeat_enabled() -> bool {
    false // Opt-in by default
}

fn default_heartbeat_interval() -> u64 {
    1800 // 30 minutes
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: default_heartbeat_enabled(),
            interval_secs: default_heartbeat_interval(),
            file_path: None,
        }
    }
}
```

**Add to main Config struct:**

```rust
/// Heartbeat service for periodic task checking
#[serde(default)]
pub heartbeat: HeartbeatConfig,
```

---

## Task 2: Create Heartbeat Service Module

**Files:**
- Create: `src/heartbeat/mod.rs`
- Create: `src/heartbeat/service.rs`

**src/heartbeat/mod.rs:**

```rust
//! Heartbeat service - periodic agent wake-up for background tasks.
//!
//! The heartbeat service reads a HEARTBEAT.md file at regular intervals
//! and triggers the agent to execute any tasks listed there.

mod service;

pub use service::{HeartbeatService, HEARTBEAT_PROMPT};
```

**src/heartbeat/service.rs:**

```rust
//! Heartbeat service implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::bus::{InboundMessage, MessageBus};
use crate::error::Result;

/// Default heartbeat interval: 30 minutes
pub const DEFAULT_INTERVAL_SECS: u64 = 30 * 60;

/// The prompt sent to agent during heartbeat
pub const HEARTBEAT_PROMPT: &str = r#"Read HEARTBEAT.md in your workspace (if it exists).
Follow any instructions or tasks listed there.
If nothing needs attention, reply with just: HEARTBEAT_OK"#;

/// Token that indicates "nothing to do"
const HEARTBEAT_OK_TOKEN: &str = "HEARTBEAT_OK";

/// Heartbeat service that periodically wakes the agent.
pub struct HeartbeatService {
    file_path: PathBuf,
    interval: Duration,
    bus: Arc<MessageBus>,
    running: Arc<RwLock<bool>>,
    session_key: String,
}

impl HeartbeatService {
    /// Create a new heartbeat service.
    pub fn new(
        file_path: PathBuf,
        interval_secs: u64,
        bus: Arc<MessageBus>,
        session_key: &str,
    ) -> Self {
        Self {
            file_path,
            interval: Duration::from_secs(interval_secs),
            bus,
            running: Arc::new(RwLock::new(false)),
            session_key: session_key.to_string(),
        }
    }

    /// Create with default settings.
    pub fn with_defaults(bus: Arc<MessageBus>) -> Self {
        let file_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zeptoclaw")
            .join("HEARTBEAT.md");

        Self::new(file_path, DEFAULT_INTERVAL_SECS, bus, "heartbeat:system")
    }

    /// Start the heartbeat service.
    pub async fn start(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            if *running {
                warn!("Heartbeat service already running");
                return Ok(());
            }
            *running = true;
        }

        info!(
            "Heartbeat service started (interval: {}s, file: {:?})",
            self.interval.as_secs(),
            self.file_path
        );

        let file_path = self.file_path.clone();
        let interval = self.interval;
        let bus = self.bus.clone();
        let running = self.running.clone();
        let session_key = self.session_key.clone();

        tokio::spawn(async move {
            let mut ticker = interval(interval);

            // Skip the first immediate tick
            ticker.tick().await;

            loop {
                ticker.tick().await;

                // Check if still running
                if !*running.read().await {
                    info!("Heartbeat service stopped");
                    break;
                }

                // Execute heartbeat
                if let Err(e) = Self::tick(&file_path, &bus, &session_key).await {
                    error!("Heartbeat error: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Stop the heartbeat service.
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Heartbeat service stopping...");
    }

    /// Execute a single heartbeat tick.
    async fn tick(file_path: &PathBuf, bus: &MessageBus, session_key: &str) -> Result<()> {
        // Read heartbeat file
        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("Heartbeat: no HEARTBEAT.md file");
                return Ok(());
            }
            Err(e) => {
                warn!("Heartbeat: failed to read file: {}", e);
                return Ok(());
            }
        };

        // Check if file has actionable content
        if Self::is_empty(&content) {
            debug!("Heartbeat: no tasks (file empty or only headers)");
            return Ok(());
        }

        info!("Heartbeat: checking for tasks...");

        // Send heartbeat message to agent via bus
        let message = InboundMessage::new("heartbeat", "system", HEARTBEAT_PROMPT);
        bus.publish_inbound(message).await?;

        Ok(())
    }

    /// Check if heartbeat content is empty (no actionable tasks).
    fn is_empty(content: &str) -> bool {
        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Skip headers
            if line.starts_with('#') {
                continue;
            }

            // Skip HTML comments
            if line.starts_with("<!--") {
                continue;
            }

            // Skip empty checkboxes
            if line == "- [ ]" || line == "* [ ]" {
                continue;
            }

            // Found actionable content
            return false;
        }

        true
    }

    /// Manually trigger a heartbeat (for testing or CLI).
    pub async fn trigger_now(&self) -> Result<()> {
        info!("Heartbeat: manual trigger");
        Self::tick(&self.file_path, &self.bus, &self.session_key).await
    }

    /// Check if the service is running.
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_empty_true() {
        assert!(HeartbeatService::is_empty(""));
        assert!(HeartbeatService::is_empty("# Heartbeat Tasks\n\n## Active\n"));
        assert!(HeartbeatService::is_empty("<!-- comment -->\n# Header\n"));
        assert!(HeartbeatService::is_empty("- [ ]\n* [ ]"));
    }

    #[test]
    fn test_is_empty_false() {
        assert!(!HeartbeatService::is_empty("Check for new orders"));
        assert!(!HeartbeatService::is_empty("# Tasks\n- Send reminders"));
        assert!(!HeartbeatService::is_empty("- [x] Completed task"));
    }

    #[test]
    fn test_default_interval() {
        assert_eq!(DEFAULT_INTERVAL_SECS, 30 * 60);
    }
}
```

---

## Task 3: Create Default HEARTBEAT.md Template

**Files:**
- Create: `src/heartbeat/template.rs`

**Implementation:**

```rust
//! Default HEARTBEAT.md template.

/// Default heartbeat file content.
pub const HEARTBEAT_TEMPLATE: &str = r#"# Heartbeat Tasks

This file is checked periodically by your ZeptoClaw agent.
Add tasks below that you want the agent to work on in the background.

If this file has no tasks (only headers and comments), the agent will skip the heartbeat.

## Active Tasks

<!-- Add your periodic tasks below this line -->
<!-- Examples:
- Check Google Sheet for new orders and send WhatsApp confirmations
- Alert me if any inventory item is below minimum stock
- Send shipping reminder for orders older than 24 hours
-->


## Completed

<!-- Move completed tasks here or delete them -->

"#;

/// Create the heartbeat file if it doesn't exist.
pub async fn ensure_heartbeat_file(path: &std::path::Path) -> std::io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }

    // Create parent directory if needed
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(path, HEARTBEAT_TEMPLATE).await?;
    Ok(true)
}
```

**Update mod.rs:**

```rust
mod service;
mod template;

pub use service::{HeartbeatService, HEARTBEAT_PROMPT, DEFAULT_INTERVAL_SECS};
pub use template::{ensure_heartbeat_file, HEARTBEAT_TEMPLATE};
```

---

## Task 4: Export Heartbeat Module

**Files:**
- Modify: `src/lib.rs`

**Add module declaration:**

```rust
pub mod heartbeat;
```

**Add to re-exports:**

```rust
pub use heartbeat::{HeartbeatService, HEARTBEAT_PROMPT};
```

---

## Task 5: Integrate Heartbeat into Gateway

**Files:**
- Modify: `src/main.rs`

**In cmd_gateway(), after starting channels:**

```rust
use zeptoclaw::heartbeat::{HeartbeatService, ensure_heartbeat_file};

// Start heartbeat service if enabled
let heartbeat_service = if config.heartbeat.enabled {
    let file_path = config.heartbeat.file_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".zeptoclaw")
                .join("HEARTBEAT.md")
        });

    // Create template file if it doesn't exist
    if let Ok(created) = ensure_heartbeat_file(&file_path).await {
        if created {
            info!("Created HEARTBEAT.md template at {:?}", file_path);
        }
    }

    let service = HeartbeatService::new(
        file_path,
        config.heartbeat.interval_secs,
        bus.clone(),
        "heartbeat:system",
    );
    service.start().await?;
    Some(service)
} else {
    None
};
```

**In shutdown section:**

```rust
// Stop heartbeat service
if let Some(ref service) = heartbeat_service {
    service.stop().await;
}
```

---

## Task 6: Add Heartbeat CLI Command

**Files:**
- Modify: `src/main.rs`

**Add heartbeat subcommand:**

```rust
#[derive(Subcommand)]
enum Commands {
    // ... existing commands ...

    /// Trigger a heartbeat check manually
    Heartbeat {
        /// Show current heartbeat file content
        #[arg(short, long)]
        show: bool,

        /// Edit heartbeat file
        #[arg(short, long)]
        edit: bool,
    },
}
```

**Implement command:**

```rust
async fn cmd_heartbeat(config: &Config, show: bool, edit: bool) -> Result<()> {
    let file_path = config.heartbeat.file_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".zeptoclaw")
                .join("HEARTBEAT.md")
        });

    // Create template if doesn't exist
    if let Ok(created) = ensure_heartbeat_file(&file_path).await {
        if created {
            println!("Created HEARTBEAT.md at {:?}", file_path);
        }
    }

    if show {
        // Show file content
        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => {
                println!("=== HEARTBEAT.md ===\n{}", content);
            }
            Err(e) => {
                eprintln!("Failed to read heartbeat file: {}", e);
            }
        }
        return Ok(());
    }

    if edit {
        // Open in default editor
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
        let status = std::process::Command::new(&editor)
            .arg(&file_path)
            .status()?;

        if !status.success() {
            eprintln!("Editor exited with error");
        }
        return Ok(());
    }

    // Trigger heartbeat manually
    println!("Triggering heartbeat check...");

    let bus = Arc::new(MessageBus::new());
    let service = HeartbeatService::new(
        file_path,
        config.heartbeat.interval_secs,
        bus,
        "heartbeat:cli",
    );

    service.trigger_now().await?;
    println!("Heartbeat triggered. Check agent output for results.");

    Ok(())
}
```

---

## Task 7: Update Onboard Wizard

**Files:**
- Modify: `src/main.rs`

**Add heartbeat configuration to onboard:**

```rust
fn configure_heartbeat(config: &mut Config) -> Result<()> {
    println!("\nHeartbeat Service Setup");
    println!("-----------------------");
    println!("Heartbeat periodically wakes the agent to check for background tasks.");
    println!("Tasks are defined in ~/.zeptoclaw/HEARTBEAT.md");

    print!("Enable heartbeat service? (y/N): ");
    std::io::stdout().flush()?;
    let enable = read_line()?.to_lowercase();

    if enable == "y" || enable == "yes" {
        config.heartbeat.enabled = true;

        print!("Interval in minutes (default 30): ");
        std::io::stdout().flush()?;
        let interval = read_line()?;

        if !interval.is_empty() {
            if let Ok(mins) = interval.parse::<u64>() {
                config.heartbeat.interval_secs = mins * 60;
            }
        }

        println!("✓ Heartbeat enabled (every {} minutes)", config.heartbeat.interval_secs / 60);
    } else {
        config.heartbeat.enabled = false;
        println!("Heartbeat disabled. Enable later in config or with --heartbeat flag.");
    }

    Ok(())
}
```

**Call in cmd_onboard:**

```rust
configure_heartbeat(&mut config)?;
```

---

## Task 8: Add Integration Tests

**Files:**
- Modify: `tests/integration.rs`

**Add tests:**

```rust
#[test]
fn test_config_heartbeat() {
    let json = r#"{
        "heartbeat": {
            "enabled": true,
            "interval_secs": 900
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.heartbeat.enabled);
    assert_eq!(config.heartbeat.interval_secs, 900);
}

#[test]
fn test_config_heartbeat_defaults() {
    let json = r#"{}"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert!(!config.heartbeat.enabled); // Default: disabled
    assert_eq!(config.heartbeat.interval_secs, 1800); // Default: 30 mins
}

#[test]
fn test_heartbeat_is_empty() {
    use zeptoclaw::heartbeat::HeartbeatService;

    // These should be considered empty (no action needed)
    assert!(HeartbeatService::is_empty(""));
    assert!(HeartbeatService::is_empty("# Header\n## Another"));
    assert!(HeartbeatService::is_empty("<!-- comment -->"));

    // These should trigger action
    assert!(!HeartbeatService::is_empty("Check orders"));
    assert!(!HeartbeatService::is_empty("# Tasks\n- Do something"));
}
```

---

## Task 9: Add Documentation

**Files:**
- Create: `docs/HEARTBEAT.md`

**Content:**

```markdown
# Heartbeat Service

The heartbeat service periodically wakes the ZeptoClaw agent to check for and execute background tasks.

## Configuration

In `~/.zeptoclaw/config.json`:

```json
{
  "heartbeat": {
    "enabled": true,
    "interval_secs": 1800,
    "file_path": "~/.zeptoclaw/HEARTBEAT.md"
  }
}
```

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `false` | Enable/disable heartbeat |
| `interval_secs` | `1800` | Interval between checks (30 min) |
| `file_path` | `~/.zeptoclaw/HEARTBEAT.md` | Path to task file |

## Environment Variables

```bash
export ZEPTOCLAW_HEARTBEAT_ENABLED=true
export ZEPTOCLAW_HEARTBEAT_INTERVAL_SECS=900  # 15 minutes
```

## Usage

### Define Tasks

Edit `~/.zeptoclaw/HEARTBEAT.md`:

```markdown
# Heartbeat Tasks

## Active Tasks

- Check Google Sheet for new orders and send WhatsApp confirmations
- Alert me if any inventory item is below minimum stock
- Send daily sales summary at 6pm

## Completed

- [x] Set up inventory tracking
```

### CLI Commands

```bash
# Show heartbeat file
zeptoclaw heartbeat --show

# Edit heartbeat file
zeptoclaw heartbeat --edit

# Trigger heartbeat manually
zeptoclaw heartbeat
```

### How It Works

1. Every `interval_secs`, the service reads `HEARTBEAT.md`
2. If the file has actionable content (not just headers/comments), it triggers the agent
3. Agent reads the file and executes tasks
4. Agent responds `HEARTBEAT_OK` if nothing needed attention

### Example Tasks for E-Commerce

```markdown
## Active Tasks

- Check Orders sheet for status="New", send WhatsApp confirmation for each
- Check Inventory sheet, alert if any SKU has stock < min_stock
- Every Monday: Generate weekly sales report from Orders sheet
- Check for orders with status="Shipped" older than 5 days, send delivery follow-up
```

## Tips

- Keep tasks specific and actionable
- Use comments `<!-- -->` for notes that shouldn't trigger action
- Move completed tasks to `## Completed` section
- Start with longer intervals (60 min) and reduce as needed
```

---

## Verification

After implementation, run:

```bash
# Format check
cargo fmt -- --check

# Lint check
cargo clippy -- -D warnings

# Unit tests
cargo test --lib

# Integration tests
cargo test --test integration

# Manual test - create heartbeat file
mkdir -p ~/.zeptoclaw
cat > ~/.zeptoclaw/HEARTBEAT.md << 'EOF'
# Heartbeat Tasks

## Active Tasks

- Say hello and confirm heartbeat is working
EOF

# Test CLI commands
cargo run -- heartbeat --show
cargo run -- heartbeat

# Test with gateway (enable in config first)
cargo run -- gateway
```

---

## Summary

| Component | Purpose |
|-----------|---------|
| `HeartbeatConfig` | Configuration for interval, file path, enabled flag |
| `HeartbeatService` | Background task that runs on interval |
| `HEARTBEAT.md` | User-editable file with tasks for agent |
| `heartbeat` CLI | Manual trigger, show, edit commands |

**Config Addition:**
```json
{
  "heartbeat": {
    "enabled": true,
    "interval_secs": 1800
  }
}
```

**Default File Location:** `~/.zeptoclaw/HEARTBEAT.md`

---

*Last updated: 2026-02-13*
