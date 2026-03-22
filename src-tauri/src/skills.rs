use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::paths;

/// Maximum number of skills to include in the system prompt.
const MAX_SKILLS_IN_PROMPT: usize = 150;
/// Maximum total characters for all skill descriptions in the prompt.
const MAX_SKILLS_PROMPT_CHARS: usize = 30_000;
/// Maximum size of a SKILL.md file (256 KB).
const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;

// ── Types ─────────────────────────────────────────────────────────

/// Environment requirements parsed from SKILL.md frontmatter `requires:` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRequires {
    /// Binaries that must exist in PATH (all required).
    pub bins: Vec<String>,
    /// Environment variables that must be set (all required).
    pub env: Vec<String>,
    /// OS identifiers the skill supports, e.g. ["darwin", "linux"].
    /// Empty means all OSes are supported.
    pub os: Vec<String>,
}

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
    /// Environment requirements from frontmatter `requires:` block.
    #[serde(default)]
    pub requires: SkillRequires,
}

/// Lightweight summary returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub source: String,
    pub base_dir: String,
    pub enabled: bool,
    /// Environment variable names required by this skill (from `requires.env`).
    #[serde(default)]
    pub requires_env: Vec<String>,
}

/// File metadata inside a skill directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Whether this is a directory.
    pub is_dir: bool,
}

/// Full skill content for detailed view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub source: String,
    pub file_path: String,
    pub base_dir: String,
    pub content: String,
    pub enabled: bool,
    /// All files/dirs inside the skill directory.
    pub files: Vec<FileInfo>,
    /// Environment requirements from frontmatter.
    #[serde(default)]
    pub requires: SkillRequires,
}

// ── Frontmatter Parsing ──────────────────────────────────────────

/// Extract YAML frontmatter from a SKILL.md file content.
/// Returns (name, description, requires, body) or None if parsing fails.
fn parse_frontmatter(content: &str) -> Option<(String, String, SkillRequires, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find the closing ---
    let after_opening = &trimmed[3..];
    let end_idx = after_opening.find("\n---")?;
    let yaml_block = &after_opening[..end_idx];
    let body = &after_opening[end_idx + 4..]; // skip \n---

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let requires = parse_requires(yaml_block);

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

    Some((name, description, requires, body.to_string()))
}

/// Parse the `requires:` block from a YAML frontmatter string.
/// Supports both inline arrays `[a, b]` and list style `- item`.
fn parse_requires(yaml_block: &str) -> SkillRequires {
    let mut req = SkillRequires::default();
    let mut in_requires = false;
    let mut current_key = String::new();

    for line in yaml_block.lines() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if indent == 0 {
            // Root-level key
            in_requires = trimmed == "requires:" || trimmed.starts_with("requires:");
            current_key.clear();
            continue;
        }

        if !in_requires {
            continue;
        }

        if indent >= 2 && indent < 4 {
            // Sub-key of requires (e.g., "bins:", "env:", "os:")
            if let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim();
                let val = val.trim();
                current_key = key.to_string();
                if !val.is_empty() {
                    // Inline array: bins: [git, gh]
                    let items = parse_yaml_inline_list(val);
                    push_requires_items(&mut req, key, items);
                }
            }
        } else if indent >= 4 {
            // List item: - git
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = unquote(item.trim()).to_string();
                if !item.is_empty() {
                    push_requires_items(&mut req, &current_key, vec![item]);
                }
            }
        }
    }

    req
}

/// Parse a YAML inline list like `[git, gh]` or `["git", "gh"]`.
fn parse_yaml_inline_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        inner
            .split(',')
            .map(|item| unquote(item.trim()).to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    }
}

fn push_requires_items(req: &mut SkillRequires, key: &str, items: Vec<String>) {
    match key {
        "bins" => req.bins.extend(items),
        "env" => req.env.extend(items),
        "os" => req.os.extend(items),
        _ => {}
    }
}

/// Check whether a skill's requirements are satisfied in the current environment.
/// `configured_env` provides user-configured env var overrides from the settings UI.
pub fn check_requirements(req: &SkillRequires, configured_env: Option<&HashMap<String, String>>) -> bool {
    // Check OS constraint
    if !req.os.is_empty() {
        let current = std::env::consts::OS; // "macos", "linux", "windows"
        let ok = req.os.iter().any(|os| {
            let os = os.as_str();
            os == current
                || (os == "darwin" && current == "macos")
                || (os == "mac" && current == "macos")
        });
        if !ok {
            return false;
        }
    }

    // Check binaries in PATH
    for bin in &req.bins {
        if !binary_in_path(bin) {
            return false;
        }
    }

    // Check environment variables: user-configured values take priority over system env
    for key in &req.env {
        let has_configured = configured_env
            .and_then(|m| m.get(key))
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if !has_configured && std::env::var(key).map(|v| v.is_empty()).unwrap_or(true) {
            return false;
        }
    }

    true
}

