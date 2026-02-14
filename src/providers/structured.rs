//! Structured output (JSON mode) support for LLM providers.
//!
//! This module provides types and helpers for requesting JSON-formatted
//! responses from LLM providers. It supports three output modes:
//!
//! - **Text** — Standard text output (default behavior).
//! - **Json** — Instructs the LLM to return valid JSON.
//! - **JsonSchema** — Instructs the LLM to return JSON conforming to a schema.
//!
//! # Provider-specific behavior
//!
//! Each provider handles structured output differently:
//!
//! - **OpenAI**: Uses the native `response_format` parameter.
//! - **Claude**: Appends JSON instructions to the system prompt.
//!
//! # Example
//!
//! ```rust
//! use zeptoclaw::providers::structured::{OutputFormat, validate_json_response};
//! use serde_json::json;
//!
//! // Simple JSON mode
//! let format = OutputFormat::json();
//! assert!(format.is_json());
//!
//! // JSON Schema mode with strict validation
//! let schema = json!({
//!     "type": "object",
//!     "properties": {
//!         "name": { "type": "string" },
//!         "age": { "type": "integer" }
//!     },
//!     "required": ["name", "age"]
//! });
//! let format = OutputFormat::json_schema("person", schema);
//!
//! // Validate a response
//! let response = r#"{"name": "Alice", "age": 30}"#;
//! let value = validate_json_response(response, &format).unwrap();
//! assert_eq!(value["name"], "Alice");
//! ```

use serde::{Deserialize, Serialize};

/// Output format configuration for LLM responses.
///
/// Controls whether the LLM should return plain text or structured JSON.
/// Each variant maps to provider-specific parameters or prompt modifications.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum OutputFormat {
    /// Standard text output (default behavior).
    #[default]
    Text,
    /// JSON mode — instructs the LLM to return valid JSON.
    ///
    /// For OpenAI: sets `response_format: { "type": "json_object" }`
    /// For Claude: adds JSON instruction to system prompt
    Json,
    /// JSON Schema mode — instructs the LLM to return JSON matching a schema.
    ///
    /// For OpenAI: sets `response_format: { "type": "json_schema", "json_schema": {...} }`
    /// For Claude: adds schema instruction to system prompt
    JsonSchema {
        /// Name for the schema (used by OpenAI's `json_schema` response format).
        name: String,
        /// The JSON Schema that the output must conform to.
        schema: serde_json::Value,
        /// Whether to require strict schema adherence (OpenAI-specific).
        strict: bool,
    },
}

impl OutputFormat {
    /// Create a JSON output format.
    ///
    /// This is a shorthand for `OutputFormat::Json`.
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    ///
    /// let format = OutputFormat::json();
    /// assert!(format.is_json());
    /// ```
    pub fn json() -> Self {
        OutputFormat::Json
    }

    /// Create a JSON Schema output format with strict validation.
    ///
    /// # Arguments
    /// * `name` - Name for the schema (used by OpenAI)
    /// * `schema` - The JSON Schema the output must conform to
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    /// use serde_json::json;
    ///
    /// let format = OutputFormat::json_schema("person", json!({
    ///     "type": "object",
    ///     "properties": {
    ///         "name": { "type": "string" }
    ///     },
    ///     "required": ["name"]
    /// }));
    ///
    /// assert!(format.is_json());
    /// ```
    pub fn json_schema(name: &str, schema: serde_json::Value) -> Self {
        OutputFormat::JsonSchema {
            name: name.to_string(),
            schema,
            strict: true,
        }
    }

    /// Create a JSON Schema output format with lenient validation.
    ///
    /// Same as [`json_schema`](OutputFormat::json_schema) but with `strict` set to `false`,
    /// allowing the LLM more flexibility in its response structure.
    ///
    /// # Arguments
    /// * `name` - Name for the schema (used by OpenAI)
    /// * `schema` - The JSON Schema the output must conform to
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    /// use serde_json::json;
    ///
    /// let format = OutputFormat::json_schema_lenient("result", json!({
    ///     "type": "object"
    /// }));
    ///
    /// assert!(format.is_json());
    /// ```
    pub fn json_schema_lenient(name: &str, schema: serde_json::Value) -> Self {
        OutputFormat::JsonSchema {
            name: name.to_string(),
            schema,
            strict: false,
        }
    }

    /// Returns `true` if this format requests JSON output (either Json or JsonSchema).
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    ///
    /// assert!(!OutputFormat::Text.is_json());
    /// assert!(OutputFormat::Json.is_json());
    /// ```
    pub fn is_json(&self) -> bool {
        matches!(self, OutputFormat::Json | OutputFormat::JsonSchema { .. })
    }

    /// Returns `true` if this format requests standard text output.
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    ///
    /// assert!(OutputFormat::Text.is_text());
    /// assert!(!OutputFormat::Json.is_text());
    /// ```
    pub fn is_text(&self) -> bool {
        matches!(self, OutputFormat::Text)
    }

