# WhatsApp Cloud API & Google Sheets Integration Plan

> **For Claude:** Use this plan to implement WhatsApp and Google Sheets tools task-by-task.

**Goal:** Add WhatsApp Cloud API and Google Sheets tools to ZeptoClaw for Malaysian e-commerce seller use cases - customer communication and simple inventory/CRM tracking.

**Architecture:** Implement two new integration modules with corresponding tools. WhatsApp uses Meta's Cloud API (REST). Google Sheets uses Google Sheets API v4. Both follow the existing `Tool` trait pattern.

**Tech Stack:** Rust, reqwest, serde, async-trait, oauth2 (for Google auth)

---

## Use Cases for Malaysian E-Commerce Sellers

| Use Case | Tool | Example |
|----------|------|---------|
| Order status updates | WhatsApp | "Your order #1234 shipped via J&T" |
| Shipping notifications | WhatsApp | "Package arriving tomorrow, tracking: JT123456" |
| Payment confirmations | WhatsApp | "Payment RM49.90 received, terima kasih!" |
| Customer inquiries | WhatsApp | Auto-reply to "bila sampai?" queries |
| Inventory tracking | Google Sheets | Update stock count after sale |
| Order logging | Google Sheets | Append new order to sheet |
| Customer CRM | Google Sheets | Track customer purchase history |
| Sales reports | Google Sheets | Read daily sales totals |

---

## Task 1: Add Dependencies

**Files:**
- Modify: `Cargo.toml`

**Changes:**

Add to `[dependencies]`:

```toml
# =============================================================================
# INTEGRATIONS
# =============================================================================
# OAuth2 for Google APIs authentication
oauth2 = "4"
# Base64 encoding for credentials
base64 = "0.22"
```

**Verification:**
```bash
cargo check
```

---

## Task 2: Add Integration Configs

**Files:**
- Modify: `src/config/types.rs`

**Step 1: Add WhatsApp config struct**

```rust
/// WhatsApp Cloud API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhatsAppConfig {
    /// WhatsApp Business Account ID
    #[serde(default)]
    pub business_account_id: Option<String>,

    /// Phone Number ID (from Meta dashboard)
    #[serde(default)]
    pub phone_number_id: Option<String>,

    /// Permanent Access Token (from Meta dashboard)
    #[serde(default)]
    pub access_token: Option<String>,

    /// Webhook verify token (for incoming messages)
    #[serde(default)]
    pub webhook_verify_token: Option<String>,
}
```

**Step 2: Add Google Sheets config struct**

```rust
/// Google Sheets API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GoogleSheetsConfig {
    /// Service account credentials JSON (base64 encoded)
    /// Get from: Google Cloud Console > IAM > Service Accounts > Keys
    #[serde(default)]
    pub service_account_base64: Option<String>,

    /// Alternative: OAuth2 client credentials
    #[serde(default)]
    pub client_id: Option<String>,

    #[serde(default)]
    pub client_secret: Option<String>,

    /// Refresh token (if using OAuth2 flow)
    #[serde(default)]
    pub refresh_token: Option<String>,
}
```

**Step 3: Update IntegrationsConfig**

```rust
/// External service integrations configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationsConfig {
    /// Brave Search API for web_search tool
    #[serde(default)]
    pub brave: BraveConfig,

    /// WhatsApp Cloud API for messaging
    #[serde(default)]
    pub whatsapp: WhatsAppConfig,

    /// Google Sheets API for data management
    #[serde(default)]
    pub google_sheets: GoogleSheetsConfig,
}
```

---

## Task 3: Create WhatsApp Tool Module

**Files:**
- Create: `src/tools/whatsapp.rs`

**Implementation:**

