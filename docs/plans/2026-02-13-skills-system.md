# Skills System Implementation Plan

> **For Claude:** Use this plan to implement the skills system task-by-task.

**Goal:** Add a skills system to ZeptoClaw that allows extending agent capabilities through markdown-based skill files, similar to NanoBot's approach.

**Architecture:** Skills are `SKILL.md` files with YAML frontmatter (metadata) and markdown body (instructions). The agent loads skills into context based on triggers or explicit requests. Workspace skills override builtin skills.

**Tech Stack:** Rust, serde_yaml, regex, existing agent infrastructure

---

## What Are Skills?

Skills are **prompt extensions** that teach the agent how to use specific tools or perform tasks:

```
skills/
├── github/
│   └── SKILL.md       # How to use gh CLI
├── weather/
│   └── SKILL.md       # Get weather via curl
├── summarize/
│   └── SKILL.md       # Summarize URLs/videos
└── shopee/
    └── SKILL.md       # Malaysian e-commerce (custom)
```

**Example SKILL.md:**
```markdown
---
name: weather
description: Get current weather and forecasts (no API key required).
metadata: {"zeptoclaw":{"emoji":"🌤️","requires":{"bins":["curl"]}}}
---

# Weather

Two free services, no API keys needed.

## wttr.in (primary)

Quick one-liner:
```bash
curl -s "wttr.in/London?format=3"
```
```

---

## Task 1: Add Skills Config

**Files:**
- Modify: `src/config/types.rs`

**Add skills configuration:**

```rust
/// Skills system configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsConfig {
    /// Enable/disable skills system
    #[serde(default = "default_skills_enabled")]
    pub enabled: bool,

    /// Path to workspace skills directory (default: ~/.zeptoclaw/skills/)
    #[serde(default)]
    pub workspace_dir: Option<String>,

    /// Skills to always load into context
    #[serde(default)]
    pub always_load: Vec<String>,

    /// Disable specific builtin skills
    #[serde(default)]
    pub disabled: Vec<String>,
}

fn default_skills_enabled() -> bool {
    true
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            workspace_dir: None,
            always_load: Vec::new(),
            disabled: Vec::new(),
        }
    }
}
```

**Add to main Config struct:**

```rust
/// Skills system for extending agent capabilities
#[serde(default)]
pub skills: SkillsConfig,
```

---

## Task 2: Create Skills Module Structure

**Files:**
- Create: `src/skills/mod.rs`
- Create: `src/skills/loader.rs`
- Create: `src/skills/types.rs`

**src/skills/mod.rs:**

```rust
//! Skills system - extend agent capabilities through markdown-based skill files.
//!
//! Skills are `SKILL.md` files that teach the agent how to use specific tools
//! or perform certain tasks. They consist of:
//! - YAML frontmatter (name, description, requirements)
//! - Markdown body (instructions for the agent)

mod loader;
mod types;

pub use loader::SkillsLoader;
pub use types::{Skill, SkillMetadata, SkillRequirements};
```

**src/skills/types.rs:**

```rust
//! Skill type definitions.

use serde::{Deserialize, Serialize};

/// A loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill name (directory name)
    pub name: String,
    /// Skill description from frontmatter
    pub description: String,
    /// Full path to SKILL.md
    pub path: String,
    /// Source: "builtin" or "workspace"
    pub source: String,
    /// Parsed metadata
    pub metadata: SkillMetadata,
    /// Raw content (without frontmatter)
    pub content: String,
}

/// Skill metadata from YAML frontmatter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill name
    #[serde(default)]
    pub name: String,
    /// Description
    #[serde(default)]
    pub description: String,
    /// Homepage URL
    #[serde(default)]
    pub homepage: Option<String>,
    /// ZeptoClaw-specific metadata (JSON string)
    #[serde(default)]
    pub metadata: Option<String>,
}

/// Parsed ZeptoClaw metadata from the metadata field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ZeptoMetadata {
    /// Emoji for display
    #[serde(default)]
    pub emoji: Option<String>,
    /// Requirements
    #[serde(default)]
    pub requires: SkillRequirements,
    /// Install instructions
    #[serde(default)]
    pub install: Vec<InstallOption>,
    /// Always load this skill
    #[serde(default)]
    pub always: bool,
}

/// Skill requirements (binaries, env vars).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRequirements {
    /// Required binaries in PATH
    #[serde(default)]
    pub bins: Vec<String>,
    /// Required environment variables
    #[serde(default)]
    pub env: Vec<String>,
}

/// Install option for missing requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOption {
    /// Unique ID
    pub id: String,
    /// Install method: "brew", "apt", "cargo", etc.
    pub kind: String,
    /// Package/formula name
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    /// Binaries provided
    #[serde(default)]
    pub bins: Vec<String>,
    /// Human-readable label
    pub label: String,
}
```