/// Mask a secret value for frontend display.
/// Same pattern as ProviderConfig::masked().
pub fn mask_value(v: &str) -> String {
    if v.len() > 8 {
        format!("{}...{}", &v[..4], &v[v.len() - 4..])
    } else if !v.is_empty() {
        "****".to_string()
    } else {
        String::new()
    }
}

/// Check if a value is a masked placeholder (should not overwrite real value).
pub fn is_masked_value(v: &str) -> bool {
    v == "****" || (v.len() > 7 && v.contains("..."))
}

/// Check whether a binary exists anywhere in PATH.
fn binary_in_path(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return true;
            }
            // Windows: also check .exe
            #[cfg(target_os = "windows")]
            {
                let exe = dir.join(format!("{}.exe", name));
                if exe.is_file() {
                    return true;
                }
            }
        }
    }
    false
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
                app_warn!("skills", "loader",
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
                app_warn!("skills", "loader", "Failed to read {}: {}", skill_md.display(), e);
                continue;
            }
        };

        if let Some((name, description, requires, _body)) = parse_frontmatter(&content) {
            entries.push(SkillEntry {
                name,
                description,
                source: source.to_string(),
                file_path: skill_md.to_string_lossy().to_string(),
                base_dir: path.to_string_lossy().to_string(),
                requires,
            });
        }
    }

    entries
}

/// Load all skills from all configured sources.
///
/// Sources (lowest → highest precedence):
/// 1. Extra directories (user-imported, lowest)
/// 2. Managed skills (~/.opencomputer/skills/)
/// 3. Project-specific skills (.opencomputer/skills/ in cwd, highest)
pub fn load_all_skills_with_extra(extra_dirs: &[String]) -> Vec<SkillEntry> {
    let mut all: Vec<SkillEntry> = Vec::new();

    // Collect from all sources (lowest precedence first)
    let mut sources: Vec<(PathBuf, String)> = Vec::new();

    // 1. Extra directories (user-imported)
    for dir in extra_dirs {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            // Use last path component as label
            let label = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dir.clone());
            sources.push((path, label));
        }
    }

    // 2. Managed skills: ~/.opencomputer/skills/
    if let Ok(dir) = paths::skills_dir() {
        sources.push((dir, "managed".to_string()));
    }

    // 3. Project-specific skills: .opencomputer/skills/ relative to cwd
    if let Ok(cwd) = std::env::current_dir() {
        let project_skills = cwd.join(".opencomputer").join("skills");
        if project_skills.is_dir() {
            sources.push((project_skills, "project".to_string()));
        }
    }

    // Higher-precedence sources override lower ones
    for (dir, source) in &sources {
        let entries = load_skills_from_dir(dir, source);
        for entry in entries {
            // Remove any previous entry with the same name (lower precedence)
            all.retain(|e| e.name != entry.name);
            all.push(entry);
        }
    }

    // Sort alphabetically
    all.sort_by(|a, b| a.name.cmp(&b.name));

    all
}

/// Convenience wrapper: load all skills without extra dirs.
#[allow(dead_code)]
pub fn load_all_skills() -> Vec<SkillEntry> {
    load_all_skills_with_extra(&[])
}

// ── Prompt Generation ────────────────────────────────────────────

/// Build the skills section of the system prompt.
/// Disabled skills are excluded.
/// When `env_check` is true, skills whose `requires` conditions are not met are also excluded.
/// `skill_env` provides user-configured env vars per skill.
/// Returns an empty string if no skills are available.
pub fn build_skills_prompt(
    skills: &[SkillEntry],
    disabled: &[String],
    env_check: bool,
    skill_env: &HashMap<String, HashMap<String, String>>,
) -> String {
    let active: Vec<&SkillEntry> = skills
        .iter()
        .filter(|s| !disabled.contains(&s.name))
        .filter(|s| !env_check || check_requirements(&s.requires, skill_env.get(&s.name)))
        .collect();
    if active.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push("\n\nThe following skills are available. Each skill provides specialized capabilities you can leverage:".to_string());

    let mut total_chars = 0;
    let mut count = 0;

    for skill in &active {
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

    if count < active.len() {
        lines.push(format!(
            "\n({} more skills available but not shown due to space limits)",
            active.len() - count
        ));
    }

    lines.push("\nWhen a user's request matches a skill's domain, use the relevant tools and knowledge described by that skill.".to_string());

    lines.join("\n")
}

/// Scan a skill directory for all files/subdirectories.
fn scan_skill_files(base_dir: &str) -> Vec<FileInfo> {
    let mut files = Vec::new();
    let dir = Path::new(base_dir);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.path().is_dir();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(FileInfo { name, size, is_dir });
        }
    }
    // Sort: directories first, then alphabetically
    files.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
    });
    files
}

