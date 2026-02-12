//! Default HEARTBEAT.md template and bootstrap helper.

use std::path::Path;

/// Default heartbeat file content.
pub const HEARTBEAT_TEMPLATE: &str = r#"# Heartbeat Tasks

This file is checked periodically by your ZeptoClaw agent.
Add background tasks below. Keep tasks actionable and specific.

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

/// Ensure heartbeat file exists, creating it with default template if needed.
pub async fn ensure_heartbeat_file(path: &Path) -> std::io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(path, HEARTBEAT_TEMPLATE).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ensure_heartbeat_file_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("HEARTBEAT.md");

        let created = ensure_heartbeat_file(&path).await.unwrap();
        assert!(created);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("Heartbeat Tasks"));
    }

    #[tokio::test]
    async fn test_ensure_heartbeat_file_noop_when_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("HEARTBEAT.md");
        tokio::fs::write(&path, "custom").await.unwrap();

        let created = ensure_heartbeat_file(&path).await.unwrap();
        assert!(!created);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "custom");
    }
}
