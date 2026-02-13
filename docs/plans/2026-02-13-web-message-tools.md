# Web Search & Message Tools Implementation Plan

> **For Claude:** Use this plan to implement web search and message tools task-by-task.

**Goal:** Add `web_search`, `web_fetch`, and `message` tools to ZeptoClaw for web access and proactive messaging capabilities.

**Architecture:** Implement three new tools following the existing `Tool` trait pattern. Web tools use reqwest (already a dependency). Message tool uses the existing MessageBus.

**Tech Stack:** Rust, reqwest, serde, async-trait, scraper (new dependency for HTML parsing)

---

## Task 1: Add Dependencies

**Files:**
- Modify: `Cargo.toml`

**Changes:**

Add to `[dependencies]`:

```toml
# =============================================================================
# WEB TOOLS
# =============================================================================
# HTML parsing for web_fetch content extraction
scraper = "0.18"
```

**Verification:**
```bash
cargo check
```

---

## Task 2: Add Web Search Config

**Files:**
- Modify: `src/config/types.rs`

**Step 1: Add BraveConfig struct**

After the existing provider configs, add:

```rust
/// Brave Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BraveConfig {
    /// API key for Brave Search
    #[serde(default)]
    pub api_key: Option<String>,
}
```

**Step 2: Add to Config struct**

In the main Config struct, add a new section:

```rust
/// External service integrations
#[serde(default)]
pub integrations: IntegrationsConfig,
```

And define:

```rust
/// External service integrations configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationsConfig {
    /// Brave Search API for web_search tool
    #[serde(default)]
    pub brave: BraveConfig,
}
```

**Step 3: Add environment variable support**

The config should pick up `ZEPTOCLAW_INTEGRATIONS_BRAVE_API_KEY` or `BRAVE_API_KEY`.

---

## Task 3: Create Web Search Tool

**Files:**
- Create: `src/tools/web.rs`

**Implementation:**

```rust
//! Web tools: web_search and web_fetch
//!
//! Provides web search via Brave Search API and URL content fetching.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{PicoError, Result};
use crate::tools::{Tool, ToolContext};

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36";
const BRAVE_API_URL: &str = "https://api.search.brave.com/res/v1/web/search";

/// Web search tool using Brave Search API.
pub struct WebSearchTool {
    api_key: String,
    client: Client,
    max_results: usize,
}

impl WebSearchTool {
    /// Create a new web search tool.
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            client: Client::new(),
            max_results: 5,
        }
    }

    /// Create with custom max results.
    pub fn with_max_results(api_key: &str, max_results: usize) -> Self {
        Self {
            api_key: api_key.to_string(),
            client: Client::new(),
            max_results,
        }
    }
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: Option<String>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns titles, URLs, and snippets."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results (1-10)",
                    "minimum": 1,
                    "maximum": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'query' parameter".into()))?;

        let count = args["count"]
            .as_u64()
            .map(|c| c.min(10).max(1) as usize)
            .unwrap_or(self.max_results);

        if self.api_key.is_empty() {
            return Err(PicoError::Tool(
                "BRAVE_API_KEY not configured. Set ZEPTOCLAW_INTEGRATIONS_BRAVE_API_KEY".into(),
            ));
        }

        let response = self
            .client
            .get(BRAVE_API_URL)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .query(&[("q", query), ("count", &count.to_string())])
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("Search request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(PicoError::Tool(format!(
                "Brave API error: {}",
                response.status()
            )));
        }

        let brave_response: BraveResponse = response
            .json()
            .await
            .map_err(|e| PicoError::Tool(format!("Failed to parse response: {}", e)))?;

        let results = brave_response
            .web
            .map(|w| w.results)
            .unwrap_or_default();

        if results.is_empty() {
            return Ok(format!("No results for: {}", query));
        }

        let mut output = format!("Results for: {}\n\n", query);
        for (i, result) in results.iter().take(count).enumerate() {
            output.push_str(&format!("{}. {}\n", i + 1, result.title));
            output.push_str(&format!("   {}\n", result.url));
            if let Some(desc) = &result.description {
                output.push_str(&format!("   {}\n", desc));
            }
            output.push('\n');
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_search_tool_properties() {
        let tool = WebSearchTool::new("test-key");
        assert_eq!(tool.name(), "web_search");
        assert!(tool.description().contains("Search"));
    }

    #[test]
    fn test_web_search_parameters() {
        let tool = WebSearchTool::new("test-key");
        let params = tool.parameters();
        assert!(params["properties"]["query"].is_object());
        assert!(params["properties"]["count"].is_object());
    }
}
```

---

## Task 4: Create Web Fetch Tool

**Files:**
- Modify: `src/tools/web.rs`

**Add to existing web.rs:**

