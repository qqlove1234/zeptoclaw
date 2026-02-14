//! Long-term memory store for ZeptoClaw.
//!
//! Provides persistent key-value memory across sessions -- facts, preferences,
//! and learnings that the agent remembers between conversations. Stored as a
//! single JSON file at `~/.zeptoclaw/memory/longterm.json`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Result, ZeptoError};

/// Returns the current unix epoch timestamp in seconds.
fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A single memory entry with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique key (e.g., "user:name", "preference:language", "fact:project-name").
    pub key: String,
    /// The memory content.
    pub value: String,
    /// Category for grouping (e.g., "user", "preference", "fact", "learning").
    pub category: String,
    /// When this memory was created (unix timestamp).
    pub created_at: u64,
    /// When this memory was last accessed (unix timestamp).
    pub last_accessed: u64,
    /// Number of times this memory has been accessed.
    pub access_count: u64,
    /// Optional tags for search.
    pub tags: Vec<String>,
}

/// Long-term memory store persisted as JSON.
#[derive(Debug)]
pub struct LongTermMemory {
    entries: HashMap<String, MemoryEntry>,
    storage_path: PathBuf,
}

impl LongTermMemory {
    /// Create a new long-term memory store at the default path
    /// (`~/.zeptoclaw/memory/longterm.json`). Creates the file and parent
    /// directories if they do not exist.
    pub fn new() -> Result<Self> {
        let path = Config::dir().join("memory").join("longterm.json");
        Self::with_path(path)
    }

    /// Create a long-term memory store at a custom path. Useful for testing.
    pub fn with_path(path: PathBuf) -> Result<Self> {
        let entries = Self::load(&path)?;
        Ok(Self {
            entries,
            storage_path: path,
        })
    }

    /// Upsert a memory entry. If the key already exists, the value, category,
    /// and tags are updated and `last_accessed` is refreshed. The entry is
    /// persisted to disk immediately.
    pub fn set(&mut self, key: &str, value: &str, category: &str, tags: Vec<String>) -> Result<()> {
        let now = now_timestamp();

        if let Some(existing) = self.entries.get_mut(key) {
            existing.value = value.to_string();
            existing.category = category.to_string();
            existing.tags = tags;
            existing.last_accessed = now;
        } else {
            let entry = MemoryEntry {
                key: key.to_string(),
                value: value.to_string(),
                category: category.to_string(),
                created_at: now,
                last_accessed: now,
                access_count: 0,
                tags,
            };
            self.entries.insert(key.to_string(), entry);
        }

        self.save()
    }

