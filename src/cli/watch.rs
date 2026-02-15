//! Watch command — monitor URLs for changes and notify via channel.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Parse interval string like "1h", "30m", "15m", "60s" into seconds.
pub fn parse_interval(s: &str) -> Result<u64> {
    let s = s.trim().to_lowercase();
    if let Some(hours) = s.strip_suffix('h') {
        let n: u64 = hours.parse().with_context(|| "Invalid hours value")?;
        Ok(n * 3600)
    } else if let Some(mins) = s.strip_suffix('m') {
        let n: u64 = mins.parse().with_context(|| "Invalid minutes value")?;
        Ok(n * 60)
    } else if let Some(secs) = s.strip_suffix('s') {
        let n: u64 = secs.parse().with_context(|| "Invalid seconds value")?;
        Ok(n)
    } else {
        s.parse::<u64>()
            .with_context(|| "Invalid interval. Use formats like 1h, 30m, or 60s")
    }
}

/// Hash a URL to a filename-safe string.
fn url_hash(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}

/// Get path for storing last snapshot of a watched URL.
fn snapshot_path(url: &str) -> PathBuf {
    let hash = format!("{:x}", url_hash(url));
    zeptoclaw::config::Config::dir()
        .join("watch")
        .join(format!("{}.txt", hash))
}

pub(crate) async fn cmd_watch(url: String, interval: String, notify: String) -> Result<()> {
    let interval_secs = parse_interval(&interval)?;

    println!("Watching: {}", url);
    println!("Interval: {} ({}s)", interval, interval_secs);
    println!("Notify via: {}", notify);
    println!();
    println!("Press Ctrl+C to stop.");
    println!();

    // Create watch directory
    let watch_dir = zeptoclaw::config::Config::dir().join("watch");
    std::fs::create_dir_all(&watch_dir)
        .with_context(|| format!("Failed to create watch directory: {:?}", watch_dir))?;

    let snap_path = snapshot_path(&url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));

    loop {
        ticker.tick().await;

        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    eprintln!(
                        "[{}] HTTP {} for {}",
                        chrono::Local::now().format("%H:%M"),
                        status,
                        url
                    );
                    continue;
                }

                let body = resp.text().await.unwrap_or_default();
                let previous = std::fs::read_to_string(&snap_path).unwrap_or_default();

                if previous.is_empty() {
                    // First fetch — save baseline
                    std::fs::write(&snap_path, &body)?;
                    println!(
                        "[{}] Baseline saved ({} bytes)",
                        chrono::Local::now().format("%H:%M"),
                        body.len()
                    );
                } else if body != previous {
                    std::fs::write(&snap_path, &body)?;
                    println!(
                        "[{}] Change detected! (was {} bytes, now {} bytes)",
                        chrono::Local::now().format("%H:%M"),
                        previous.len(),
                        body.len()
                    );

                    // Notification message
                    let message = format!(
                        "URL changed: {}\nPrevious: {} bytes -> New: {} bytes",
                        url,
                        previous.len(),
                        body.len()
                    );
                    println!("  Notification ({}): {}", notify, message);

                    // TODO: Wire to actual channel send via ChannelManager
                    // For now, notifications are printed to stdout
                } else {
                    eprintln!("[{}] No change", chrono::Local::now().format("%H:%M"));
                }
            }
            Err(e) => {
                eprintln!(
                    "[{}] Fetch error: {}",
                    chrono::Local::now().format("%H:%M"),
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_interval_hours() {
        assert_eq!(parse_interval("1h").unwrap(), 3600);
        assert_eq!(parse_interval("2h").unwrap(), 7200);
    }

    #[test]
    fn test_parse_interval_minutes() {
        assert_eq!(parse_interval("30m").unwrap(), 1800);
        assert_eq!(parse_interval("15m").unwrap(), 900);
    }

    #[test]
    fn test_parse_interval_seconds() {
        assert_eq!(parse_interval("60s").unwrap(), 60);
        assert_eq!(parse_interval("120s").unwrap(), 120);
    }

    #[test]
    fn test_parse_interval_bare_number() {
        assert_eq!(parse_interval("3600").unwrap(), 3600);
    }

    #[test]
    fn test_parse_interval_invalid() {
        assert!(parse_interval("abc").is_err());
        assert!(parse_interval("").is_err());
    }

    #[test]
    fn test_url_hash_deterministic() {
        let h1 = url_hash("https://example.com");
        let h2 = url_hash("https://example.com");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_url_hash_different_urls() {
        let h1 = url_hash("https://example.com");
        let h2 = url_hash("https://other.com");
        assert_ne!(h1, h2);
    }
}