    /// Convert to OpenAI's `response_format` parameter value.
    ///
    /// Returns `None` for `Text` (no response_format needed), or the
    /// appropriate JSON object for `Json` and `JsonSchema` modes.
    ///
    /// # Returns
    /// - `None` for `Text`
    /// - `Some({"type": "json_object"})` for `Json`
    /// - `Some({"type": "json_schema", "json_schema": {...}})` for `JsonSchema`
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    /// use serde_json::json;
    ///
    /// let format = OutputFormat::json();
    /// let openai_param = format.to_openai_response_format().unwrap();
    /// assert_eq!(openai_param["type"], "json_object");
    /// ```
    pub fn to_openai_response_format(&self) -> Option<serde_json::Value> {
        match self {
            OutputFormat::Text => None,
            OutputFormat::Json => Some(serde_json::json!({
                "type": "json_object"
            })),
            OutputFormat::JsonSchema {
                name,
                schema,
                strict,
            } => Some(serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": name,
                    "schema": schema,
                    "strict": strict,
                }
            })),
        }
    }

    /// Generate a system prompt suffix for Claude to request JSON output.
    ///
    /// Claude does not have a native `response_format` parameter, so we
    /// append instructions to the system prompt instead.
    ///
    /// # Returns
    /// - `None` for `Text`
    /// - `Some(instruction)` for `Json` and `JsonSchema` with appropriate guidance
    ///
    /// # Example
    /// ```
    /// use zeptoclaw::providers::structured::OutputFormat;
    ///
    /// let format = OutputFormat::json();
    /// let suffix = format.to_claude_system_suffix().unwrap();
    /// assert!(suffix.contains("valid JSON"));
    /// ```
    pub fn to_claude_system_suffix(&self) -> Option<String> {
        match self {
            OutputFormat::Text => None,
            OutputFormat::Json => Some(
                "\n\nIMPORTANT: You MUST respond with valid JSON only. No markdown, no explanation, just JSON.".to_string()
            ),
            OutputFormat::JsonSchema { schema, .. } => {
                let pretty_schema = serde_json::to_string_pretty(schema)
                    .unwrap_or_else(|_| schema.to_string());
                Some(format!(
                    "\n\nIMPORTANT: You MUST respond with valid JSON matching this schema:\n```json\n{}\n```\nRespond with JSON only, no markdown fences, no explanation.",
                    pretty_schema
                ))
            }
        }
    }
}