```rust
use scraper::{Html, Selector};
use std::time::Duration;

const MAX_REDIRECTS: usize = 5;
const DEFAULT_MAX_CHARS: usize = 50000;

/// Web fetch tool for retrieving URL content.
pub struct WebFetchTool {
    client: Client,
    max_chars: usize,
}

impl WebFetchTool {
    /// Create a new web fetch tool.
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            max_chars: DEFAULT_MAX_CHARS,
        }
    }

    /// Create with custom max characters.
    pub fn with_max_chars(max_chars: usize) -> Self {
        let mut tool = Self::new();
        tool.max_chars = max_chars;
        tool
    }

    /// Extract readable text from HTML.
    fn extract_text(&self, html: &str) -> String {
        let document = Html::parse_document(html);

        // Remove script and style elements
        let body_selector = Selector::parse("body").unwrap();
        let text_selector = Selector::parse("p, h1, h2, h3, h4, h5, h6, li, td, th, span, div").unwrap();

        let mut text = String::new();

        if let Some(body) = document.select(&body_selector).next() {
            for element in body.select(&text_selector) {
                let element_text: String = element.text().collect::<Vec<_>>().join(" ");
                let trimmed = element_text.trim();
                if !trimmed.is_empty() {
                    text.push_str(trimmed);
                    text.push('\n');
                }
            }
        }

        // Normalize whitespace
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Extract title from HTML.
    fn extract_title(&self, html: &str) -> Option<String> {
        let document = Html::parse_document(html);
        let title_selector = Selector::parse("title").ok()?;
        document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch URL and extract readable content (HTML → text)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return",
                    "minimum": 100
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'url' parameter".into()))?;

        let max_chars = args["max_chars"]
            .as_u64()
            .map(|c| c as usize)
            .unwrap_or(self.max_chars);

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(PicoError::Tool(
                "Only http/https URLs are allowed".into(),
            ));
        }

        let response = self
            .client
            .get(url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("Fetch failed: {}", e)))?;

        let status = response.status();
        let final_url = response.url().to_string();

        if !status.is_success() {
            return Err(PicoError::Tool(format!("HTTP error: {}", status)));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let body = response
            .text()
            .await
            .map_err(|e| PicoError::Tool(format!("Failed to read body: {}", e)))?;

        let (text, extractor) = if content_type.contains("application/json") {
            (body, "json")
        } else if content_type.contains("text/html") || body.trim_start().starts_with('<') {
            let title = self.extract_title(&body).unwrap_or_default();
            let extracted = self.extract_text(&body);
            let text = if title.is_empty() {
                extracted
            } else {
                format!("# {}\n\n{}", title, extracted)
            };
            (text, "html")
        } else {
            (body, "raw")
        };

        let truncated = text.len() > max_chars;
        let text = if truncated {
            text[..max_chars].to_string()
        } else {
            text
        };

        Ok(json!({
            "url": url,
            "final_url": final_url,
            "status": status.as_u16(),
            "extractor": extractor,
            "truncated": truncated,
            "length": text.len(),
            "text": text
        })
        .to_string())
    }
}

#[cfg(test)]
mod web_fetch_tests {
    use super::*;

    #[test]
    fn test_web_fetch_tool_properties() {
        let tool = WebFetchTool::new();
        assert_eq!(tool.name(), "web_fetch");
        assert!(tool.description().contains("Fetch"));
    }

    #[test]
    fn test_extract_text() {
        let tool = WebFetchTool::new();
        let html = r#"
            <html>
            <body>
                <h1>Title</h1>
                <p>This is a paragraph.</p>
                <script>alert('hi')</script>
            </body>
            </html>
        "#;
        let text = tool.extract_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("paragraph"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_extract_title() {
        let tool = WebFetchTool::new();
        let html = "<html><head><title>My Page</title></head></html>";
        assert_eq!(tool.extract_title(html), Some("My Page".to_string()));
    }
}
```

---

## Task 5: Create Message Tool

**Files:**
- Create: `src/tools/message.rs`

**Implementation:**

```rust
//! Message tool for sending messages to users.
//!
//! Allows the agent to proactively send messages to chat channels.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::bus::{MessageBus, OutboundMessage};
use crate::error::{PicoError, Result};
use crate::tools::{Tool, ToolContext};

/// Tool for sending messages to chat channels.
pub struct MessageTool {
    bus: Arc<MessageBus>,
}

impl MessageTool {
    /// Create a new message tool with the given message bus.
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self { bus }
    }
}

#[async_trait]
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to a user or chat. Use this to communicate proactively."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The message content to send"
                },
                "channel": {
                    "type": "string",
                    "description": "Target channel (telegram, discord, etc.). Optional if context has default."
                },
                "chat_id": {
                    "type": "string",
                    "description": "Target chat/user ID. Optional if context has default."
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String> {
        let content = args["content"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'content' parameter".into()))?;

        // Get channel and chat_id from args or context
        let channel = args["channel"]
            .as_str()
            .map(String::from)
            .or_else(|| ctx.channel.clone())
            .ok_or_else(|| PicoError::Tool("No target channel specified".into()))?;

        let chat_id = args["chat_id"]
            .as_str()
            .map(String::from)
            .or_else(|| ctx.chat_id.clone())
            .ok_or_else(|| PicoError::Tool("No target chat_id specified".into()))?;

        let outbound = OutboundMessage::new(&channel, &chat_id, content);

        self.bus
            .publish_outbound(outbound)
            .await
            .map_err(|e| PicoError::Tool(format!("Failed to send message: {}", e)))?;

        Ok(format!("Message sent to {}:{}", channel, chat_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_tool_properties() {
        let bus = Arc::new(MessageBus::new());
        let tool = MessageTool::new(bus);
        assert_eq!(tool.name(), "message");
        assert!(tool.description().contains("Send"));
    }

    #[test]
    fn test_message_parameters() {
        let bus = Arc::new(MessageBus::new());
        let tool = MessageTool::new(bus);
        let params = tool.parameters();
        assert!(params["properties"]["content"].is_object());
        assert!(params["properties"]["channel"].is_object());
        assert!(params["properties"]["chat_id"].is_object());
    }

    #[tokio::test]
    async fn test_message_missing_content() {
        let bus = Arc::new(MessageBus::new());
        let tool = MessageTool::new(bus);
        let ctx = ToolContext::new();

        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_message_with_context() {
        let bus = Arc::new(MessageBus::new());
        let tool = MessageTool::new(bus.clone());

        let ctx = ToolContext::new()
            .with_channel("telegram", "12345");

        let result = tool
            .execute(json!({"content": "Hello!"}), &ctx)
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap().contains("telegram:12345"));
    }
}
```