```rust
//! WhatsApp Cloud API tool for sending messages.
//!
//! Enables the agent to send WhatsApp messages to customers.
//! Uses Meta's WhatsApp Cloud API.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info};

use crate::error::{PicoError, Result};
use crate::tools::{Tool, ToolContext};

const WHATSAPP_API_URL: &str = "https://graph.facebook.com/v18.0";

/// WhatsApp messaging tool using Cloud API.
pub struct WhatsAppTool {
    phone_number_id: String,
    access_token: String,
    client: Client,
}

impl WhatsAppTool {
    /// Create a new WhatsApp tool.
    pub fn new(phone_number_id: &str, access_token: &str) -> Self {
        Self {
            phone_number_id: phone_number_id.to_string(),
            access_token: access_token.to_string(),
            client: Client::new(),
        }
    }
}

#[derive(Debug, Serialize)]
struct WhatsAppMessage {
    messaging_product: String,
    recipient_type: String,
    to: String,
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<TextContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<TemplateContent>,
}

#[derive(Debug, Serialize)]
struct TextContent {
    preview_url: bool,
    body: String,
}

#[derive(Debug, Serialize)]
struct TemplateContent {
    name: String,
    language: TemplateLanguage,
    #[serde(skip_serializing_if = "Option::is_none")]
    components: Option<Vec<TemplateComponent>>,
}

#[derive(Debug, Serialize)]
struct TemplateLanguage {
    code: String,
}

#[derive(Debug, Serialize)]
struct TemplateComponent {
    #[serde(rename = "type")]
    component_type: String,
    parameters: Vec<TemplateParameter>,
}

#[derive(Debug, Serialize)]
struct TemplateParameter {
    #[serde(rename = "type")]
    param_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct WhatsAppResponse {
    messages: Option<Vec<MessageResponse>>,
    error: Option<WhatsAppError>,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct WhatsAppError {
    message: String,
    code: i32,
}

#[async_trait]
impl Tool for WhatsAppTool {
    fn name(&self) -> &str {
        "whatsapp_send"
    }

    fn description(&self) -> &str {
        "Send a WhatsApp message to a customer. Use for order updates, shipping notifications, or customer communication."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient phone number with country code (e.g., '60123456789' for Malaysia)"
                },
                "message": {
                    "type": "string",
                    "description": "Message text to send (max 4096 characters)"
                },
                "template": {
                    "type": "string",
                    "description": "Optional: Template name for pre-approved messages (e.g., 'order_shipped')"
                },
                "template_params": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional: Parameters for template placeholders"
                }
            },
            "required": ["to", "message"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String> {
        let to = args["to"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'to' phone number".into()))?;

        let message = args["message"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'message' content".into()))?;

        // Validate phone number format (basic check)
        if !to.chars().all(|c| c.is_ascii_digit()) {
            return Err(PicoError::Tool(
                "Phone number must contain only digits with country code".into(),
            ));
        }

        let template_name = args["template"].as_str();
        let template_params: Option<Vec<String>> = args["template_params"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

        let wa_message = if let Some(template) = template_name {
            // Use template message
            let components = template_params.map(|params| {
                vec![TemplateComponent {
                    component_type: "body".to_string(),
                    parameters: params
                        .into_iter()
                        .map(|text| TemplateParameter {
                            param_type: "text".to_string(),
                            text,
                        })
                        .collect(),
                }]
            });

            WhatsAppMessage {
                messaging_product: "whatsapp".to_string(),
                recipient_type: "individual".to_string(),
                to: to.to_string(),
                msg_type: "template".to_string(),
                text: None,
                template: Some(TemplateContent {
                    name: template.to_string(),
                    language: TemplateLanguage {
                        code: "ms".to_string(), // Malay default
                    },
                    components,
                }),
            }
        } else {
            // Plain text message (only works within 24h window)
            WhatsAppMessage {
                messaging_product: "whatsapp".to_string(),
                recipient_type: "individual".to_string(),
                to: to.to_string(),
                msg_type: "text".to_string(),
                text: Some(TextContent {
                    preview_url: false,
                    body: message.to_string(),
                }),
                template: None,
            }
        };

        let url = format!(
            "{}/{}/messages",
            WHATSAPP_API_URL, self.phone_number_id
        );

        debug!("Sending WhatsApp message to {}", to);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&wa_message)
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("WhatsApp request failed: {}", e)))?;

        let status = response.status();
        let body: WhatsAppResponse = response
            .json()
            .await
            .map_err(|e| PicoError::Tool(format!("Failed to parse response: {}", e)))?;

        if let Some(error) = body.error {
            return Err(PicoError::Tool(format!(
                "WhatsApp API error ({}): {}",
                error.code, error.message
            )));
        }

        if let Some(messages) = body.messages {
            if let Some(msg) = messages.first() {
                info!("WhatsApp message sent: {}", msg.id);
                return Ok(format!("Message sent to {} (ID: {})", to, msg.id));
            }
        }

        Err(PicoError::Tool(format!(
            "Unexpected response status: {}",
            status
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whatsapp_tool_properties() {
        let tool = WhatsAppTool::new("123456789", "test-token");
        assert_eq!(tool.name(), "whatsapp_send");
        assert!(tool.description().contains("WhatsApp"));
    }

    #[test]
    fn test_whatsapp_parameters() {
        let tool = WhatsAppTool::new("123456789", "test-token");
        let params = tool.parameters();
        assert!(params["properties"]["to"].is_object());
        assert!(params["properties"]["message"].is_object());
        assert!(params["properties"]["template"].is_object());
    }

    #[tokio::test]
    async fn test_whatsapp_missing_to() {
        let tool = WhatsAppTool::new("123456789", "test-token");
        let ctx = ToolContext::new();
        let result = tool.execute(json!({"message": "Hello"}), &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("to"));
    }

    #[tokio::test]
    async fn test_whatsapp_invalid_phone() {
        let tool = WhatsAppTool::new("123456789", "test-token");
        let ctx = ToolContext::new();
        let result = tool
            .execute(json!({"to": "+60-123", "message": "Hello"}), &ctx)
            .await;
        assert!(result.is_err());
    }
}
```