---

## Task 3: Implement Skills Loader

**Files:**
- Create: `src/skills/loader.rs`

**Implementation:**

```rust
//! Skills loader - discovers and loads skills from filesystem.

use std::path::{Path, PathBuf};
use std::collections::HashMap;

use regex::Regex;
use tracing::{debug, info, warn};

use crate::error::{PicoError, Result};
use super::types::{Skill, SkillMetadata, ZeptoMetadata};

/// Default builtin skills directory (relative to binary or embedded)
const BUILTIN_SKILLS_DIR: &str = "skills";

/// Skills loader.
pub struct SkillsLoader {
    workspace_dir: PathBuf,
    builtin_dir: PathBuf,
    cache: HashMap<String, Skill>,
}

impl SkillsLoader {
    /// Create a new skills loader.
    pub fn new(workspace_dir: PathBuf, builtin_dir: Option<PathBuf>) -> Self {
        let builtin = builtin_dir.unwrap_or_else(|| {
            // Try relative to executable, then current dir
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join(BUILTIN_SKILLS_DIR)))
                .unwrap_or_else(|| PathBuf::from(BUILTIN_SKILLS_DIR))
        });

        Self {
            workspace_dir,
            builtin_dir: builtin,
            cache: HashMap::new(),
        }
    }

    /// Create with default paths.
    pub fn with_defaults() -> Self {
        let workspace = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zeptoclaw")
            .join("skills");

        Self::new(workspace, None)
    }

    /// List all available skills.
    pub fn list_skills(&self, filter_unavailable: bool) -> Vec<SkillInfo> {
        let mut skills = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Workspace skills (highest priority)
        if self.workspace_dir.exists() {
            for entry in self.workspace_dir.read_dir().into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    let skill_file = entry.path().join("SKILL.md");
                    if skill_file.exists() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !seen.contains(&name) {
                            seen.insert(name.clone());
                            skills.push(SkillInfo {
                                name,
                                path: skill_file.to_string_lossy().to_string(),
                                source: "workspace".to_string(),
                            });
                        }
                    }
                }
            }
        }

        // Builtin skills
        if self.builtin_dir.exists() {
            for entry in self.builtin_dir.read_dir().into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    let skill_file = entry.path().join("SKILL.md");
                    if skill_file.exists() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !seen.contains(&name) {
                            seen.insert(name.clone());
                            skills.push(SkillInfo {
                                name,
                                path: skill_file.to_string_lossy().to_string(),
                                source: "builtin".to_string(),
                            });
                        }
                    }
                }
            }
        }

        // Filter by requirements
        if filter_unavailable {
            skills.retain(|s| {
                self.load_skill(&s.name)
                    .map(|skill| self.check_requirements(&skill))
                    .unwrap_or(false)
            });
        }

        skills
    }

    /// Load a skill by name.
    pub fn load_skill(&self, name: &str) -> Option<Skill> {
        // Check cache
        if let Some(skill) = self.cache.get(name) {
            return Some(skill.clone());
        }

        // Check workspace first
        let workspace_skill = self.workspace_dir.join(name).join("SKILL.md");
        if workspace_skill.exists() {
            return self.parse_skill_file(&workspace_skill, name, "workspace");
        }

        // Check builtin
        let builtin_skill = self.builtin_dir.join(name).join("SKILL.md");
        if builtin_skill.exists() {
            return self.parse_skill_file(&builtin_skill, name, "builtin");
        }

        None
    }

    /// Load multiple skills for context injection.
    pub fn load_skills_for_context(&self, names: &[String]) -> String {
        let mut parts = Vec::new();

        for name in names {
            if let Some(skill) = self.load_skill(name) {
                let emoji = self.get_emoji(&skill).unwrap_or("📚".to_string());
                parts.push(format!(
                    "### {} {} Skill\n\n{}",
                    emoji, skill.name, skill.content
                ));
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            parts.join("\n\n---\n\n")
        }
    }

    /// Build XML summary of all skills for agent context.
    pub fn build_skills_summary(&self) -> String {
        let skills = self.list_skills(false);
        if skills.is_empty() {
            return String::new();
        }

        let mut lines = vec!["<skills>".to_string()];

        for info in skills {
            if let Some(skill) = self.load_skill(&info.name) {
                let available = self.check_requirements(&skill);
                let emoji = self.get_emoji(&skill).unwrap_or_default();
                let desc = html_escape(&skill.description);

                lines.push(format!("  <skill available=\"{}\">", available));
                lines.push(format!("    <name>{}{}</name>", emoji, html_escape(&skill.name)));
                lines.push(format!("    <description>{}</description>", desc));
                lines.push(format!("    <location>{}</location>", info.path));

                if !available {
                    if let Some(missing) = self.get_missing_requirements(&skill) {
                        lines.push(format!("    <requires>{}</requires>", html_escape(&missing)));
                    }
                }

                lines.push("  </skill>".to_string());
            }
        }

        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    /// Get skills marked as "always load".
    pub fn get_always_skills(&self) -> Vec<String> {
        self.list_skills(true)
            .into_iter()
            .filter(|info| {
                self.load_skill(&info.name)
                    .map(|skill| self.get_zeptometa(&skill).always)
                    .unwrap_or(false)
            })
            .map(|info| info.name)
            .collect()
    }

    /// Check if skill requirements are met.
    pub fn check_requirements(&self, skill: &Skill) -> bool {
        let meta = self.get_zeptometa(skill);

        // Check binaries
        for bin in &meta.requires.bins {
            if which::which(bin).is_err() {
                return false;
            }
        }

        // Check env vars
        for env in &meta.requires.env {
            if std::env::var(env).is_err() {
                return false;
            }
        }

        true
    }

    /// Get missing requirements description.
    fn get_missing_requirements(&self, skill: &Skill) -> Option<String> {
        let meta = self.get_zeptometa(skill);
        let mut missing = Vec::new();

        for bin in &meta.requires.bins {
            if which::which(bin).is_err() {
                missing.push(format!("CLI: {}", bin));
            }
        }

        for env in &meta.requires.env {
            if std::env::var(env).is_err() {
                missing.push(format!("ENV: {}", env));
            }
        }

        if missing.is_empty() {
            None
        } else {
            Some(missing.join(", "))
        }
    }

    /// Parse a SKILL.md file.
    fn parse_skill_file(&self, path: &Path, name: &str, source: &str) -> Option<Skill> {
        let content = std::fs::read_to_string(path).ok()?;
        let (metadata, body) = self.parse_frontmatter(&content);

        Some(Skill {
            name: name.to_string(),
            description: metadata.description.clone(),
            path: path.to_string_lossy().to_string(),
            source: source.to_string(),
            metadata,
            content: body,
        })
    }

    /// Parse YAML frontmatter from markdown.
    fn parse_frontmatter(&self, content: &str) -> (SkillMetadata, String) {
        let re = Regex::new(r"^---\n([\s\S]*?)\n---\n?").unwrap();

        if let Some(captures) = re.captures(content) {
            let yaml = captures.get(1).map(|m| m.as_str()).unwrap_or("");
            let body = content[captures.get(0).unwrap().end()..].trim().to_string();

            // Simple YAML parsing
            let mut metadata = SkillMetadata::default();
            for line in yaml.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim();
                    let value = value.trim().trim_matches('"').trim_matches('\'');

                    match key {
                        "name" => metadata.name = value.to_string(),
                        "description" => metadata.description = value.to_string(),
                        "homepage" => metadata.homepage = Some(value.to_string()),
                        "metadata" => metadata.metadata = Some(value.to_string()),
                        _ => {}
                    }
                }
            }

            (metadata, body)
        } else {
            (SkillMetadata::default(), content.to_string())
        }
    }

    /// Get ZeptoClaw-specific metadata.
    fn get_zeptometa(&self, skill: &Skill) -> ZeptoMetadata {
        skill.metadata.metadata.as_ref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("zeptoclaw").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Get emoji from metadata.
    fn get_emoji(&self, skill: &Skill) -> Option<String> {
        self.get_zeptometa(skill).emoji
    }
}

/// Skill info for listing.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub path: String,
    pub source: String,
}

/// HTML escape for XML output.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let loader = SkillsLoader::with_defaults();
        let content = r#"---
name: test
description: A test skill
metadata: {"zeptoclaw":{"emoji":"🧪"}}
---

# Test Skill

Instructions here.
"#;

        let (meta, body) = loader.parse_frontmatter(content);
        assert_eq!(meta.name, "test");
        assert_eq!(meta.description, "A test skill");
        assert!(body.contains("# Test Skill"));
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let loader = SkillsLoader::with_defaults();
        let content = "# Just content\n\nNo frontmatter here.";

        let (meta, body) = loader.parse_frontmatter(content);
        assert!(meta.name.is_empty());
        assert_eq!(body, content);
    }
}
```