/// Validate a response string against the expected output format.
///
/// - For `Text`: returns `Err` since validation is not applicable.
/// - For `Json`: parses the response as JSON and returns the parsed value.
/// - For `JsonSchema`: parses the response as JSON, then checks that all
///   required top-level keys (from the schema's `"required"` field) are present.
///
/// # Arguments
/// * `response` - The raw response string from the LLM
/// * `format` - The expected output format
///
/// # Returns
/// - `Ok(serde_json::Value)` if the response is valid JSON matching the format
/// - `Err(String)` with a description of the validation failure
///
/// # Example
/// ```
/// use zeptoclaw::providers::structured::{OutputFormat, validate_json_response};
///
/// let format = OutputFormat::json();
/// let result = validate_json_response(r#"{"key": "value"}"#, &format);
/// assert!(result.is_ok());
///
/// let result = validate_json_response("not json", &format);
/// assert!(result.is_err());
/// ```
pub fn validate_json_response(
    response: &str,
    format: &OutputFormat,
) -> Result<serde_json::Value, String> {
    match format {
        OutputFormat::Text => Err("Not in JSON mode".to_string()),
        OutputFormat::Json => {
            serde_json::from_str(response).map_err(|e| format!("Invalid JSON: {}", e))
        }
        OutputFormat::JsonSchema { schema, .. } => {
            let value: serde_json::Value =
                serde_json::from_str(response).map_err(|e| format!("Invalid JSON: {}", e))?;

            // Check required top-level keys if the schema specifies them
            if let Some(required) = schema.get("required") {
                if let Some(required_arr) = required.as_array() {
                    for req in required_arr {
                        if let Some(key) = req.as_str() {
                            if value.get(key).is_none() {
                                return Err(format!("Missing required key: {}", key));
                            }
                        }
                    }
                }
            }

            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_output_format_default_is_text() {
        let format = OutputFormat::default();
        assert_eq!(format, OutputFormat::Text);
    }

    #[test]
    fn test_output_format_is_json() {
        assert!(OutputFormat::Json.is_json());
        assert!(OutputFormat::JsonSchema {
            name: "test".to_string(),
            schema: json!({}),
            strict: true,
        }
        .is_json());
    }

    #[test]
    fn test_output_format_is_text() {
        assert!(OutputFormat::Text.is_text());
        assert!(!OutputFormat::Json.is_text());
        assert!(!OutputFormat::JsonSchema {
            name: "test".to_string(),
            schema: json!({}),
            strict: true,
        }
        .is_text());
    }

    #[test]
    fn test_json_constructor() {
        let format = OutputFormat::json();
        assert_eq!(format, OutputFormat::Json);
        assert!(format.is_json());
    }

    #[test]
    fn test_json_schema_constructor() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });
        let format = OutputFormat::json_schema("person", schema.clone());

        match format {
            OutputFormat::JsonSchema {
                name,
                schema: s,
                strict,
            } => {
                assert_eq!(name, "person");
                assert_eq!(s, schema);
                assert!(strict);
            }
            _ => panic!("Expected JsonSchema variant"),
        }
    }

    #[test]
    fn test_json_schema_lenient_constructor() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });
        let format = OutputFormat::json_schema_lenient("result", schema.clone());

        match format {
            OutputFormat::JsonSchema {
                name,
                schema: s,
                strict,
            } => {
                assert_eq!(name, "result");
                assert_eq!(s, schema);
                assert!(!strict);
            }
            _ => panic!("Expected JsonSchema variant"),
        }
    }

    #[test]
    fn test_openai_format_text() {
        let format = OutputFormat::Text;
        assert!(format.to_openai_response_format().is_none());
    }

    #[test]
    fn test_openai_format_json() {
        let format = OutputFormat::Json;
        let result = format.to_openai_response_format().unwrap();
        assert_eq!(result, json!({"type": "json_object"}));
    }

    #[test]
    fn test_openai_format_json_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let format = OutputFormat::json_schema("person", schema.clone());
        let result = format.to_openai_response_format().unwrap();

        assert_eq!(result["type"], "json_schema");
        assert_eq!(result["json_schema"]["name"], "person");
        assert_eq!(result["json_schema"]["schema"], schema);
        assert_eq!(result["json_schema"]["strict"], true);
    }

    #[test]
    fn test_claude_suffix_text() {
        let format = OutputFormat::Text;
        assert!(format.to_claude_system_suffix().is_none());
    }

    #[test]
    fn test_claude_suffix_json() {
        let format = OutputFormat::Json;
        let suffix = format.to_claude_system_suffix().unwrap();
        assert!(suffix.contains("valid JSON"));
        assert!(suffix.contains("IMPORTANT"));
        assert!(suffix.contains("No markdown"));
    }

    #[test]
    fn test_claude_suffix_json_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let format = OutputFormat::json_schema("person", schema);
        let suffix = format.to_claude_system_suffix().unwrap();

        assert!(suffix.contains("valid JSON matching this schema"));
        assert!(suffix.contains("\"name\""));
        assert!(suffix.contains("IMPORTANT"));
    }

    #[test]
    fn test_validate_json_response_text_mode() {
        let format = OutputFormat::Text;
        let result = validate_json_response(r#"{"key": "value"}"#, &format);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Not in JSON mode");
    }

    #[test]
    fn test_validate_json_response_valid_json() {
        let format = OutputFormat::Json;
        let result = validate_json_response(r#"{"key": "value"}"#, &format);
        assert!(result.is_ok());
        let value = result.unwrap();
        assert_eq!(value["key"], "value");
    }

    #[test]
    fn test_validate_json_response_invalid_json() {
        let format = OutputFormat::Json;
        let result = validate_json_response("not valid json {", &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.starts_with("Invalid JSON:"));
    }

    #[test]
    fn test_validate_json_schema_with_required_keys() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name", "age"]
        });
        let format = OutputFormat::json_schema("person", schema);

        let result = validate_json_response(r#"{"name": "Alice", "age": 30}"#, &format);
        assert!(result.is_ok());
        let value = result.unwrap();
        assert_eq!(value["name"], "Alice");
        assert_eq!(value["age"], 30);
    }

    #[test]
    fn test_validate_json_schema_missing_required_key() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name", "age"]
        });
        let format = OutputFormat::json_schema("person", schema);

        let result = validate_json_response(r#"{"name": "Alice"}"#, &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Missing required key: age"));
    }

    #[test]
    fn test_output_format_serialize_roundtrip() {
        let formats = vec![
            OutputFormat::Text,
            OutputFormat::Json,
            OutputFormat::json_schema(
                "test",
                json!({
                    "type": "object",
                    "properties": {
                        "field": { "type": "string" }
                    }
                }),
            ),
        ];

        for original in formats {
            let serialized = serde_json::to_string(&original).unwrap();
            let deserialized: OutputFormat = serde_json::from_str(&serialized).unwrap();
            assert_eq!(original, deserialized);
        }
    }

    #[test]
    fn test_output_format_clone() {
        let original = OutputFormat::json_schema(
            "test",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }),
        );

        let cloned = original.clone();
        assert_eq!(original, cloned);
    }
}