---

## Task 4: Create Google Sheets Tool Module

**Files:**
- Create: `src/tools/gsheets.rs`

**Implementation:**

```rust
//! Google Sheets API tool for data management.
//!
//! Enables reading and writing to Google Sheets for inventory tracking,
//! order logging, and simple CRM use cases.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info};

use crate::error::{PicoError, Result};
use crate::tools::{Tool, ToolContext};

const SHEETS_API_URL: &str = "https://sheets.googleapis.com/v4/spreadsheets";

/// Google Sheets authentication helper.
#[derive(Clone)]
pub struct GoogleAuth {
    access_token: String,
    expires_at: std::time::Instant,
}

/// Google Sheets tool for reading and writing data.
pub struct GoogleSheetsTool {
    client: Client,
    access_token: String,
}

impl GoogleSheetsTool {
    /// Create with a pre-obtained access token.
    pub fn new(access_token: &str) -> Self {
        Self {
            client: Client::new(),
            access_token: access_token.to_string(),
        }
    }

    /// Create from service account credentials (base64-encoded JSON).
    pub async fn from_service_account(credentials_base64: &str) -> Result<Self> {
        use base64::Engine;

        let credentials_json = base64::engine::general_purpose::STANDARD
            .decode(credentials_base64)
            .map_err(|e| PicoError::Config(format!("Invalid base64 credentials: {}", e)))?;

        let credentials: ServiceAccountCredentials = serde_json::from_slice(&credentials_json)
            .map_err(|e| PicoError::Config(format!("Invalid credentials JSON: {}", e)))?;

        // Generate JWT for service account
        let access_token = Self::get_service_account_token(&credentials).await?;

        Ok(Self {
            client: Client::new(),
            access_token,
        })
    }

    async fn get_service_account_token(creds: &ServiceAccountCredentials) -> Result<String> {
        // Create JWT claim
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = json!({
            "iss": creds.client_email,
            "scope": "https://www.googleapis.com/auth/spreadsheets",
            "aud": "https://oauth2.googleapis.com/token",
            "iat": now,
            "exp": now + 3600,
        });

        // For production: Sign JWT with private key and exchange for access token
        // This is a simplified version - full implementation would use the jwt crate

        // Exchange JWT for access token
        let client = Client::new();
        let response = client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &claims.to_string()), // In production: signed JWT
            ])
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("Token exchange failed: {}", e)))?;

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| PicoError::Tool(format!("Failed to parse token: {}", e)))?;

        Ok(token_response.access_token)
    }
}

#[derive(Debug, Deserialize)]
struct ServiceAccountCredentials {
    client_email: String,
    private_key: String,
    private_key_id: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct SheetValues {
    values: Option<Vec<Vec<String>>>,
}

#[async_trait]
impl Tool for GoogleSheetsTool {
    fn name(&self) -> &str {
        "google_sheets"
    }

    fn description(&self) -> &str {
        "Read from or write to Google Sheets. Use for inventory tracking, order logging, or customer data."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "spreadsheet_id": {
                    "type": "string",
                    "description": "The spreadsheet ID (from the URL: docs.google.com/spreadsheets/d/{ID}/edit)"
                },
                "action": {
                    "type": "string",
                    "enum": ["read", "append", "update"],
                    "description": "Action: 'read' cells, 'append' new row, or 'update' existing cells"
                },
                "range": {
                    "type": "string",
                    "description": "Cell range in A1 notation (e.g., 'Sheet1!A1:D10' or 'Orders!A:F')"
                },
                "values": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "description": "Data rows to write (for append/update). Each inner array is a row."
                }
            },
            "required": ["spreadsheet_id", "action", "range"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String> {
        let spreadsheet_id = args["spreadsheet_id"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'spreadsheet_id'".into()))?;

        let action = args["action"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'action'".into()))?;

        let range = args["range"]
            .as_str()
            .ok_or_else(|| PicoError::Tool("Missing 'range'".into()))?;

        match action {
            "read" => self.read_range(spreadsheet_id, range).await,
            "append" => {
                let values = self.extract_values(&args)?;
                self.append_rows(spreadsheet_id, range, values).await
            }
            "update" => {
                let values = self.extract_values(&args)?;
                self.update_range(spreadsheet_id, range, values).await
            }
            _ => Err(PicoError::Tool(format!("Unknown action: {}", action))),
        }
    }
}

impl GoogleSheetsTool {
    fn extract_values(&self, args: &Value) -> Result<Vec<Vec<String>>> {
        args["values"]
            .as_array()
            .ok_or_else(|| PicoError::Tool("Missing 'values' for write operation".into()))?
            .iter()
            .map(|row| {
                row.as_array()
                    .ok_or_else(|| PicoError::Tool("Each row must be an array".into()))?
                    .iter()
                    .map(|cell| Ok(cell.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .collect()
    }

    async fn read_range(&self, spreadsheet_id: &str, range: &str) -> Result<String> {
        let url = format!(
            "{}/{}/values/{}",
            SHEETS_API_URL,
            spreadsheet_id,
            urlencoding::encode(range)
        );

        debug!("Reading from Google Sheets: {}", range);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("Sheets API request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Tool(format!("Sheets API error: {}", error_text)));
        }

        let data: SheetValues = response
            .json()
            .await
            .map_err(|e| PicoError::Tool(format!("Failed to parse response: {}", e)))?;

        let values = data.values.unwrap_or_default();

        if values.is_empty() {
            return Ok("No data found in range".to_string());
        }

        // Format as readable table
        let mut output = String::new();
        for (i, row) in values.iter().enumerate() {
            output.push_str(&format!("Row {}: {}\n", i + 1, row.join(" | ")));
        }

        info!("Read {} rows from Google Sheets", values.len());
        Ok(output)
    }

    async fn append_rows(
        &self,
        spreadsheet_id: &str,
        range: &str,
        values: Vec<Vec<String>>,
    ) -> Result<String> {
        let url = format!(
            "{}/{}/values/{}:append?valueInputOption=USER_ENTERED&insertDataOption=INSERT_ROWS",
            SHEETS_API_URL,
            spreadsheet_id,
            urlencoding::encode(range)
        );

        let body = json!({
            "values": values
        });

        debug!("Appending {} rows to Google Sheets", values.len());

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("Sheets API request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Tool(format!("Sheets API error: {}", error_text)));
        }

        info!("Appended {} rows to Google Sheets", values.len());
        Ok(format!("Successfully appended {} row(s)", values.len()))
    }

    async fn update_range(
        &self,
        spreadsheet_id: &str,
        range: &str,
        values: Vec<Vec<String>>,
    ) -> Result<String> {
        let url = format!(
            "{}/{}/values/{}?valueInputOption=USER_ENTERED",
            SHEETS_API_URL,
            spreadsheet_id,
            urlencoding::encode(range)
        );

        let body = json!({
            "values": values
        });

        debug!("Updating range {} in Google Sheets", range);

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| PicoError::Tool(format!("Sheets API request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Tool(format!("Sheets API error: {}", error_text)));
        }

        info!("Updated range {} in Google Sheets", range);
        Ok(format!("Successfully updated range {}", range))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gsheets_tool_properties() {
        let tool = GoogleSheetsTool::new("test-token");
        assert_eq!(tool.name(), "google_sheets");
        assert!(tool.description().contains("Google Sheets"));
    }

    #[test]
    fn test_gsheets_parameters() {
        let tool = GoogleSheetsTool::new("test-token");
        let params = tool.parameters();
        assert!(params["properties"]["spreadsheet_id"].is_object());
        assert!(params["properties"]["action"].is_object());
        assert!(params["properties"]["range"].is_object());
    }

    #[tokio::test]
    async fn test_gsheets_missing_spreadsheet_id() {
        let tool = GoogleSheetsTool::new("test-token");
        let ctx = ToolContext::new();
        let result = tool
            .execute(json!({"action": "read", "range": "A1:B2"}), &ctx)
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_values() {
        let tool = GoogleSheetsTool::new("test-token");
        let args = json!({
            "values": [
                ["A", "B", "C"],
                ["1", "2", "3"]
            ]
        });
        let values = tool.extract_values(&args).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], vec!["A", "B", "C"]);
    }
}
```

