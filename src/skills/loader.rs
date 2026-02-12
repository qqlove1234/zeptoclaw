//! Skills loader and parser.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;

use super::types::{Skill, SkillInfo, SkillMetadata, ZeptoMetadata};

const BUILTIN_SKILLS_DIR: &str = "skills";

/// Discover and load markdown skills from workspace and builtin directories.
pub struct SkillsLoader {
    workspace_dir: PathBuf,
    builtin_dir: PathBuf,
}

impl SkillsLoader {
    /// Create loader with explicit directories.
    pub fn new(workspace_dir: PathBuf, builtin_dir: Option<PathBuf>) -> Self {
        let builtin = builtin_dir.unwrap_or_else(default_builtin_skills_dir);
        Self {
            workspace_dir,
            builtin_dir: builtin,
        }
    }

    /// Create loader with default directories.
    pub fn with_defaults() -> Self {
        let workspace = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zeptoclaw")
            .join("skills");
        Self::new(workspace, None)
    }

    /// Workspace skill directory.
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    /// Builtin skill directory.
    pub fn builtin_dir(&self) -> &Path {
        &self.builtin_dir
    }

    /// List known skills (`workspace` overrides `builtin` by name).
    pub fn list_skills(&self, filter_unavailable: bool) -> Vec<SkillInfo> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();

        collect_skill_infos(&self.workspace_dir, "workspace", &mut out, &mut seen);
        collect_skill_infos(&self.builtin_dir, "builtin", &mut out, &mut seen);

        if filter_unavailable {
            out.retain(|info| {
                self.load_skill(&info.name)
                    .map(|skill| self.check_requirements(&skill))
                    .unwrap_or(false)
            });
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Load one skill by name.
    pub fn load_skill(&self, name: &str) -> Option<Skill> {
        let workspace = self.workspace_dir.join(name).join("SKILL.md");
        if workspace.is_file() {
            return self.parse_skill_file(&workspace, name, "workspace");
        }

        let builtin = self.builtin_dir.join(name).join("SKILL.md");
        if builtin.is_file() {
            return self.parse_skill_file(&builtin, name, "builtin");
        }

        None
    }

    /// Build summary XML block for prompt context.
    pub fn build_skills_summary(&self) -> String {
        let skills = self.list_skills(false);
        if skills.is_empty() {
            return String::new();
        }

        let mut lines = vec!["<skills>".to_string()];
        for info in skills {
            if let Some(skill) = self.load_skill(&info.name) {
                let available = self.check_requirements(&skill);
                let emoji = self.get_zeptometa(&skill).emoji.unwrap_or_default();
                let desc = escape_xml(&skill.description);
                lines.push(format!("  <skill available=\"{}\">", available));
                lines.push(format!(
                    "    <name>{}{}</name>",
                    emoji,
                    escape_xml(&skill.name)
                ));
                lines.push(format!("    <description>{}</description>", desc));
                lines.push(format!(
                    "    <location>{}</location>",
                    escape_xml(&skill.path)
                ));
                lines.push("  </skill>".to_string());
            }
        }
        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    /// Load full content for a set of named skills.
    pub fn load_skills_for_context(&self, names: &[String]) -> String {
        let mut parts = Vec::new();
        for name in names {
            if let Some(skill) = self.load_skill(name) {
                let emoji = self
                    .get_zeptometa(&skill)
                    .emoji
                    .unwrap_or_else(|| "📚".to_string());
                parts.push(format!(
                    "### {} {} Skill\n\n{}",
                    emoji, skill.name, skill.content
                ));
            }
        }

        parts.join("\n\n---\n\n")
    }

    /// Return names of skills marked `always = true`.
    pub fn get_always_skills(&self) -> Vec<String> {
        self.list_skills(false)
            .into_iter()
            .filter_map(|info| self.load_skill(&info.name))
            .filter(|skill| self.get_zeptometa(skill).always)
            .map(|skill| skill.name)
            .collect()
    }

    /// Check if required binaries and env vars are present.
    pub fn check_requirements(&self, skill: &Skill) -> bool {
        let meta = self.get_zeptometa(skill);
        for bin in &meta.requires.bins {
            if !binary_in_path(bin) {
                return false;
            }
        }
        for env_name in &meta.requires.env {
            if std::env::var(env_name).is_err() {
                return false;
            }
        }
        true
    }

    fn parse_skill_file(&self, path: &Path, fallback_name: &str, source: &str) -> Option<Skill> {
        let raw = std::fs::read_to_string(path).ok()?;
        let (metadata, body) = self.parse_frontmatter(&raw);

        let name = if metadata.name.trim().is_empty() {
            fallback_name.to_string()
        } else {
            metadata.name.clone()
        };
        let description = if metadata.description.trim().is_empty() {
            format!("Skill '{}'", name)
        } else {
            metadata.description.clone()
        };

        Some(Skill {
            name,
            description,
            path: path.to_string_lossy().to_string(),
            source: source.to_string(),
            metadata,
            content: body,
        })
    }

    fn parse_frontmatter(&self, content: &str) -> (SkillMetadata, String) {
        let re = Regex::new(r"(?s)^---\n(.*?)\n---\n?").ok();
        if let Some(re) = re {
            if let Some(captures) = re.captures(content) {
                if let (Some(frontmatter), Some(full)) = (captures.get(1), captures.get(0)) {
                    let metadata = parse_frontmatter_metadata(frontmatter.as_str());
                    let body = content[full.end()..].trim().to_string();
                    return (metadata, body);
                }
            }
        }

        (SkillMetadata::default(), content.to_string())
    }

    fn get_zeptometa(&self, skill: &Skill) -> ZeptoMetadata {
        skill
            .metadata
            .metadata
            .as_ref()
            .and_then(|value| {
                if let Some(scoped) = value.get("zeptoclaw") {
                    serde_json::from_value(scoped.clone()).ok()
                } else {
                    serde_json::from_value(value.clone()).ok()
                }
            })
            .unwrap_or_default()
    }
}

fn default_builtin_skills_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|p| p.join(BUILTIN_SKILLS_DIR)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(BUILTIN_SKILLS_DIR))
}