/// Get the full content of a specific skill's SKILL.md.
pub fn get_skill_content(name: &str, extra_dirs: &[String], disabled: &[String]) -> Option<SkillDetail> {
    let skills = load_all_skills_with_extra(extra_dirs);
    let entry = skills.into_iter().find(|s| s.name == name)?;

    let content = std::fs::read_to_string(&entry.file_path).ok()?;

    let files = scan_skill_files(&entry.base_dir);
    let enabled = !disabled.contains(&entry.name);

    Some(SkillDetail {
        name: entry.name,
        description: entry.description,
        source: entry.source,
        file_path: entry.file_path,
        base_dir: entry.base_dir,
        content,
        enabled,
        files,
        requires: entry.requires,
    })
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, desc: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            description: desc.to_string(),
            source: "managed".to_string(),
            file_path: format!("/tmp/{}/SKILL.md", name),
            base_dir: format!("/tmp/{}", name),
            requires: SkillRequires::default(),
        }
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: github
description: "GitHub operations via gh CLI"
---

# GitHub Skill

Use the gh CLI.
"#;
        let (name, desc, _req, body) = parse_frontmatter(content).unwrap();
        assert_eq!(name, "github");
        assert_eq!(desc, "GitHub operations via gh CLI");
        assert!(body.contains("# GitHub Skill"));
    }

    #[test]
    fn test_parse_frontmatter_unquoted() {
        let content = "---\nname: my-skill\ndescription: A simple skill\n---\nBody here";
        let (name, desc, _req, _body) = parse_frontmatter(content).unwrap();
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
    fn test_parse_requires_inline() {
        let yaml = "name: git\ndescription: d\nrequires:\n  bins: [git, gh]\n  env: [GITHUB_TOKEN]\n  os: [darwin, linux]\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git", "gh"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
        assert_eq!(req.os, vec!["darwin", "linux"]);
    }

    #[test]
    fn test_parse_requires_list_style() {
        let yaml = "name: git\ndescription: d\nrequires:\n  bins:\n    - git\n    - gh\n  env:\n    - GITHUB_TOKEN\n";
        let req = parse_requires(yaml);
        assert_eq!(req.bins, vec!["git", "gh"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
    }

    #[test]
    fn test_build_skills_prompt_empty() {
        assert_eq!(build_skills_prompt(&[], &[], false, &HashMap::new()), "");
    }

    #[test]
    fn test_build_skills_prompt_one_skill() {
        let skills = vec![make_skill("github", "GitHub ops")];
        let prompt = build_skills_prompt(&skills, &[], false, &HashMap::new());
        assert!(prompt.contains("- github: GitHub ops"));
    }

    #[test]
    fn test_build_skills_prompt_disabled() {
        let skills = vec![make_skill("github", "GitHub ops")];
        let prompt = build_skills_prompt(&skills, &["github".to_string()], false, &HashMap::new());
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_build_skills_prompt_env_check_no_requires() {
        // Skill with no requires should always pass env_check
        let skills = vec![make_skill("basic", "A basic skill")];
        let prompt = build_skills_prompt(&skills, &[], true, &HashMap::new());
        assert!(prompt.contains("- basic: A basic skill"));
    }

    #[test]
    fn test_check_requirements_empty() {
        // Empty requirements always pass
        assert!(check_requirements(&SkillRequires::default(), None));
    }

    #[test]
    fn test_check_requirements_wrong_os() {
        let req = SkillRequires {
            bins: vec![],
            env: vec![],
            os: vec!["nonexistent-os-xyz".to_string()],
        };
        assert!(!check_requirements(&req, None));
    }

    #[test]
    fn test_check_requirements_with_configured_env() {
        let req = SkillRequires {
            bins: vec![],
            env: vec!["MY_TEST_KEY_XYZ".to_string()],
            os: vec![],
        };
        // Without configured env, should fail (assuming MY_TEST_KEY_XYZ is not set)
        assert!(!check_requirements(&req, None));
        // With configured env, should pass
        let mut configured = HashMap::new();
        configured.insert("MY_TEST_KEY_XYZ".to_string(), "some-value".to_string());
        assert!(check_requirements(&req, Some(&configured)));
        // Empty value should still fail
        configured.insert("MY_TEST_KEY_XYZ".to_string(), String::new());
        assert!(!check_requirements(&req, Some(&configured)));
    }

    #[test]
    fn test_mask_value() {
        assert_eq!(mask_value(""), "");
        assert_eq!(mask_value("short"), "****");
        assert_eq!(mask_value("12345678"), "****");
        assert_eq!(mask_value("123456789"), "1234...6789");
        assert_eq!(mask_value("sk-abcdefghijklmnop"), "sk-a...mnop");
    }

    #[test]
    fn test_is_masked_value() {
        assert!(is_masked_value("****"));
        assert!(is_masked_value("1234...6789"));
        assert!(!is_masked_value("real-value"));
        assert!(!is_masked_value(""));
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'world'"), "world");
        assert_eq!(unquote("plain"), "plain");
    }
}
