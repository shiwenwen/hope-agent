use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ha_core::paths;

use super::frontmatter::parse_frontmatter;
use super::types::*;

// ── Bundled Skills ──────────────────────────────────────────────

/// Cached bundled skills directory path. Only SUCCESS is cached: resolution
/// now performs real IO (embedded extraction), so a transient failure (disk
/// full, AV lock) must retry on the next call instead of pinning `None` for
/// the process lifetime.
static BUNDLED_SKILLS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Resolve the bundled skills directory shipped with the application.
///
/// Search order:
/// 1. `HOPE_AGENT_BUNDLED_SKILLS_DIR` env override
/// 2. Workspace root `skills/` via `CARGO_MANIFEST_DIR` (dev builds — skill
///    edits take effect without re-extraction)
/// 3. Skills embedded in the binary, extracted to the data dir (release /
///    packaged / Docker / bare binary)
fn resolve_bundled_skills_dir() -> Option<PathBuf> {
    // 1. Env override
    if let Ok(dir) = std::env::var("HOPE_AGENT_BUNDLED_SKILLS_DIR") {
        let p = PathBuf::from(dir.trim());
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. Dev builds only: workspace root skills/ (CARGO_MANIFEST_DIR is crates/ha-skills)
    //    Use env!() compile-time macro so the path is baked in at build time,
    //    since the runtime env var is only present under `cargo run`.
    #[cfg(debug_assertions)]
    {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = Path::new(manifest_dir).parent().and_then(|p| p.parent());
        if let Some(root) = workspace_root {
            let candidate = root.join("skills");
            if candidate.is_dir() && looks_like_skills_dir(&candidate) {
                return Some(candidate);
            }
        }
    }

    // 3. Embedded in the binary.
    match super::embedded::ensure_extracted() {
        Ok(dir) => Some(dir),
        Err(e) => {
            app_warn!(
                "skills",
                "discovery",
                "failed to extract embedded bundled skills: {e:#}"
            );
            None
        }
    }
}

/// Quick check: does the directory contain at least one subdirectory with SKILL.md?
pub(super) fn looks_like_skills_dir(dir: &Path) -> bool {
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
    if let Some(dir) = BUNDLED_SKILLS_DIR.get() {
        return Some(dir);
    }
    let dir = resolve_bundled_skills_dir()?;
    // Two threads may both resolve on first miss; get_or_init keeps one,
    // the loser's identical PathBuf is dropped.
    Some(BUNDLED_SKILLS_DIR.get_or_init(|| dir))
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

/// Maximum number of levels below a configured skills root at which a
/// `SKILL.md` will be picked up. Real-world layouts observed:
///
/// - Level 1: `<root>/<skill>/SKILL.md` — Hope Agent / OpenClaw / Anthropic
///   marketplace. The base case.
/// - Level 2: `<root>/<category>/<skill>/SKILL.md` — Hermes Agent groups
///   skills by category (`apple/`, `creative/`, `software-development/`, …).
/// - Level 2 (alt): `<root>/<package>/skills/<skill>/SKILL.md` — explicit
///   `skills/` re-entry, still capped at 2 hops by the same constant.
///
/// Going deeper would scan unrelated directory trees (test fixtures, vendored
/// dependencies) and inflate startup latency without observed benefit. The
/// per-root [`SkillPromptBudget::max_candidates_per_root`] cap still applies
/// across the whole walk, so a runaway tree is bounded.
const MAX_SKILL_DEPTH: usize = 2;

/// Discover skills from a directory.
///
/// Each immediate subdirectory with a `SKILL.md` is loaded as a skill. When
/// a subdirectory has neither a `SKILL.md` nor a recognizable layout marker,
/// the loader recurses one level deeper (up to [`MAX_NESTED_SKILL_DEPTH`])
/// to find the SKILL.md. This handles three real-world layouts:
///
/// 1. Flat: `<root>/<skill>/SKILL.md` (Hope Agent, OpenClaw, Anthropic marketplace)
/// 2. Nested-`skills/`: `<root>/<x>/skills/<skill>/SKILL.md` (OW extensions)
/// 3. Category-grouped: `<root>/<category>/<skill>/SKILL.md` (Hermes Agent)
pub(super) fn load_skills_from_dir(
    dir: &Path,
    source: &str,
    budget: &SkillPromptBudget,
) -> Vec<SkillEntry> {
    let mut total_candidates = 0usize;
    let Ok(canonical_root) = dir.canonicalize() else {
        return Vec::new();
    };
    load_skills_from_dir_recursive(
        dir,
        &canonical_root,
        source,
        budget,
        0,
        &mut total_candidates,
    )
}

fn load_skills_from_dir_recursive(
    dir: &Path,
    canonical_root: &Path,
    source: &str,
    budget: &SkillPromptBudget,
    depth: usize,
    total_candidates: &mut usize,
) -> Vec<SkillEntry> {
    // `depth` counts levels already descended from the configured root;
    // SKILL.md found by this call lands at `depth + 1`. Once `depth + 1 >
    // MAX_SKILL_DEPTH` the call would only see SKILL.md beyond the cap, so
    // bail out early without even reading the directory.
    if depth + 1 > MAX_SKILL_DEPTH {
        return Vec::new();
    }

    // A symlinked category/skill directory must not escape the configured
    // source root. read_dir's entry file_type check below rejects directory
    // symlinks; this canonical boundary is the defense-in-depth backstop.
    let Ok(canonical_dir) = dir.canonicalize() else {
        return Vec::new();
    };
    if !canonical_dir.starts_with(canonical_root) {
        app_warn!(
            "skills",
            "loader",
            "Skipping skill directory outside configured source root: {}",
            dir.display()
        );
        return Vec::new();
    }

    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.flatten() {
        *total_candidates += 1;
        if *total_candidates > budget.max_candidates_per_root {
            app_warn!(
                "skills",
                "loader",
                "Reached max candidates limit ({}) for directory tree rooted at: {}",
                budget.max_candidates_per_root,
                dir.display()
            );
            break;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        let skill_md_is_regular = std::fs::symlink_metadata(&skill_md)
            .map(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            .unwrap_or(false);
        if skill_md_is_regular {
            if let Some(skill) = load_single_skill(&skill_md, &path, source, budget.max_file_bytes)
            {
                entries.push(skill);
            }
            continue;
        }

        // Two recursion targets:
        //   1. Explicit `skills/` re-entry (OW extensions ship with this).
        //   2. Otherwise descend into the directory itself — covers Hermes-
        //      style category grouping and any other 2-level nesting.
        // Both share the same depth budget; the next call's entry-check
        // refuses anything past MAX_SKILL_DEPTH.
        let nested_skills = path.join("skills");
        let target = if nested_skills.is_dir() {
            nested_skills
        } else {
            path
        };
        let nested = load_skills_from_dir_recursive(
            &target,
            canonical_root,
            source,
            budget,
            depth + 1,
            total_candidates,
        );
        entries.extend(nested);
    }

    entries
}

/// Standard repository skill sources, ordered from lowest to highest
/// precedence: repository root first, current working directory last. Scanning
/// stops at the nearest `.git` directory/file; outside a repository only the
/// current directory is considered. Symlinked `.agents/skills` roots are
/// rejected so another tree cannot inherit this repository's discovery scope.
pub(super) fn workspace_skill_sources(cwd: &Path) -> Vec<(PathBuf, String)> {
    let Ok(canonical_cwd) = cwd.canonicalize() else {
        return Vec::new();
    };
    if !canonical_cwd.is_dir() {
        return Vec::new();
    }
    let repository_root = canonical_cwd
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(canonical_cwd.as_path())
        .to_path_buf();
    let mut ancestors = Vec::new();
    for ancestor in canonical_cwd.ancestors() {
        ancestors.push(ancestor.to_path_buf());
        if ancestor == repository_root {
            break;
        }
    }
    ancestors.reverse();

    ancestors
        .into_iter()
        .filter_map(|ancestor| {
            let candidate = ancestor.join(".agents").join("skills");
            let metadata = std::fs::symlink_metadata(&candidate).ok()?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
            if candidate.canonicalize().ok().as_deref() != Some(candidate.as_path()) {
                app_warn!(
                    "skills",
                    "discovery",
                    "Skipping aliased repository skill source: {}",
                    candidate.display()
                );
                return None;
            }
            let source = if ancestor == canonical_cwd {
                "workspace-shared"
            } else {
                "repo-ancestor"
            };
            Some((candidate, source.to_string()))
        })
        .collect()
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
        aliases: parsed.aliases,
        description: parsed.description,
        when_to_use: parsed.when_to_use,
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
        allowed_tools_declared: parsed.allowed_tools_declared,
        context_mode: parsed.context_mode,
        agent: parsed.agent,
        effort: parsed.effort,
        paths: parsed.paths,
        status: parsed.status,
        authored_by: parsed.authored_by,
        rationale: parsed.rationale,
        display: parsed.display,
    })
}

/// Load all skills from all configured sources.
///
/// Sources (lowest -> highest precedence):
/// 0. Bundled skills (shipped with the application, lowest)
/// 1. Shared skills (~/.agents/skills/, cross-tool convention)
/// 2. Extra directories (user-imported)
/// 3. Managed skills (~/.hope-agent/skills/)
/// 4. Repository-standard skills (`.agents/skills`) from repo root to the
///    explicit session workspace
/// 5. Hope project skills (`.hope-agent/skills/` in that workspace, highest)
pub fn load_all_skills_with_extra(
    extra_dirs: &[String],
    workspace_dir: Option<&Path>,
) -> Vec<SkillEntry> {
    load_all_skills_with_budget(extra_dirs, &SkillPromptBudget::default(), workspace_dir)
}

/// Load all skills with configurable budget limits.
pub fn load_all_skills_with_budget(
    extra_dirs: &[String],
    budget: &SkillPromptBudget,
    workspace_dir: Option<&Path>,
) -> Vec<SkillEntry> {
    let mut all: Vec<SkillEntry> = Vec::new();

    // Collect from all sources (lowest precedence first)
    let mut sources: Vec<(PathBuf, String)> = Vec::new();

    // 0. Bundled skills (shipped with the application)
    if let Some(dir) = bundled_skills_dir() {
        sources.push((dir.clone(), "bundled".to_string()));
    }

    // 1. Shared skills: ~/.agents/skills/ (cross-tool convention)
    if let Some(home) = dirs::home_dir() {
        let shared = home.join(".agents").join("skills");
        if shared.is_dir() {
            sources.push((shared, "shared".to_string()));
        }
    }

    // 2. Extra directories (user-imported)
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

    // 3. Managed skills: ~/.hope-agent/skills/
    if let Ok(dir) = paths::skills_dir() {
        sources.push((dir, "managed".to_string()));
    }

    // 4. Standard repository skills: every .agents/skills from the nearest
    // repository root down to the caller-resolved session workspace. More
    // specific descendants are loaded later and therefore override ancestor
    // names deterministically. Never fall back to the process cwd: desktop
    // and daemon processes serve unrelated workspaces concurrently.
    if let Some(workspace_dir) = workspace_dir {
        sources.extend(workspace_skill_sources(workspace_dir));

        // 5. Hope project-specific skills remain the highest-precedence source
        // for backward compatibility. Reject path aliases/symlinks just like
        // the portable repository sources.
        if let Ok(canonical_workspace) = workspace_dir.canonicalize() {
            let project_skills = canonical_workspace.join(".hope-agent").join("skills");
            let project_is_canonical_dir = std::fs::symlink_metadata(&project_skills)
                .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                .unwrap_or(false)
                && project_skills.canonicalize().ok().as_deref() == Some(project_skills.as_path());
            if project_is_canonical_dir {
                sources.push((project_skills, "project".to_string()));
            }
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
    load_all_skills_with_extra(&[], None)
}

/// Build slash command definitions from user-invocable skills.
/// Returns skill entries that should be registered as slash commands.
pub fn get_invocable_skills(
    extra_dirs: &[String],
    disabled: &[String],
    workspace_dir: Option<&Path>,
) -> Vec<SkillEntry> {
    let skills = load_all_skills_with_extra(extra_dirs, workspace_dir);
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
    let skills = load_all_skills_with_extra(extra_dirs, None);
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
        allowed_tools_declared: entry.allowed_tools_declared,
        context_mode: entry.context_mode,
        agent: entry.agent,
        effort: entry.effort,
        paths: entry.paths,
        status: entry.status,
        authored_by: entry.authored_by,
        rationale: entry.rationale,
        display: entry.display,
    })
}