fn collect_skill_infos(
    dir: &Path,
    source: &str,
    output: &mut Vec<SkillInfo>,
    seen: &mut HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if seen.contains(&name) {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }

        seen.insert(name.clone());
        output.push(SkillInfo {
            name,
            path: skill_file.to_string_lossy().to_string(),
            source: source.to_string(),
        });
    }
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn parse_frontmatter_metadata(frontmatter: &str) -> SkillMetadata {
    let mut metadata = SkillMetadata::default();

    for raw_line in frontmatter.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "name" => metadata.name = unquote(value),
                "description" => metadata.description = unquote(value),
                "homepage" => {
                    let parsed = unquote(value);
                    if !parsed.is_empty() {
                        metadata.homepage = Some(parsed);
                    }
                }
                "metadata" => {
                    let parsed = unquote(value);
                    if !parsed.is_empty() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&parsed) {
                            metadata.metadata = Some(json);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    metadata
}

fn unquote(input: &str) -> String {
    input
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn binary_in_path(bin: &str) -> bool {
    if bin.trim().is_empty() {
        return false;
    }
    let path = match std::env::var_os("PATH") {
        Some(path) => path,
        None => return false,
    };

    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            let candidate = dir.join(format!("{}.exe", bin));
            if candidate.is_file() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let loader = SkillsLoader::with_defaults();
        let content = r#"---
name: weather
description: Weather helper
metadata: {"zeptoclaw":{"emoji":"🌤️","requires":{"bins":["curl"]}}}
---
# Weather

Use wttr.in.
"#;

        let (meta, body) = loader.parse_frontmatter(content);
        assert_eq!(meta.name, "weather");
        assert_eq!(meta.description, "Weather helper");
        assert!(body.contains("# Weather"));
    }

    #[test]
    fn test_parse_frontmatter_without_frontmatter() {
        let loader = SkillsLoader::with_defaults();
        let content = "# Just markdown";
        let (meta, body) = loader.parse_frontmatter(content);
        assert!(meta.name.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn test_workspace_overrides_builtin() {
        let temp = tempfile::tempdir().unwrap();
        let ws = temp.path().join("workspace");
        let builtin = temp.path().join("builtin");
        std::fs::create_dir_all(ws.join("demo")).unwrap();
        std::fs::create_dir_all(builtin.join("demo")).unwrap();
        std::fs::write(
            ws.join("demo/SKILL.md"),
            "---\nname: demo\ndescription: workspace\n---\nworkspace",
        )
        .unwrap();
        std::fs::write(
            builtin.join("demo/SKILL.md"),
            "---\nname: demo\ndescription: builtin\n---\nbuiltin",
        )
        .unwrap();

        let loader = SkillsLoader::new(ws, Some(builtin));
        let skill = loader.load_skill("demo").unwrap();
        assert_eq!(skill.source, "workspace");
        assert_eq!(skill.description, "workspace");
    }
}