---

## Task 4: Create Builtin Skills

**Files:**
- Create: `skills/github/SKILL.md`
- Create: `skills/weather/SKILL.md`
- Create: `skills/shopee/SKILL.md`

**skills/github/SKILL.md:**

```markdown
---
name: github
description: Interact with GitHub using the gh CLI for issues, PRs, and CI runs.
metadata: {"zeptoclaw":{"emoji":"🐙","requires":{"bins":["gh"]}}}
---

# GitHub Skill

Use the `gh` CLI to interact with GitHub.

## Pull Requests

Check CI status:
```bash
gh pr checks 55 --repo owner/repo
```

List workflow runs:
```bash
gh run list --repo owner/repo --limit 10
```

View failed logs:
```bash
gh run view <run-id> --repo owner/repo --log-failed
```

## Issues

List issues:
```bash
gh issue list --repo owner/repo --json number,title
```

Create issue:
```bash
gh issue create --repo owner/repo --title "Bug" --body "Description"
```
```

**skills/weather/SKILL.md:**

```markdown
---
name: weather
description: Get current weather and forecasts (no API key required).
metadata: {"zeptoclaw":{"emoji":"🌤️","requires":{"bins":["curl"]}}}
---

# Weather

No API keys needed.

## wttr.in

Quick weather:
```bash
curl -s "wttr.in/Kuala+Lumpur?format=3"
# Output: Kuala Lumpur: ☀️ +32°C
```

Detailed:
```bash
curl -s "wttr.in/Kuala+Lumpur?format=%l:+%c+%t+%h"
```

Tips:
- URL-encode spaces: `Kuala+Lumpur`
- Metric: `?m` · Today only: `?1`
```

