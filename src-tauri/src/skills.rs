use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::paths;

/// Maximum number of skills to include in the system prompt.
const MAX_SKILLS_IN_PROMPT: usize = 150;
/// Maximum total characters for all skill descriptions in the prompt.
const MAX_SKILLS_PROMPT_CHARS: usize = 30_000;
/// Maximum size of a SKILL.md file (256 KB).
const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;

// ── Types ─────────────────────────────────────────────────────────

/// A parsed skill entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    /// Skill identifier (from frontmatter `name`).
    pub name: String,
    /// Human-readable description (from frontmatter `description`).
    pub description: String,
    /// Source category (e.g., "bundled", "managed", "project").
    pub source: String,
    /// Absolute path to the SKILL.md file.
    pub file_path: String,
    /// Directory containing the skill.
    pub base_dir: String,
}

/// Lightweight summary returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub source: String,
}

/// Full skill content for detailed view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub source: String,
    pub file_path: String,
    pub content: String,
}

// ── Frontmatter Parsing ──────────────────────────────────────────

/// Extract YAML frontmatter from a SKILL.md file content.
/// Returns (name, description, body) or None if parsing fails.
fn parse_frontmatter(content: &str) -> Option<(String, String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find the closing ---
    let after_opening = &trimmed[3..];
    let end_idx = after_opening.find("\n---")?;
    let yaml_block = &after_opening[..end_idx];
    let body = &after_opening[end_idx + 4..]; // skip \n---

    // Parse name and description from YAML manually
    // We avoid pulling in a full YAML parser by doing simple line-based extraction.
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in yaml_block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("name:") {
            name = Some(unquote(rest.trim()));
        } else if let Some(rest) = line.strip_prefix("description:") {
            description = Some(unquote(rest.trim()));
        }
    }

    let name = name.filter(|n| !n.is_empty())?;
    let description = description.unwrap_or_default();

    Some((name, description, body.to_string()))
}

/// Remove surrounding quotes from a YAML string value.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ── Discovery ────────────────────────────────────────────────────

/// Discover skills from a single directory.
/// Each immediate subdirectory containing a SKILL.md is treated as a skill.
fn load_skills_from_dir(dir: &Path, source: &str) -> Vec<SkillEntry> {
    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        // Check file size
        if let Ok(meta) = std::fs::metadata(&skill_md) {
            if meta.len() > MAX_SKILL_FILE_BYTES {
                log::warn!(
                    "Skipping oversized SKILL.md: {} ({} bytes)",
                    skill_md.display(),
                    meta.len()
                );
                continue;
            }
        }

        let content = match std::fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to read {}: {}", skill_md.display(), e);
                continue;
            }
        };

        if let Some((name, description, _body)) = parse_frontmatter(&content) {
            entries.push(SkillEntry {
                name,
                description,
                source: source.to_string(),
                file_path: skill_md.to_string_lossy().to_string(),
                base_dir: path.to_string_lossy().to_string(),
            });
        }
    }

    entries
}

/// Load all skills from all configured sources.
///
/// Sources (lowest → highest precedence):
/// 1. Bundled skills (shipped with the app, if any)
/// 2. Managed skills (~/.opencomputer/skills/)
/// 3. Project-specific skills (.opencomputer/skills/ in cwd)
pub fn load_all_skills() -> Vec<SkillEntry> {
    let mut all: Vec<SkillEntry> = Vec::new();
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Collect from all sources (lowest precedence first)
    let mut sources: Vec<(PathBuf, &str)> = Vec::new();

    // 1. Managed skills: ~/.opencomputer/skills/
    if let Ok(dir) = paths::skills_dir() {
        sources.push((dir, "managed"));
    }

    // 2. Project-specific skills: .opencomputer/skills/ relative to cwd
    if let Ok(cwd) = std::env::current_dir() {
        let project_skills = cwd.join(".opencomputer").join("skills");
        if project_skills.is_dir() {
            sources.push((project_skills, "project"));
        }
    }

    // Higher-precedence sources override lower ones
    for (dir, source) in &sources {
        let entries = load_skills_from_dir(dir, source);
        for entry in entries {
            seen_names.insert(entry.name.clone());
            // Remove any previous entry with the same name (lower precedence)
            all.retain(|e| e.name != entry.name);
            all.push(entry);
        }
    }

    // Sort alphabetically
    all.sort_by(|a, b| a.name.cmp(&b.name));

    let _ = seen_names; // suppress unused warning
    all
}

// ── Prompt Generation ────────────────────────────────────────────

/// Build the skills section of the system prompt.
/// Returns an empty string if no skills are available.
pub fn build_skills_prompt(skills: &[SkillEntry]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push("\n\nThe following skills are available. Each skill provides specialized capabilities you can leverage:".to_string());

    let mut total_chars = 0;
    let mut count = 0;

    for skill in skills {
        if count >= MAX_SKILLS_IN_PROMPT {
            break;
        }
        let entry = format!("- {}: {}", skill.name, skill.description);
        if total_chars + entry.len() > MAX_SKILLS_PROMPT_CHARS {
            break;
        }
        lines.push(entry.clone());
        total_chars += entry.len();
        count += 1;
    }

    if count < skills.len() {
        lines.push(format!(
            "\n({} more skills available but not shown due to space limits)",
            skills.len() - count
        ));
    }

    lines.push("\nWhen a user's request matches a skill's domain, use the relevant tools and knowledge described by that skill.".to_string());

    lines.join("\n")
}

/// Get the full content of a specific skill's SKILL.md.
pub fn get_skill_content(name: &str) -> Option<SkillDetail> {
    let skills = load_all_skills();
    let entry = skills.into_iter().find(|s| s.name == name)?;

    let content = std::fs::read_to_string(&entry.file_path).ok()?;
    // Strip frontmatter, return only the body
    let body = if let Some((_name, _desc, body)) = parse_frontmatter(&content) {
        body.trim().to_string()
    } else {
        content
    };

    Some(SkillDetail {
        name: entry.name,
        description: entry.description,
        source: entry.source,
        file_path: entry.file_path,
        content: body,
    })
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: github
description: "GitHub operations via gh CLI"
---

# GitHub Skill

Use the gh CLI.
"#;
        let (name, desc, body) = parse_frontmatter(content).unwrap();
        assert_eq!(name, "github");
        assert_eq!(desc, "GitHub operations via gh CLI");
        assert!(body.contains("# GitHub Skill"));
    }

    #[test]
    fn test_parse_frontmatter_unquoted() {
        let content = "---\nname: my-skill\ndescription: A simple skill\n---\nBody here";
        let (name, desc, _body) = parse_frontmatter(content).unwrap();
        assert_eq!(name, "my-skill");
        assert_eq!(desc, "A simple skill");
    }

    #[test]
    fn test_parse_frontmatter_missing_name() {
        let content = "---\ndescription: No name\n---\nBody";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just regular markdown";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_build_skills_prompt_empty() {
        assert_eq!(build_skills_prompt(&[]), "");
    }

    #[test]
    fn test_build_skills_prompt_one_skill() {
        let skills = vec![SkillEntry {
            name: "github".to_string(),
            description: "GitHub ops".to_string(),
            source: "managed".to_string(),
            file_path: "/tmp/github/SKILL.md".to_string(),
            base_dir: "/tmp/github".to_string(),
        }];
        let prompt = build_skills_prompt(&skills);
        assert!(prompt.contains("- github: GitHub ops"));
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'world'"), "world");
        assert_eq!(unquote("plain"), "plain");
    }
}
