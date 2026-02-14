//! Batch processing of multiple prompts from a file.
//!
//! This module provides types and logic for loading prompts from files
//! (plain text or JSON/JSONL) and formatting batch results in text or JSONL format.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{Result, ZeptoError};

/// Configuration for batch processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchConfig {
    /// Number of prompts to process concurrently (1 = sequential).
    pub concurrency: usize,
    /// Output format for batch results.
    pub output_format: BatchOutputFormat,
    /// Whether to stop processing on the first error.
    pub stop_on_error: bool,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            concurrency: 1,
            output_format: BatchOutputFormat::default(),
            stop_on_error: false,
        }
    }
}

/// Output format for batch results.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BatchOutputFormat {
    /// Plain text output, each response separated by a blank line.
    #[default]
    Text,
    /// Each response as a JSON object per line.
    Jsonl,
}

/// Result of processing a single prompt in a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    /// Zero-based index of the prompt in the batch.
    pub index: usize,
    /// The original prompt text.
    pub prompt: String,
    /// The response text, if successful.
    pub response: Option<String>,
    /// The error message, if failed.
    pub error: Option<String>,
    /// Time taken to process this prompt, in milliseconds.
    pub duration_ms: u64,
}

/// Load prompts from a file.
///
/// Supports two formats:
/// - **JSON/JSONL** (`.json` or `.jsonl` extension): Each line is parsed as either
///   a bare JSON string (`"prompt text"`) or a JSON object with a `"prompt"` field
///   (`{"prompt": "prompt text"}`).
/// - **Plain text** (any other extension): One prompt per non-empty line.
///
/// In both modes, lines are trimmed of whitespace. Empty lines and lines starting
/// with `#` (comments) are skipped.
///
/// Returns an error if the file does not exist or yields no prompts after filtering.
pub fn load_prompts(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Err(ZeptoError::NotFound(format!(
            "Batch file not found: {}",
            path.display()
        )));
    }

    let content = std::fs::read_to_string(path)?;

    let is_json = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json") || ext.eq_ignore_ascii_case("jsonl"))
        .unwrap_or(false);

    let prompts: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            if is_json {
                parse_json_prompt(line)
            } else {
                Some(line.to_string())
            }
        })
        .collect();

    if prompts.is_empty() {
        return Err(ZeptoError::Config(
            "Batch file contains no valid prompts".to_string(),
        ));
    }

    Ok(prompts)
}

/// Attempt to parse a single line as a JSON prompt.
///
/// Tries two forms:
/// 1. A bare JSON string: `"some prompt text"`
/// 2. A JSON object with a `"prompt"` field: `{"prompt": "some prompt text"}`
fn parse_json_prompt(line: &str) -> Option<String> {
    // Try as a bare JSON string first.
    if let Ok(s) = serde_json::from_str::<String>(line) {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed);
    }

    // Try as a JSON object with a "prompt" field.
    #[derive(Deserialize)]
    struct PromptObj {
        prompt: String,
    }

    if let Ok(obj) = serde_json::from_str::<PromptObj>(line) {
        let trimmed = obj.prompt.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed);
    }

    None
}

/// Format batch results into a string according to the specified output format.
///
/// - **Text**: Human-readable blocks separated by blank lines.
/// - **Jsonl**: One JSON object per line, suitable for machine consumption.
pub fn format_results(results: &[BatchResult], format: &BatchOutputFormat) -> String {
    match format {
        BatchOutputFormat::Text => format_results_text(results),
        BatchOutputFormat::Jsonl => format_results_jsonl(results),
    }
}

fn format_results_text(results: &[BatchResult]) -> String {
    let mut output = String::new();
    for (i, result) in results.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        output.push_str(&format!("--- Prompt {} ---\n", result.index));
        output.push_str(&result.prompt);
        output.push('\n');
        if let Some(ref error) = result.error {
            output.push_str("--- Error ---\n");
            output.push_str(error);
            output.push('\n');
        } else if let Some(ref response) = result.response {
            output.push_str("--- Response ---\n");
            output.push_str(response);
            output.push('\n');
        }
    }
    output
}

