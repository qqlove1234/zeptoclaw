//! Path validation utilities for secure file operations
//!
//! This module provides path validation to prevent directory traversal attacks
//! and ensure all file operations stay within the designated workspace.

use std::path::{Component, Path, PathBuf};

use crate::error::{PicoError, Result};

/// A validated path that is guaranteed to be within the workspace.
///
/// This struct can only be created through `validate_path_in_workspace`,
/// ensuring that any `SafePath` instance represents a path that has been
/// verified to be within the allowed workspace directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafePath {
    path: PathBuf,
}

impl SafePath {
    /// Returns a reference to the underlying path.
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Converts the SafePath into a PathBuf.
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

impl AsRef<Path> for SafePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// Validates that a path is within the specified workspace directory.
///
/// This function performs the following checks:
/// 1. Resolves the target path (joins with workspace if relative)
/// 2. Normalizes the path to remove `.` and `..` components
/// 3. Verifies the normalized path starts with the canonical workspace path
///
/// # Arguments
///
/// * `path` - The path to validate (can be relative or absolute)
/// * `workspace` - The workspace directory that the path must be within
///
/// # Returns
///
/// * `Ok(SafePath)` - If the path is valid and within the workspace
/// * `Err(PicoError::SecurityViolation)` - If the path escapes the workspace
///
/// # Examples
///
/// ```
/// use zeptoclaw::security::validate_path_in_workspace;
///
/// // Relative path within workspace
/// let result = validate_path_in_workspace("src/main.rs", "/workspace");
/// assert!(result.is_ok());
///
/// // Path traversal attempt
/// let result = validate_path_in_workspace("../../../etc/passwd", "/workspace");
/// assert!(result.is_err());
/// ```
pub fn validate_path_in_workspace(path: &str, workspace: &str) -> Result<SafePath> {
    // Check for obvious traversal patterns in the raw input
    if contains_traversal_pattern(path) {
        return Err(PicoError::SecurityViolation(format!(
            "Path contains suspicious traversal pattern: {}",
            path
        )));
    }

    let workspace_path = Path::new(workspace);
    let target_path = Path::new(path);

    // Resolve the target path - join with workspace if relative
    let resolved_path = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        workspace_path.join(target_path)
    };

    // Normalize the path to resolve . and .. components
    let normalized_path = normalize_path(&resolved_path);

    // Get the canonical workspace path for comparison
    // If workspace doesn't exist, use the normalized workspace path
    let canonical_workspace = workspace_path
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(workspace_path));

    // Check if the normalized path starts with the workspace
    if !normalized_path.starts_with(&canonical_workspace) {
        return Err(PicoError::SecurityViolation(format!(
            "Path escapes workspace: {} is not within {}",
            path, workspace
        )));
    }

    Ok(SafePath {
        path: normalized_path,
    })
}

/// Normalizes a path by resolving `.` and `..` components.
///
/// This function processes path components to remove:
/// - `.` (current directory) components
/// - `..` (parent directory) components by popping from the normalized path
///
/// If the resulting path exists on the filesystem, it returns the canonical path.
/// Otherwise, it returns the normalized path.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                // Pop the last component if possible
                normalized.pop();
            }
            Component::CurDir => {
                // Skip current directory components
            }
            _ => {
                // Push all other components (Normal, RootDir, Prefix)
                normalized.push(component);
            }
        }
    }

    // Try to canonicalize if the path exists
    normalized.canonicalize().unwrap_or(normalized)
}

