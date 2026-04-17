use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::paths;

use super::frontmatter::parse_frontmatter;
use super::types::*;

// ── Bundled Skills ──────────────────────────────────────────────

/// Cached bundled skills directory path, resolved once per process.
static BUNDLED_SKILLS_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Resolve the bundled skills directory shipped with the application.
///
/// Search order:
/// 1. `OPENCOMPUTER_BUNDLED_SKILLS_DIR` env override
/// 2. Sibling `skills/` next to the executable (release / packaged)
/// 3. Workspace root `skills/` via `CARGO_MANIFEST_DIR` (dev builds)
fn resolve_bundled_skills_dir() -> Option<PathBuf> {
    // 1. Env override
    if let Ok(dir) = std::env::var("OPENCOMPUTER_BUNDLED_SKILLS_DIR") {
        let p = PathBuf::from(dir.trim());
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. Sibling to executable: <exe_dir>/skills/
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if let Some(found) = try_skills_dir(exe_dir.join("skills")) {
                return Some(found);
            }
            // Also check one level up (e.g., macOS .app bundle: Contents/MacOS/../Resources/skills)
            if let Some(parent) = exe_dir.parent() {
                if let Some(found) = try_skills_dir(parent.join("skills"))
                    .or_else(|| try_skills_dir(parent.join("Resources").join("skills")))
                {
                    return Some(found);
                }
            }
        }
    }

    // 3. Dev builds only: workspace root skills/ (CARGO_MANIFEST_DIR is crates/oc-core)
    //    Use env!() compile-time macro so the path is baked in at build time,
    //    since the runtime env var is only present under `cargo run`.
    #[cfg(debug_assertions)]
    {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = Path::new(manifest_dir)
            .parent()
            .and_then(|p| p.parent());
        if let Some(root) = workspace_root {
            if let Some(found) = try_skills_dir(root.join("skills")) {
                return Some(found);
            }
        }
    }

    None
}

/// Return the path if it's a valid skills directory, None otherwise.
fn try_skills_dir(candidate: PathBuf) -> Option<PathBuf> {
    if candidate.is_dir() && looks_like_skills_dir(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

/// Quick check: does the directory contain at least one subdirectory with SKILL.md?
fn looks_like_skills_dir(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if is_dir && entry.path().join("SKILL.md").is_file() {
                return true;
            }
        }
    }
    false
}

/// Get the cached bundled skills directory.
pub fn bundled_skills_dir() -> Option<&'static PathBuf> {
    BUNDLED_SKILLS_DIR
        .get_or_init(resolve_bundled_skills_dir)
        .as_ref()
}

// ── Path Utilities ───────────────────────────────────────────────

/// Compact a file path by replacing the home directory prefix with `~`.
/// Retained after the prompt-catalog no longer exposes paths — still useful
/// for log messages and tests, so marked `allow(dead_code)` rather than
/// removed.
#[allow(dead_code)]
pub(super) fn compact_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        let home_ref = home_str.as_ref();
        if path.starts_with(home_ref) {
            let suffix = &path[home_ref.len()..];
            if suffix.starts_with('/') || suffix.starts_with('\\') {
                return format!("~{}", suffix);
            }
        }
    }
    path.to_string()
}

// ── Discovery ────────────────────────────────────────────────────

/// Discover skills from a single directory.
/// Each immediate subdirectory containing a SKILL.md is treated as a skill.
/// Also detects nested `skills/` subdirectories for recursive scan.
fn load_skills_from_dir(dir: &Path, source: &str, budget: &SkillPromptBudget) -> Vec<SkillEntry> {
    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    let mut candidate_count = 0;

    for entry in read_dir.flatten() {
        candidate_count += 1;
        if candidate_count > budget.max_candidates_per_root {
            app_warn!(
                "skills",
                "loader",
                "Reached max candidates limit ({}) for directory: {}",
                budget.max_candidates_per_root,
                dir.display()
            );
            break;
        }

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if skill_md.is_file() {
            // Direct skill directory
            if let Some(skill) = load_single_skill(&skill_md, &path, source, budget.max_file_bytes)
            {
                entries.push(skill);
            }
        } else {
            // Check for nested skills/ subdirectory
            let nested_skills = path.join("skills");
            if nested_skills.is_dir() {
                let nested = load_skills_from_dir(&nested_skills, source, budget);
                entries.extend(nested);
            }
        }
    }

    entries
}