---

## Task 5: Export Tools from Module

**Files:**
- Modify: `src/tools/mod.rs`

**Add module declarations:**

```rust
pub mod gsheets;
pub mod whatsapp;
```

**Add to re-exports:**

```rust
pub use gsheets::GoogleSheetsTool;
pub use whatsapp::WhatsAppTool;
```

---

## Task 6: Export from Library

**Files:**
- Modify: `src/lib.rs`

**Add to tools re-exports:**

```rust
pub use tools::{
    // existing exports...
    GoogleSheetsTool,
    WhatsAppTool,
};
```

---

## Task 7: Register Tools in Agent

**Files:**
- Modify: `src/main.rs`

**In the agent setup section:**

```rust
// Register WhatsApp tool if configured
if let Some(ref whatsapp) = config.integrations.whatsapp {
    if let (Some(phone_id), Some(token)) = (&whatsapp.phone_number_id, &whatsapp.access_token) {
        if !phone_id.is_empty() && !token.is_empty() {
            agent
                .register_tool(Box::new(WhatsAppTool::new(phone_id, token)))
                .await;
            info!("Registered whatsapp_send tool");
        }
    }
}

// Register Google Sheets tool if configured
if let Some(ref gsheets) = config.integrations.google_sheets {
    // Try service account first, then OAuth token
    if let Some(ref sa_creds) = gsheets.service_account_base64 {
        if !sa_creds.is_empty() {
            match GoogleSheetsTool::from_service_account(sa_creds).await {
                Ok(tool) => {
                    agent.register_tool(Box::new(tool)).await;
                    info!("Registered google_sheets tool (service account)");
                }
                Err(e) => {
                    warn!("Failed to initialize Google Sheets: {}", e);
                }
            }
        }
    }
}
```