    /// Retrieve a memory entry by key, updating its access stats
    /// (`last_accessed` and `access_count`). Does NOT auto-save; call
    /// `save()` periodically to persist access stat changes.
    pub fn get(&mut self, key: &str) -> Option<&MemoryEntry> {
        let now = now_timestamp();
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_accessed = now;
            entry.access_count += 1;
        }
        self.entries.get(key)
    }

    /// Retrieve a memory entry by key without updating access stats.
    pub fn get_readonly(&self, key: &str) -> Option<&MemoryEntry> {
        self.entries.get(key)
    }

    /// Delete a memory entry by key. Returns `true` if the entry existed
    /// (and was removed), `false` otherwise. Saves to disk on deletion.
    pub fn delete(&mut self, key: &str) -> Result<bool> {
        let existed = self.entries.remove(key).is_some();
        if existed {
            self.save()?;
        }
        Ok(existed)
    }

    /// Case-insensitive substring search across key, value, category, and tags.
    /// Results are sorted by relevance: exact key matches first, then by
    /// `access_count` descending.
    pub fn search(&self, query: &str) -> Vec<&MemoryEntry> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<&MemoryEntry> = self
            .entries
            .values()
            .filter(|entry| {
                entry.key.to_lowercase().contains(&query_lower)
                    || entry.value.to_lowercase().contains(&query_lower)
                    || entry.category.to_lowercase().contains(&query_lower)
                    || entry
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query_lower))
            })
            .collect();

        results.sort_by(|a, b| {
            let a_exact = a.key.to_lowercase() == query_lower;
            let b_exact = b.key.to_lowercase() == query_lower;
            match (a_exact, b_exact) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.access_count.cmp(&a.access_count),
            }
        });

        results
    }

    /// List all entries in a given category, sorted by `last_accessed`
    /// descending (most recently accessed first).
    pub fn list_by_category(&self, category: &str) -> Vec<&MemoryEntry> {
        let cat_lower = category.to_lowercase();
        let mut results: Vec<&MemoryEntry> = self
            .entries
            .values()
            .filter(|entry| entry.category.to_lowercase() == cat_lower)
            .collect();

        results.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));
        results
    }

    /// List all entries, sorted by `last_accessed` descending.
    pub fn list_all(&self) -> Vec<&MemoryEntry> {
        let mut results: Vec<&MemoryEntry> = self.entries.values().collect();
        results.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));
        results
    }

    /// Return the number of stored entries.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Return a sorted list of unique category names.
    pub fn categories(&self) -> Vec<String> {
        let mut cats: Vec<String> = self
            .entries
            .values()
            .map(|e| e.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        cats.sort();
        cats
    }

    /// Remove entries with the lowest `access_count` to keep at most
    /// `keep_count` entries. Returns the number of entries removed.
    pub fn cleanup_least_used(&mut self, keep_count: usize) -> Result<usize> {
        if self.entries.len() <= keep_count {
            return Ok(0);
        }

        let mut entries_vec: Vec<(String, u64)> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.access_count))
            .collect();

        // Sort by access_count ascending so that the least-used are first.
        entries_vec.sort_by(|a, b| a.1.cmp(&b.1));

        let to_remove = entries_vec.len() - keep_count;
        let keys_to_remove: Vec<String> = entries_vec
            .into_iter()
            .take(to_remove)
            .map(|(k, _)| k)
            .collect();

        for key in &keys_to_remove {
            self.entries.remove(key);
        }

        self.save()?;
        Ok(to_remove)
    }

    /// Return a human-readable summary of the memory store.
    pub fn summary(&self) -> String {
        let count = self.count();
        let cat_count = self.categories().len();
        format!(
            "Long-term memory: {} entries ({} categories)",
            count, cat_count
        )
    }

    /// Persist the current memory state to disk as pretty-printed JSON.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ZeptoError::Config(format!(
                    "Failed to create memory directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        let json = serde_json::to_string_pretty(&self.entries).map_err(|e| {
            ZeptoError::Config(format!("Failed to serialize long-term memory: {}", e))
        })?;

        std::fs::write(&self.storage_path, json).map_err(|e| {
            ZeptoError::Config(format!(
                "Failed to write long-term memory to {}: {}",
                self.storage_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Load memory entries from a JSON file on disk. Returns an empty map if
    /// the file does not exist.
    fn load(path: &PathBuf) -> Result<HashMap<String, MemoryEntry>> {
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let content = std::fs::read_to_string(path).map_err(|e| {
            ZeptoError::Config(format!(
                "Failed to read long-term memory from {}: {}",
                path.display(),
                e
            ))
        })?;

        if content.trim().is_empty() {
            return Ok(HashMap::new());
        }

        let entries: HashMap<String, MemoryEntry> =
            serde_json::from_str(&content).map_err(|e| {
                ZeptoError::Config(format!("Failed to parse long-term memory JSON: {}", e))
            })?;

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a LongTermMemory backed by a temp directory.
    fn temp_memory() -> (LongTermMemory, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path().join("longterm.json");
        let mem = LongTermMemory::with_path(path).expect("failed to create memory");
        (mem, dir)
    }

    #[test]
    fn test_memory_entry_creation() {
        let entry = MemoryEntry {
            key: "user:name".to_string(),
            value: "Alice".to_string(),
            category: "user".to_string(),
            created_at: 1000,
            last_accessed: 2000,
            access_count: 5,
            tags: vec!["identity".to_string()],
        };

        assert_eq!(entry.key, "user:name");
        assert_eq!(entry.value, "Alice");
        assert_eq!(entry.category, "user");
        assert_eq!(entry.created_at, 1000);
        assert_eq!(entry.last_accessed, 2000);
        assert_eq!(entry.access_count, 5);
        assert_eq!(entry.tags, vec!["identity"]);
    }

    #[test]
    fn test_longterm_memory_new_empty() {
        let (mem, _dir) = temp_memory();
        assert_eq!(mem.count(), 0);
    }

    #[test]
    fn test_set_and_get() {
        let (mut mem, _dir) = temp_memory();
        mem.set("user:name", "Alice", "user", vec!["identity".to_string()])
            .unwrap();

        let entry = mem.get("user:name").unwrap();
        assert_eq!(entry.value, "Alice");
        assert_eq!(entry.category, "user");
    }

    #[test]
    fn test_set_upsert() {
        let (mut mem, _dir) = temp_memory();
        mem.set("user:name", "Alice", "user", vec![]).unwrap();
        mem.set("user:name", "Bob", "user", vec!["updated".to_string()])
            .unwrap();

        let entry = mem.get("user:name").unwrap();
        assert_eq!(entry.value, "Bob");
        assert_eq!(entry.tags, vec!["updated"]);
        // Should still be 1 entry, not 2.
        assert_eq!(mem.count(), 1);
    }

    #[test]
    fn test_get_updates_access_stats() {
        let (mut mem, _dir) = temp_memory();
        mem.set("key1", "value1", "test", vec![]).unwrap();

        let before_access = mem.get_readonly("key1").unwrap().last_accessed;
        let before_count = mem.get_readonly("key1").unwrap().access_count;

        // Small delay to ensure timestamp may differ (though on fast machines
        // it may be the same second).
        let _ = mem.get("key1");
        let _ = mem.get("key1");

        let entry = mem.get_readonly("key1").unwrap();
        assert_eq!(entry.access_count, before_count + 2);
        assert!(entry.last_accessed >= before_access);
    }

    #[test]
    fn test_get_readonly_no_update() {
        let (mut mem, _dir) = temp_memory();
        mem.set("key1", "value1", "test", vec![]).unwrap();

        let before = mem.get_readonly("key1").unwrap().access_count;
        let _ = mem.get_readonly("key1");
        let _ = mem.get_readonly("key1");
        let after = mem.get_readonly("key1").unwrap().access_count;

        assert_eq!(before, after);
    }

    #[test]
    fn test_get_nonexistent() {
        let (mut mem, _dir) = temp_memory();
        assert!(mem.get("nonexistent").is_none());
    }

    #[test]
    fn test_delete_existing() {
        let (mut mem, _dir) = temp_memory();
        mem.set("key1", "value1", "test", vec![]).unwrap();
        assert_eq!(mem.count(), 1);

        let existed = mem.delete("key1").unwrap();
        assert!(existed);
        assert_eq!(mem.count(), 0);
        assert!(mem.get("key1").is_none());
    }

    #[test]
    fn test_delete_nonexistent() {
        let (mut mem, _dir) = temp_memory();
        let existed = mem.delete("nonexistent").unwrap();
        assert!(!existed);
    }

    #[test]
    fn test_search_by_key() {
        let (mut mem, _dir) = temp_memory();
        mem.set("user:name", "Alice", "user", vec![]).unwrap();
        mem.set("project:name", "ZeptoClaw", "project", vec![])
            .unwrap();

        let results = mem.search("user");
        assert!(!results.is_empty());
        assert!(results.iter().any(|e| e.key == "user:name"));
    }

    #[test]
    fn test_search_by_value() {
        let (mut mem, _dir) = temp_memory();
        mem.set("key1", "Rust programming language", "fact", vec![])
            .unwrap();
        mem.set("key2", "Python scripting", "fact", vec![]).unwrap();

        let results = mem.search("Rust");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "key1");
    }

    #[test]
    fn test_search_by_tag() {
        let (mut mem, _dir) = temp_memory();
        mem.set(
            "key1",
            "some value",
            "test",
            vec!["important".to_string(), "work".to_string()],
        )
        .unwrap();
        mem.set("key2", "other value", "test", vec!["personal".to_string()])
            .unwrap();

        let results = mem.search("important");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "key1");
    }

    #[test]
    fn test_search_case_insensitive() {
        let (mut mem, _dir) = temp_memory();
        mem.set("Key1", "Hello World", "Test", vec!["MyTag".to_string()])
            .unwrap();

        // Search with different casing.
        assert!(!mem.search("hello").is_empty());
        assert!(!mem.search("HELLO").is_empty());
        assert!(!mem.search("key1").is_empty());
        assert!(!mem.search("KEY1").is_empty());
        assert!(!mem.search("mytag").is_empty());
        assert!(!mem.search("test").is_empty());
    }

    #[test]
    fn test_list_by_category() {
        let (mut mem, _dir) = temp_memory();
        mem.set("k1", "v1", "user", vec![]).unwrap();
        mem.set("k2", "v2", "user", vec![]).unwrap();
        mem.set("k3", "v3", "project", vec![]).unwrap();

        let user_entries = mem.list_by_category("user");
        assert_eq!(user_entries.len(), 2);
        assert!(user_entries.iter().all(|e| e.category == "user"));

        let project_entries = mem.list_by_category("project");
        assert_eq!(project_entries.len(), 1);
    }

    #[test]
    fn test_list_all() {
        let (mut mem, _dir) = temp_memory();
        mem.set("k1", "v1", "a", vec![]).unwrap();
        mem.set("k2", "v2", "b", vec![]).unwrap();
        mem.set("k3", "v3", "c", vec![]).unwrap();

        let all = mem.list_all();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_count() {
        let (mut mem, _dir) = temp_memory();
        assert_eq!(mem.count(), 0);

        mem.set("k1", "v1", "test", vec![]).unwrap();
        assert_eq!(mem.count(), 1);

        mem.set("k2", "v2", "test", vec![]).unwrap();
        assert_eq!(mem.count(), 2);

        mem.delete("k1").unwrap();
        assert_eq!(mem.count(), 1);
    }

    #[test]
    fn test_categories() {
        let (mut mem, _dir) = temp_memory();
        mem.set("k1", "v1", "user", vec![]).unwrap();
        mem.set("k2", "v2", "fact", vec![]).unwrap();
        mem.set("k3", "v3", "user", vec![]).unwrap();
        mem.set("k4", "v4", "preference", vec![]).unwrap();

        let cats = mem.categories();
        assert_eq!(cats, vec!["fact", "preference", "user"]);
    }

    #[test]
    fn test_cleanup_least_used() {
        let (mut mem, _dir) = temp_memory();
        mem.set("k1", "v1", "test", vec![]).unwrap();
        mem.set("k2", "v2", "test", vec![]).unwrap();
        mem.set("k3", "v3", "test", vec![]).unwrap();

        // Access k3 several times so it has the highest access_count.
        let _ = mem.get("k3");
        let _ = mem.get("k3");
        let _ = mem.get("k3");

        // Access k1 once.
        let _ = mem.get("k1");

        // k2 has 0 accesses, k1 has 1, k3 has 3.
        // Keeping 1 should remove k2 and k1.
        let removed = mem.cleanup_least_used(1).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(mem.count(), 1);
        assert!(mem.get_readonly("k3").is_some());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path().join("longterm.json");

        // Create and populate a store.
        {
            let mut mem = LongTermMemory::with_path(path.clone()).unwrap();
            mem.set("user:name", "Alice", "user", vec!["identity".to_string()])
                .unwrap();
            mem.set("fact:lang", "Rust", "fact", vec!["tech".to_string()])
                .unwrap();
        }

        // Open a new store at the same path and verify entries loaded.
        {
            let mem = LongTermMemory::with_path(path).unwrap();
            assert_eq!(mem.count(), 2);
            let entry = mem.get_readonly("user:name").unwrap();
            assert_eq!(entry.value, "Alice");
            assert_eq!(entry.tags, vec!["identity"]);

            let entry2 = mem.get_readonly("fact:lang").unwrap();
            assert_eq!(entry2.value, "Rust");
        }
    }

    #[test]
    fn test_summary() {
        let (mut mem, _dir) = temp_memory();
        assert_eq!(mem.summary(), "Long-term memory: 0 entries (0 categories)");

        mem.set("k1", "v1", "user", vec![]).unwrap();
        mem.set("k2", "v2", "fact", vec![]).unwrap();
        mem.set("k3", "v3", "fact", vec![]).unwrap();

        assert_eq!(mem.summary(), "Long-term memory: 3 entries (2 categories)");
    }
}