/// Load a single skill from its SKILL.md file.
fn load_single_skill(
    skill_md: &Path,
    skill_dir: &Path,
    source: &str,
    max_file_bytes: u64,
) -> Option<SkillEntry> {
    // Check file size
    if let Ok(meta) = std::fs::metadata(skill_md) {
        if meta.len() > max_file_bytes {
            app_warn!(
                "skills",
                "loader",
                "Skipping oversized SKILL.md: {} ({} bytes)",
                skill_md.display(),
                meta.len()
            );
            return None;
        }
    }

    let content = match std::fs::read_to_string(skill_md) {
        Ok(c) => c,
        Err(e) => {
            app_warn!(
                "skills",
                "loader",
                "Failed to read {}: {}",
                skill_md.display(),
                e
            );
            return None;
        }
    };

    let parsed = parse_frontmatter(&content)?;

    Some(SkillEntry {
        name: parsed.name,
        description: parsed.description,
        source: source.to_string(),
        file_path: skill_md.to_string_lossy().to_string(),
        base_dir: skill_dir.to_string_lossy().to_string(),
        requires: parsed.requires,
        skill_key: parsed.skill_key,
        user_invocable: parsed.user_invocable,
        disable_model_invocation: parsed.disable_model_invocation,
        command_dispatch: parsed.command_dispatch,
        command_tool: parsed.command_tool,
        command_arg_mode: parsed.command_arg_mode,
        command_arg_placeholder: parsed.command_arg_placeholder,
        command_arg_options: parsed.command_arg_options,
        command_prompt_template: parsed.command_prompt_template,
        install: parsed.install,
        allowed_tools: parsed.allowed_tools,
        context_mode: parsed.context_mode,
        agent: parsed.agent,
        effort: parsed.effort,
        paths: parsed.paths,
        status: parsed.status,
        authored_by: parsed.authored_by,
        rationale: parsed.rationale,
    })
}

/// Load all skills from all configured sources.
///
/// Sources (lowest -> highest precedence):
/// 0. Bundled skills (shipped with the application, lowest)
/// 1. Extra directories (user-imported)
/// 2. Managed skills (~/.opencomputer/skills/)
/// 3. Project-specific skills (.opencomputer/skills/ in cwd, highest)
pub fn load_all_skills_with_extra(extra_dirs: &[String]) -> Vec<SkillEntry> {
    load_all_skills_with_budget(extra_dirs, &SkillPromptBudget::default())
}

/// Load all skills with configurable budget limits.
pub fn load_all_skills_with_budget(
    extra_dirs: &[String],
    budget: &SkillPromptBudget,
) -> Vec<SkillEntry> {
    let mut all: Vec<SkillEntry> = Vec::new();

    // Collect from all sources (lowest precedence first)
    let mut sources: Vec<(PathBuf, String)> = Vec::new();

    // 0. Bundled skills (shipped with the application)
    if let Some(dir) = bundled_skills_dir() {
        sources.push((dir.clone(), "bundled".to_string()));
    }

    // 1. Extra directories (user-imported)
    for dir in extra_dirs {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            // Use last path component as label
            let label = path
                .file_name()
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
        let entries = load_skills_from_dir(dir, source, budget);
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

/// Build slash command definitions from user-invocable skills.
/// Returns skill entries that should be registered as slash commands.
pub fn get_invocable_skills(extra_dirs: &[String], disabled: &[String]) -> Vec<SkillEntry> {
    let skills = load_all_skills_with_extra(extra_dirs);
    skills
        .into_iter()
        .filter(|s| !disabled.contains(&s.name))
        .filter(|s| s.user_invocable != Some(false))
        // Draft/Archived skills are excluded from slash command registration
        .filter(|s| s.status.is_discoverable())
        .collect()
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
    files.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    files
}

/// Get the full content of a specific skill's SKILL.md.
pub fn get_skill_content(
    name: &str,
    extra_dirs: &[String],
    disabled: &[String],
) -> Option<SkillDetail> {
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
        skill_key: entry.skill_key,
        user_invocable: entry.user_invocable,
        disable_model_invocation: entry.disable_model_invocation,
        command_dispatch: entry.command_dispatch,
        command_tool: entry.command_tool,
        install: entry.install,
        allowed_tools: entry.allowed_tools,
        context_mode: entry.context_mode,
        agent: entry.agent,
        effort: entry.effort,
        paths: entry.paths,
        status: entry.status,
        authored_by: entry.authored_by,
        rationale: entry.rationale,
    })
}