**Add imports at top:**

```rust
use zeptoclaw::tools::{GoogleSheetsTool, WhatsAppTool};
```

---

## Task 8: Update Onboard Wizard

**Files:**
- Modify: `src/main.rs`

**Add WhatsApp configuration:**

```rust
fn configure_whatsapp(config: &mut Config) -> Result<()> {
    println!("\nWhatsApp Business Setup");
    println!("-----------------------");
    println!("Get credentials from: https://developers.facebook.com/apps/");
    println!("You need: Phone Number ID and Permanent Access Token");

    print!("Enter Phone Number ID (or press Enter to skip): ");
    std::io::stdout().flush()?;
    let phone_id = read_line()?;

    if !phone_id.is_empty() {
        print!("Enter Access Token: ");
        std::io::stdout().flush()?;
        let token = read_line()?;

        config.integrations.whatsapp.phone_number_id = Some(phone_id);
        config.integrations.whatsapp.access_token = Some(token);
        println!("WhatsApp configured");
    }
    Ok(())
}
```

**Add Google Sheets configuration:**

```rust
fn configure_google_sheets(config: &mut Config) -> Result<()> {
    println!("\nGoogle Sheets Setup");
    println!("-------------------");
    println!("Get service account from: Google Cloud Console > IAM > Service Accounts");
    println!("Create a key (JSON), then base64 encode it: base64 -i service-account.json");

    print!("Enter base64-encoded service account JSON (or press Enter to skip): ");
    std::io::stdout().flush()?;
    let creds = read_line()?;

    if !creds.is_empty() {
        config.integrations.google_sheets.service_account_base64 = Some(creds);
        println!("Google Sheets configured");
    }
    Ok(())
}
```

**Call in cmd_onboard:**

```rust
configure_whatsapp(&mut config)?;
configure_google_sheets(&mut config)?;
```

---

## Task 9: Add Integration Tests

**Files:**
- Modify: `tests/integration.rs`

**Add tests:**