**skills/shopee/SKILL.md:**

```markdown
---
name: shopee
description: Malaysian e-commerce helper for Shopee sellers (order management, inventory).
metadata: {"zeptoclaw":{"emoji":"🛒","requires":{}}}
---

# Shopee Seller Skill

Helpers for Malaysian Shopee sellers.

## Order Status Messages (Malay)

Copy-paste templates for WhatsApp:

**Order Received:**
```
Terima kasih atas pesanan anda! 🙏
Order #{ORDER_ID} telah diterima.
Kami akan proses dalam 24 jam.
```

**Order Shipped:**
```
Pesanan anda telah dihantar! 📦
Tracking: {TRACKING_NUMBER}
Courier: {COURIER}
Anggaran tiba: {ETA}
```

**Payment Reminder:**
```
Hi! Pesanan #{ORDER_ID} menunggu pembayaran.
Jumlah: RM{AMOUNT}
Sila buat bayaran untuk kami proses.
```

## Common Couriers

| Courier | Tracking URL |
|---------|--------------|
| J&T Express | https://www.jtexpress.my/track |
| Pos Laju | https://www.poslaju.com.my/track-trace |
| DHL eCommerce | https://www.dhl.com/my-en/home/tracking.html |
| Ninja Van | https://www.ninjavan.co/en-my/tracking |

## Inventory Checklist

Before restocking, check:
- [ ] Low stock items (< 5 units)
- [ ] Best sellers this week
- [ ] Items with pending orders
- [ ] Slow movers (no sales 30 days)
```

---

## Task 5: Export Skills Module

**Files:**
- Modify: `src/lib.rs`

**Add module declaration:**

```rust
pub mod skills;
```

**Add to re-exports:**

```rust
pub use skills::{SkillsLoader, Skill, SkillMetadata};
```

---

## Task 6: Integrate Skills into Agent Context

**Files:**
- Modify: `src/agent/context.rs`

**Add skills to agent context builder:**

```rust
use crate::skills::SkillsLoader;

impl AgentContext {
    /// Build system prompt with skills.
    pub fn build_system_prompt(&self, skills_loader: &SkillsLoader) -> String {
        let mut prompt = self.base_system_prompt.clone();

        // Add skills summary
        let skills_summary = skills_loader.build_skills_summary();
        if !skills_summary.is_empty() {
            prompt.push_str("\n\n## Available Skills\n\n");
            prompt.push_str("You can read full skill content from the location path when needed.\n\n");
            prompt.push_str(&skills_summary);
        }

        // Load always-on skills
        let always_skills = skills_loader.get_always_skills();
        if !always_skills.is_empty() {
            let skills_content = skills_loader.load_skills_for_context(&always_skills);
            prompt.push_str("\n\n## Active Skills\n\n");
            prompt.push_str(&skills_content);
        }

        prompt
    }
}
```

---

## Task 7: Add Skills CLI Commands

**Files:**
- Modify: `src/main.rs`

**Add skills subcommand:**