---

## Task 6: Export Tools from Module

**Files:**
- Modify: `src/tools/mod.rs`

**Add module declarations:**

```rust
pub mod message;
pub mod web;
```

**Add to re-exports:**

```rust
pub use message::MessageTool;
pub use web::{WebFetchTool, WebSearchTool};
```

---

## Task 7: Register Tools in Agent

**Files:**
- Modify: `src/main.rs`

**In the agent setup section, after registering existing tools:**

```rust
// Register web tools if Brave API key is configured
if let Some(brave_key) = config
    .integrations
    .brave
    .api_key
    .as_ref()
    .filter(|k| !k.is_empty())
{
    agent
        .register_tool(Box::new(WebSearchTool::new(brave_key)))
        .await;
    info!("Registered web_search tool");
}

// Always register web_fetch (no API key needed)
agent
    .register_tool(Box::new(WebFetchTool::new()))
    .await;
info!("Registered web_fetch tool");

// Register message tool with bus reference
agent
    .register_tool(Box::new(MessageTool::new(bus.clone())))
    .await;
info!("Registered message tool");
```

**Add imports at top:**

```rust
use zeptoclaw::tools::{MessageTool, WebFetchTool, WebSearchTool};
```

---

## Task 8: Update Onboard Wizard

**Files:**
- Modify: `src/main.rs`

**Add Brave API key prompt in onboard:**

```rust
fn configure_brave(config: &mut Config) -> Result<()> {
    println!("\nWeb Search Setup (Brave)");
    println!("-------------------------");
    println!("Get a free API key at: https://brave.com/search/api/");
    print!("Enter Brave Search API key (or press Enter to skip): ");
    std::io::stdout().flush()?;

    let api_key = read_line()?;
    if !api_key.is_empty() {
        config.integrations.brave.api_key = Some(api_key);
        println!("✓ Brave Search configured");
    }
    Ok(())
}
```

**Call it in cmd_onboard after provider setup:**

```rust
configure_brave(&mut config)?;
```

---

## Task 9: Add Integration Tests

**Files:**
- Modify: `tests/integration.rs`

**Add tests:**

```rust
#[test]
fn test_config_brave_integration() {
    let json = r#"{
        "integrations": {
            "brave": {
                "api_key": "test-brave-key"
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.integrations.brave.api_key,
        Some("test-brave-key".to_string())
    );
}

#[test]
fn test_web_search_tool_creation() {
    use zeptoclaw::tools::WebSearchTool;

    let tool = WebSearchTool::new("test-key");
    assert_eq!(tool.name(), "web_search");
}

#[test]
fn test_web_fetch_tool_creation() {
    use zeptoclaw::tools::WebFetchTool;

    let tool = WebFetchTool::new();
    assert_eq!(tool.name(), "web_fetch");
}

#[test]
fn test_message_tool_creation() {
    use std::sync::Arc;
    use zeptoclaw::bus::MessageBus;
    use zeptoclaw::tools::MessageTool;

    let bus = Arc::new(MessageBus::new());
    let tool = MessageTool::new(bus);
    assert_eq!(tool.name(), "message");
}
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

# Manual test web_fetch (no API key needed)
cargo run -- agent -m "Fetch the content from https://example.com"

# Manual test web_search (requires BRAVE_API_KEY)
export ZEPTOCLAW_INTEGRATIONS_BRAVE_API_KEY="your-key"
cargo run -- agent -m "Search the web for Rust async programming"

# Manual test message tool
cargo run -- gateway
# Then send a message via Telegram asking to send another message
```

---

## Summary

| Tool | Purpose | API Key Required |
|------|---------|------------------|
| `web_search` | Search web via Brave API | Yes (Brave) |
| `web_fetch` | Fetch & extract URL content | No |
| `message` | Send messages to channels | No |

**New Dependency:** `scraper = "0.18"` for HTML parsing

**Config Addition:**
```json
{
  "integrations": {
    "brave": {
      "api_key": "your-brave-api-key"
    }
  }
}
```