```rust
#[test]
fn test_config_whatsapp_integration() {
    let json = r#"{
        "integrations": {
            "whatsapp": {
                "phone_number_id": "123456789",
                "access_token": "test-token"
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    let wa = config.integrations.whatsapp;
    assert_eq!(wa.phone_number_id, Some("123456789".to_string()));
    assert_eq!(wa.access_token, Some("test-token".to_string()));
}

#[test]
fn test_config_google_sheets_integration() {
    let json = r#"{
        "integrations": {
            "google_sheets": {
                "service_account_base64": "eyJ0eXBlIjoic2VydmljZV9hY2NvdW50In0="
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    let gs = config.integrations.google_sheets;
    assert!(gs.service_account_base64.is_some());
}

#[test]
fn test_whatsapp_tool_creation() {
    use zeptoclaw::tools::WhatsAppTool;

    let tool = WhatsAppTool::new("123456789", "test-token");
    assert_eq!(tool.name(), "whatsapp_send");
}

#[test]
fn test_gsheets_tool_creation() {
    use zeptoclaw::tools::GoogleSheetsTool;

    let tool = GoogleSheetsTool::new("test-token");
    assert_eq!(tool.name(), "google_sheets");
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

# Manual test WhatsApp (requires credentials)
export ZEPTOCLAW_INTEGRATIONS_WHATSAPP_PHONE_NUMBER_ID="your-phone-id"
export ZEPTOCLAW_INTEGRATIONS_WHATSAPP_ACCESS_TOKEN="your-token"
cargo run -- agent -m "Send a WhatsApp message to 60123456789 saying 'Test from ZeptoClaw'"

# Manual test Google Sheets (requires credentials)
export ZEPTOCLAW_INTEGRATIONS_GOOGLE_SHEETS_SERVICE_ACCOUNT_BASE64="your-base64-creds"
cargo run -- agent -m "Read rows A1:D5 from spreadsheet 1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms"
```

---

## Summary

| Tool | Purpose | Credentials Required |
|------|---------|---------------------|
| `whatsapp_send` | Send WhatsApp messages | Phone Number ID + Access Token |
| `google_sheets` | Read/write spreadsheets | Service Account JSON (base64) |

**New Dependencies:**
- `oauth2 = "4"` - Google OAuth2 authentication
- `base64 = "0.22"` - Credential encoding

**Config Addition:**
```json
{
  "integrations": {
    "whatsapp": {
      "phone_number_id": "123456789012345",
      "access_token": "EAA..."
    },
    "google_sheets": {
      "service_account_base64": "eyJ0eXBlIjoic2VydmljZV9hY2NvdW50Ii..."
    }
  }
}
```

---

## Setup Guides for Malaysian Sellers

### WhatsApp Business API Setup

1. **Create Meta Developer Account**: https://developers.facebook.com/
2. **Create App**: Choose "Business" type
3. **Add WhatsApp Product**: Get free test phone number
4. **Get Credentials**:
   - Phone Number ID: WhatsApp > API Setup
   - Access Token: Generate permanent token
5. **Template Approval**: Submit templates for proactive messaging (order_shipped, payment_received, etc.)

### Google Sheets Setup

1. **Create Google Cloud Project**: https://console.cloud.google.com/
2. **Enable Sheets API**: APIs & Services > Enable APIs
3. **Create Service Account**: IAM > Service Accounts > Create
4. **Generate Key**: Service Account > Keys > Add Key > JSON
5. **Base64 Encode**: `base64 -i service-account.json | tr -d '\n'`
6. **Share Spreadsheet**: Share with service account email (found in JSON)

---

## Malaysian E-Commerce Templates

### WhatsApp Message Templates (Submit for Approval)

```
Template: order_confirmation
Language: ms (Malay)
Body: Terima kasih atas pesanan anda! Order #{{1}} telah diterima. Jumlah: RM{{2}}

Template: order_shipped
Language: ms (Malay)
Body: Pesanan #{{1}} telah dihantar via {{2}}. Tracking: {{3}}. Anggaran tiba: {{4}}

Template: payment_received
Language: ms (Malay)
Body: Pembayaran RM{{1}} untuk order #{{2}} telah diterima. Terima kasih!
```

### Google Sheets Structure

**Orders Sheet:**
| Order ID | Customer | Phone | Items | Total | Status | Tracking |
|----------|----------|-------|-------|-------|--------|----------|
| ORD001 | Ahmad | 60123456789 | Baju Melayu x2 | RM89 | Shipped | JT123 |

**Inventory Sheet:**
| SKU | Product | Stock | Min Stock | Last Updated |
|-----|---------|-------|-----------|--------------|
| BM001 | Baju Melayu Biru | 15 | 5 | 2026-02-13 |

---

*Last updated: 2026-02-13*