/// Checks if a path string contains common traversal patterns.
///
/// This provides an early detection of obvious traversal attempts
/// before more expensive path normalization.
fn contains_traversal_pattern(path: &str) -> bool {
    // Check for common traversal patterns
    let patterns = [
        "..",         // Parent directory
        "%2e%2e",     // URL encoded ..
        "%252e%252e", // Double URL encoded ..
        "..%2f",      // Mixed encoding
        "%2f..",      // Mixed encoding
        "..\\",       // Windows style
        "\\..\\",     // Windows style with prefix
    ];

    let lower_path = path.to_lowercase();
    patterns.iter().any(|p| lower_path.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_valid_relative_path() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        // Create a subdirectory
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "fn main() {}").unwrap();

        let result = validate_path_in_workspace("src/main.rs", workspace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_absolute_path_in_workspace() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        // Create a file
        std::fs::write(temp.path().join("file.txt"), "content").unwrap();

        let absolute_path = temp.path().join("file.txt");
        let result = validate_path_in_workspace(absolute_path.to_str().unwrap(), workspace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_traversal_with_double_dots() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        let result = validate_path_in_workspace("../../../etc/passwd", workspace);
        assert!(result.is_err());

        if let Err(PicoError::SecurityViolation(msg)) = result {
            assert!(msg.contains("traversal pattern") || msg.contains("escapes workspace"));
        } else {
            panic!("Expected SecurityViolation error");
        }
    }

    #[test]
    fn test_traversal_with_encoded_dots() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        let result = validate_path_in_workspace("%2e%2e/etc/passwd", workspace);
        assert!(result.is_err());
    }

    #[test]
    fn test_traversal_with_mixed_encoding() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        let result = validate_path_in_workspace("..%2f../etc/passwd", workspace);
        assert!(result.is_err());
    }

    #[test]
    fn test_absolute_path_outside_workspace() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        let result = validate_path_in_workspace("/etc/passwd", workspace);
        assert!(result.is_err());

        if let Err(PicoError::SecurityViolation(msg)) = result {
            assert!(msg.contains("escapes workspace"));
        } else {
            panic!("Expected SecurityViolation error");
        }
    }

    #[test]
    fn test_nested_traversal() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        // Create nested directory
        std::fs::create_dir_all(temp.path().join("a/b/c")).unwrap();

        let result = validate_path_in_workspace("a/b/c/../../../../etc/passwd", workspace);
        assert!(result.is_err());
    }

    #[test]
    fn test_current_directory_reference() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        // Create a file
        std::fs::write(temp.path().join("file.txt"), "content").unwrap();

        // ./file.txt should be valid
        let result = validate_path_in_workspace("./file.txt", workspace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_complex_valid_path() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        // Create nested structure
        std::fs::create_dir_all(temp.path().join("src/lib")).unwrap();
        std::fs::write(temp.path().join("src/lib/mod.rs"), "// module").unwrap();

        // This path has . but stays within workspace
        let result = validate_path_in_workspace("src/./lib/mod.rs", workspace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_safe_path_conversion() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        std::fs::write(temp.path().join("test.txt"), "content").unwrap();

        let safe_path = validate_path_in_workspace("test.txt", workspace).unwrap();

        // Test as_path
        assert!(safe_path.as_path().ends_with("test.txt"));

        // Test into_path_buf
        let path_buf = safe_path.clone().into_path_buf();
        assert!(path_buf.ends_with("test.txt"));

        // Test AsRef<Path>
        let path_ref: &Path = safe_path.as_ref();
        assert!(path_ref.ends_with("test.txt"));
    }

    #[test]
    fn test_windows_style_traversal() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        let result = validate_path_in_workspace("..\\..\\etc\\passwd", workspace);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_path() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().to_str().unwrap();

        // Empty path should resolve to workspace itself, which is valid
        let result = validate_path_in_workspace("", workspace);
        assert!(result.is_ok());
    }

    #[test]
    fn test_normalize_path_basic() {
        let path = Path::new("/a/b/../c/./d");
        let normalized = normalize_path(path);

        // Should normalize to /a/c/d
        let components: Vec<_> = normalized.components().collect();
        assert!(components
            .iter()
            .any(|c| matches!(c, Component::Normal(s) if s.to_str() == Some("a"))));
        assert!(components
            .iter()
            .any(|c| matches!(c, Component::Normal(s) if s.to_str() == Some("c"))));
        assert!(components
            .iter()
            .any(|c| matches!(c, Component::Normal(s) if s.to_str() == Some("d"))));
    }
}