```rust
#[derive(Subcommand)]
enum Commands {
    // ... existing commands ...

    /// Manage skills
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
}

#[derive(Subcommand)]
enum SkillsAction {
    /// List available skills
    List {
        /// Show all skills including unavailable
        #[arg(short, long)]
        all: bool,
    },
    /// Show skill content
    Show {
        /// Skill name
        name: String,
    },
    /// Create a new skill
    Create {
        /// Skill name
        name: String,
    },
}
```

**Implement commands:**

```rust
async fn cmd_skills(config: &Config, action: SkillsAction) -> Result<()> {
    let workspace_dir = config.skills.workspace_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".zeptoclaw")
                .join("skills")
        });

    let loader = SkillsLoader::new(workspace_dir.clone(), None);

    match action {
        SkillsAction::List { all } => {
            let skills = loader.list_skills(!all);

            if skills.is_empty() {
                println!("No skills found.");
                println!("Create one with: zeptoclaw skills create <name>");
                return Ok(());
            }

            println!("Available Skills:\n");
            for skill in skills {
                let status = if loader.load_skill(&skill.name)
                    .map(|s| loader.check_requirements(&s))
                    .unwrap_or(false)
                {
                    "✓"
                } else {
                    "✗"
                };

                println!("  {} {} ({})", status, skill.name, skill.source);
            }
        }

        SkillsAction::Show { name } => {
            match loader.load_skill(&name) {
                Some(skill) => {
                    println!("=== {} ===\n", skill.name);
                    println!("Description: {}", skill.description);
                    println!("Source: {}", skill.source);
                    println!("Path: {}\n", skill.path);
                    println!("{}", skill.content);
                }
                None => {
                    eprintln!("Skill '{}' not found", name);
                }
            }
        }

        SkillsAction::Create { name } => {
            let skill_dir = workspace_dir.join(&name);
            let skill_file = skill_dir.join("SKILL.md");

            if skill_file.exists() {
                eprintln!("Skill '{}' already exists at {:?}", name, skill_file);
                return Ok(());
            }

            // Create directory
            std::fs::create_dir_all(&skill_dir)?;

            // Create template
            let template = format!(r#"---
name: {}
description: Describe what this skill does.
metadata: {{"zeptoclaw":{{"emoji":"📚","requires":{{}}}}}}
---

# {} Skill

Instructions for the agent go here.

## Usage

Add examples and commands.
"#, name, name);

            std::fs::write(&skill_file, template)?;
            println!("Created skill at {:?}", skill_file);
            println!("Edit the file to add your skill content.");
        }
    }

    Ok(())
}
```

---

## Task 8: Add Dependency for which crate

**Files:**
- Modify: `Cargo.toml`

**Add:**

```toml
# For checking if binaries exist in PATH
which = "6"
```

---

## Task 9: Add Tests

**Files:**
- Modify: `tests/integration.rs`

**Add tests:**

```rust
#[test]
fn test_config_skills() {
    let json = r#"{
        "skills": {
            "enabled": true,
            "always_load": ["github", "weather"]
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.skills.enabled);
    assert_eq!(config.skills.always_load, vec!["github", "weather"]);
}

#[test]
fn test_skills_loader_parse_frontmatter() {
    use zeptoclaw::skills::SkillsLoader;

    let loader = SkillsLoader::with_defaults();
    // Test would need actual skill files or mock
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

# Test CLI
cargo run -- skills list
cargo run -- skills list --all
cargo run -- skills create my-custom-skill
cargo run -- skills show weather
```

---

## Summary

| Component | Purpose |
|-----------|---------|
| `SkillsLoader` | Discovers and loads skills from filesystem |
| `Skill` | Parsed skill with metadata and content |
| `skills/` directory | Builtin skills shipped with ZeptoClaw |
| `~/.zeptoclaw/skills/` | User workspace skills (override builtin) |
| `zeptoclaw skills` | CLI for listing, showing, creating skills |

**Skill Format:**
```markdown
---
name: skill-name
description: What it does
metadata: {"zeptoclaw":{"emoji":"🔧","requires":{"bins":["curl"],"env":["API_KEY"]}}}
---

# Skill Name

Instructions for agent...
```

**Config:**
```json
{
  "skills": {
    "enabled": true,
    "always_load": ["github"],
    "disabled": ["weather"]
  }
}
```

---

## Future Enhancements

- **Skill triggers**: Auto-load skills based on message content
- **Skill marketplace**: Download skills from registry
- **Skill dependencies**: Skills that depend on other skills
- **Skill versioning**: Track skill versions

---

*Last updated: 2026-02-13*