fn format_results_jsonl(results: &[BatchResult]) -> String {
    results
        .iter()
        .filter_map(|r| serde_json::to_string(r).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper to create a temporary file with given content and extension.
    fn create_temp_file(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = std::fs::File::create(&path).expect("Failed to create temp file");
        file.write_all(content.as_bytes())
            .expect("Failed to write temp file");
        path
    }

    #[test]
    fn test_batch_config_defaults() {
        let config = BatchConfig::default();
        assert_eq!(config.concurrency, 1);
        assert_eq!(config.output_format, BatchOutputFormat::Text);
        assert!(!config.stop_on_error);
    }

    #[test]
    fn test_batch_output_format_default_is_text() {
        let format = BatchOutputFormat::default();
        assert_eq!(format, BatchOutputFormat::Text);
    }

    #[test]
    fn test_batch_result_creation_and_serialization() {
        let result = BatchResult {
            index: 0,
            prompt: "Hello".to_string(),
            response: Some("Hi there".to_string()),
            error: None,
            duration_ms: 150,
        };

        assert_eq!(result.index, 0);
        assert_eq!(result.prompt, "Hello");
        assert_eq!(result.response.as_deref(), Some("Hi there"));
        assert!(result.error.is_none());
        assert_eq!(result.duration_ms, 150);

        let json = serde_json::to_string(&result).expect("Serialization failed");
        assert!(json.contains("\"index\":0"));
        assert!(json.contains("\"prompt\":\"Hello\""));
        assert!(json.contains("\"response\":\"Hi there\""));
        assert!(json.contains("\"error\":null"));
        assert!(json.contains("\"duration_ms\":150"));
    }

    #[test]
    fn test_batch_result_deserialization() {
        let json =
            r#"{"index":1,"prompt":"test","response":"answer","error":null,"duration_ms":42}"#;
        let result: BatchResult = serde_json::from_str(json).expect("Deserialization failed");
        assert_eq!(result.index, 1);
        assert_eq!(result.prompt, "test");
        assert_eq!(result.response.as_deref(), Some("answer"));
        assert!(result.error.is_none());
        assert_eq!(result.duration_ms, 42);
    }

    #[test]
    fn test_load_prompts_plain_text() {
        let path = create_temp_file(
            "test_batch_plain.txt",
            "What is Rust?\nExplain async/await\nWhat is ownership?\n",
        );
        let prompts = load_prompts(&path).expect("Should load prompts");
        assert_eq!(prompts.len(), 3);
        assert_eq!(prompts[0], "What is Rust?");
        assert_eq!(prompts[1], "Explain async/await");
        assert_eq!(prompts[2], "What is ownership?");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prompts_skips_empty_lines_and_comments() {
        let path = create_temp_file(
            "test_batch_skip.txt",
            "# This is a comment\n\nFirst prompt\n   \n# Another comment\nSecond prompt\n\n",
        );
        let prompts = load_prompts(&path).expect("Should load prompts");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0], "First prompt");
        assert_eq!(prompts[1], "Second prompt");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prompts_json_bare_strings() {
        let path = create_temp_file(
            "test_batch_json_strings.jsonl",
            "\"Hello world\"\n\"How are you?\"\n\"What is 2+2?\"\n",
        );
        let prompts = load_prompts(&path).expect("Should load prompts");
        assert_eq!(prompts.len(), 3);
        assert_eq!(prompts[0], "Hello world");
        assert_eq!(prompts[1], "How are you?");
        assert_eq!(prompts[2], "What is 2+2?");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prompts_json_objects() {
        let path = create_temp_file(
            "test_batch_json_objects.jsonl",
            "{\"prompt\": \"Explain monads\"}\n{\"prompt\": \"What is a functor?\"}\n",
        );
        let prompts = load_prompts(&path).expect("Should load prompts");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0], "Explain monads");
        assert_eq!(prompts[1], "What is a functor?");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prompts_error_on_empty_file() {
        let path = create_temp_file("test_batch_empty.txt", "");
        let result = load_prompts(&path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no valid prompts"),
            "Expected 'no valid prompts' error, got: {}",
            err
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prompts_error_on_only_comments() {
        let path = create_temp_file(
            "test_batch_comments_only.txt",
            "# comment 1\n# comment 2\n   \n\n",
        );
        let result = load_prompts(&path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no valid prompts"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prompts_error_on_nonexistent_file() {
        let path = Path::new("/tmp/zeptoclaw_nonexistent_batch_file_xyz.txt");
        let result = load_prompts(path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found") || err.contains("Not found"),
            "Expected 'not found' error, got: {}",
            err
        );
    }

    #[test]
    fn test_format_results_text_format() {
        let results = vec![
            BatchResult {
                index: 0,
                prompt: "What is Rust?".to_string(),
                response: Some("A systems programming language.".to_string()),
                error: None,
                duration_ms: 100,
            },
            BatchResult {
                index: 1,
                prompt: "What is Cargo?".to_string(),
                response: Some("The Rust package manager.".to_string()),
                error: None,
                duration_ms: 80,
            },
        ];

        let output = format_results(&results, &BatchOutputFormat::Text);
        assert!(output.contains("--- Prompt 0 ---"));
        assert!(output.contains("What is Rust?"));
        assert!(output.contains("--- Response ---"));
        assert!(output.contains("A systems programming language."));
        assert!(output.contains("--- Prompt 1 ---"));
        assert!(output.contains("What is Cargo?"));
        assert!(output.contains("The Rust package manager."));
    }

    #[test]
    fn test_format_results_jsonl_format() {
        let results = vec![BatchResult {
            index: 0,
            prompt: "Hello".to_string(),
            response: Some("Hi".to_string()),
            error: None,
            duration_ms: 50,
        }];

        let output = format_results(&results, &BatchOutputFormat::Jsonl);
        let parsed: BatchResult =
            serde_json::from_str(&output).expect("Output should be valid JSON");
        assert_eq!(parsed.index, 0);
        assert_eq!(parsed.prompt, "Hello");
        assert_eq!(parsed.response.as_deref(), Some("Hi"));
        assert!(parsed.error.is_none());
    }

    #[test]
    fn test_format_results_with_errors() {
        let results = vec![BatchResult {
            index: 0,
            prompt: "Bad prompt".to_string(),
            response: None,
            error: Some("Provider timeout".to_string()),
            duration_ms: 30000,
        }];

        // Text format
        let text_output = format_results(&results, &BatchOutputFormat::Text);
        assert!(text_output.contains("--- Error ---"));
        assert!(text_output.contains("Provider timeout"));
        assert!(!text_output.contains("--- Response ---"));

        // JSONL format
        let jsonl_output = format_results(&results, &BatchOutputFormat::Jsonl);
        let parsed: BatchResult =
            serde_json::from_str(&jsonl_output).expect("Should be valid JSON");
        assert!(parsed.response.is_none());
        assert_eq!(parsed.error.as_deref(), Some("Provider timeout"));
    }

    #[test]
    fn test_batch_config_serde_roundtrip() {
        let config = BatchConfig {
            concurrency: 4,
            output_format: BatchOutputFormat::Jsonl,
            stop_on_error: true,
        };

        let json = serde_json::to_string(&config).expect("Serialization failed");
        let deserialized: BatchConfig =
            serde_json::from_str(&json).expect("Deserialization failed");

        assert_eq!(deserialized.concurrency, 4);
        assert_eq!(deserialized.output_format, BatchOutputFormat::Jsonl);
        assert!(deserialized.stop_on_error);
    }

    #[test]
    fn test_batch_config_serde_default_fills_missing_fields() {
        let json = r#"{}"#;
        let config: BatchConfig =
            serde_json::from_str(json).expect("Should deserialize with defaults");
        assert_eq!(config.concurrency, 1);
        assert_eq!(config.output_format, BatchOutputFormat::Text);
        assert!(!config.stop_on_error);
    }

    #[test]
    fn test_load_prompts_trims_whitespace() {
        let path = create_temp_file(
            "test_batch_trim.txt",
            "  spaced prompt  \n\ttabbed prompt\t\n",
        );
        let prompts = load_prompts(&path).expect("Should load prompts");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0], "spaced prompt");
        assert_eq!(prompts[1], "tabbed prompt");
        let _ = std::fs::remove_file(&path);
    }
}
