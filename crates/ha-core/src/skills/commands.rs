//! Shared command-layer skill operations used by both the Tauri desktop
//! shell ([`src-tauri/src/commands/skills.rs`]) and the HTTP server
//! ([`crates/ha-server/src/routes/skills.rs`]).
//!
//! Each function owns its config read / mutation and is transport-agnostic:
//! callers only translate request extraction and response formatting. The
//! `source: &str` argument (typically `"settings-ui"` or `"http"`) tags the
//! autosave backup so users / operators can trace which surface triggered a
//! change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::{
    author, auto_review, binary_in_path_public, bump_skill_version, check_all_skills_status,
    get_skill_content, is_masked_value, load_all_skills_with_budget, mask_value, SkillDetail,
    SkillStatus, SkillStatusEntry, SkillSummary,
};

// ── Catalog / detail ──────────────────────────────────────────────

pub fn list_skills() -> Vec<SkillSummary> {
    let store = crate::config::cached_config();
    let entries = load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    let disabled = &store.disabled_skills;
    entries
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            e.to_summary(enabled)
        })
        .collect()
}

pub fn get_skill_detail(name: &str) -> Option<SkillDetail> {
    let store = crate::config::cached_config();
    get_skill_content(name, &store.extra_skills_dirs, &store.disabled_skills)
}

// ── Extra skills directories ──────────────────────────────────────

pub fn get_extra_skills_dirs() -> Vec<String> {
    crate::config::cached_config().extra_skills_dirs.clone()
}

pub fn add_extra_skills_dir(dir: String, source: &str) -> Result<()> {
    crate::config::mutate_config(("extra_skills_dirs", source), |store| {
        if !store.extra_skills_dirs.contains(&dir) {
            store.extra_skills_dirs.push(dir);
        }
        Ok(())
    })?;
    bump_skill_version();
    Ok(())
}

pub fn remove_extra_skills_dir(dir: &str, source: &str) -> Result<()> {
    crate::config::mutate_config(("extra_skills_dirs", source), |store| {
        store.extra_skills_dirs.retain(|d| d != dir);
        Ok(())
    })?;
    bump_skill_version();
    Ok(())
}

// ── Enable / disable ──────────────────────────────────────────────

pub fn toggle_skill(name: String, enabled: bool, source: &str) -> Result<()> {
    crate::config::mutate_config(("disabled_skills", source), |store| {
        if enabled {
            store.disabled_skills.retain(|n| n != &name);
        } else if !store.disabled_skills.contains(&name) {
            store.disabled_skills.push(name);
        }
        Ok(())
    })?;
    bump_skill_version();
    Ok(())
}

// ── Skill env-check + per-skill env vars ──────────────────────────

pub fn get_skill_env_check() -> bool {
    crate::config::cached_config().skill_env_check
}

pub fn set_skill_env_check(enabled: bool, source: &str) -> Result<()> {
    crate::config::mutate_config(("skill_env_check", source), |store| {
        store.skill_env_check = enabled;
        Ok(())
    })?;
    bump_skill_version();
    Ok(())
}

/// Env vars for a skill with values masked (safe to return to UI).
pub fn get_skill_env_masked(name: &str) -> HashMap<String, String> {
    crate::config::cached_config()
        .skill_env
        .get(name)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, mask_value(&v)))
        .collect()
}

/// Set one env var for a skill. Returns Ok(()) without writing when `value`
/// is the masked placeholder — prevents the UI from accidentally replacing a
/// real secret with its own mask.
pub fn set_skill_env_var(skill: String, key: String, value: String, source: &str) -> Result<()> {
    if is_masked_value(&value) {
        return Ok(());
    }
    crate::config::mutate_config(("skill_env", source), |store| {
        store.skill_env.entry(skill).or_default().insert(key, value);
        Ok(())
    })?;
    bump_skill_version();
    Ok(())
}

pub fn remove_skill_env_var(skill: &str, key: &str, source: &str) -> Result<()> {
    crate::config::mutate_config(("skill_env", source), |store| {
        if let Some(map) = store.skill_env.get_mut(skill) {
            map.remove(key);
            if map.is_empty() {
                store.skill_env.remove(skill);
            }
        }
        Ok(())
    })?;
    bump_skill_version();
    Ok(())
}

/// `skill → { env_var → configured? }` snapshot (configured = user-set or
/// inherited from the process environment). Only skills that declare
/// `requires.env` are included.
pub fn get_skills_env_status() -> HashMap<String, HashMap<String, bool>> {
    let store = crate::config::cached_config();
    let entries = load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    let mut result = HashMap::new();
    for entry in &entries {
        if entry.requires.env.is_empty() {
            continue;
        }
        let configured = store.skill_env.get(&entry.name);
        let mut status = HashMap::new();
        for key in &entry.requires.env {
            let has_configured = configured
                .and_then(|m| m.get(key))
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let has_system = std::env::var(key).map(|v| !v.is_empty()).unwrap_or(false);
            status.insert(key.clone(), has_configured || has_system);
        }
        result.insert(entry.name.clone(), status);
    }
    result
}

pub fn get_skills_status() -> Vec<SkillStatusEntry> {
    let store = crate::config::cached_config();
    let entries = load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    check_all_skills_status(
        &entries,
        &store.disabled_skills,
        store.skill_env_check,
        &store.skill_env,
        &store.skill_allow_bundled,
    )
}

// ── Phase B' draft review ─────────────────────────────────────────

pub fn list_draft_skills() -> Vec<SkillSummary> {
    let store = crate::config::cached_config();
    let drafts = author::list_drafts(&store.extra_skills_dirs);
    let disabled = &store.disabled_skills;
    drafts
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            e.to_summary(enabled)
        })
        .collect()
}

pub fn activate_draft_skill(name: &str) -> Result<()> {
    author::set_skill_status(name, SkillStatus::Active)
}

pub fn discard_draft_skill(name: &str) -> Result<()> {
    author::delete_skill(name)
}

/// Read the current auto-review promotion mode.
/// `true` = newly auto-created skills are written directly as `Active` (skip review).
/// `false` = newly auto-created skills land in `Draft` for manual user activation.
pub fn get_auto_review_promotion() -> bool {
    matches!(
        crate::config::cached_config().skills.auto_review.promotion,
        auto_review::AutoReviewPromotion::Auto
    )
}

/// Toggle the auto-review promotion mode. `true` skips the draft buffer.
pub fn set_auto_review_promotion(auto: bool, source: &str) -> Result<()> {
    crate::config::mutate_config(("skills.auto_review", source), |store| {
        store.skills.auto_review.promotion = if auto {
            auto_review::AutoReviewPromotion::Auto
        } else {
            auto_review::AutoReviewPromotion::Draft
        };
        Ok(())
    })?;
    Ok(())
}

/// Read the master enabled flag for the auto-review pipeline.
/// `true` (default) = post-turn cooldown + threshold gating runs and may invoke
/// the side_query review. `false` = the pipeline is fully suppressed; nothing
/// auto-creates or auto-patches skills.
pub fn get_auto_review_enabled() -> bool {
    crate::config::cached_config().skills.auto_review.enabled
}

/// Toggle the master enabled flag.
pub fn set_auto_review_enabled(enabled: bool, source: &str) -> Result<()> {
    crate::config::mutate_config(("skills.auto_review", source), |store| {
        store.skills.auto_review.enabled = enabled;
        Ok(())
    })?;
    Ok(())
}

// ── Install dependency ────────────────────────────────────────────
//
// Spawns a package-manager process (`brew install …`, `npm install -g …`,
// `go install …`, `uv tool install …`) based on the skill's `install:` spec.
//
// SECURITY: the core function itself performs no authorization — callers
// decide whether the request is trusted:
//   * Tauri desktop: unconditional (user clicked in their own GUI = intent).
//   * HTTP surface: gate on `AppConfig.skills.allow_remote_install` — an
//     opt-in flag that must be flipped manually in settings. Without it,
//     anyone with the API key could pivot to arbitrary package installs.

/// Run the install spec at `spec_index` for `skill_name`. Returns combined
/// `stdout + stderr + binary verification` log on success, or an error with
/// the same format when the process exits non-zero.
pub async fn install_skill_dependency(skill_name: &str, spec_index: usize) -> Result<String> {
    let (cmd_program, cmd_args, bins) = {
        let store = crate::config::cached_config();
        let entries =
            load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
        let skill = entries
            .into_iter()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| anyhow!("Skill not found: {}", skill_name))?;

        let spec = skill
            .install
            .get(spec_index)
            .ok_or_else(|| anyhow!("Install spec index {} out of range", spec_index))?
            .clone();

        // OS guard — refuse to spawn platform-mismatched installers so the
        // user doesn't hit a cryptic process-spawn failure.
        if !spec.os.is_empty() {
            let current = std::env::consts::OS;
            let ok = spec.os.iter().any(|os| {
                os == current
                    || (os == "darwin" && current == "macos")
                    || (os == "mac" && current == "macos")
            });
            if !ok {
                return Err(anyhow!(
                    "Install spec is not available on this platform ({}), requires: {:?}",
                    current,
                    spec.os
                ));
            }
        }

        match spec.kind.as_str() {
            "brew" => {
                let formula = spec
                    .formula
                    .as_deref()
                    .ok_or_else(|| anyhow!("Brew install spec missing 'formula' field"))?;
                // Reject flag-looking / traversal args so we never feed the
                // spec into brew as an option flag.
                if formula.contains("..") || formula.contains('\\') || formula.starts_with('-') {
                    return Err(anyhow!("Invalid brew formula name"));
                }
                (
                    "brew".to_string(),
                    vec!["install".to_string(), formula.to_string()],
                    spec.bins,
                )
            }
            "node" => {
                let package = spec
                    .package
                    .as_deref()
                    .ok_or_else(|| anyhow!("Node install spec missing 'package' field"))?;
                if package.contains("..") || package.contains('\\') {
                    return Err(anyhow!("Invalid npm package name"));
                }
                (
                    "npm".to_string(),
                    vec!["install".to_string(), "-g".to_string(), package.to_string()],
                    spec.bins,
                )
            }
            "go" => {
                let module = spec
                    .go_module
                    .as_deref()
                    .ok_or_else(|| anyhow!("Go install spec missing 'module' field"))?;
                if module.contains("..") || module.contains('\\') {
                    return Err(anyhow!("Invalid go module path"));
                }
                (
                    "go".to_string(),
                    vec!["install".to_string(), module.to_string()],
                    spec.bins,
                )
            }
            "uv" => {
                let package = spec
                    .package
                    .as_deref()
                    .ok_or_else(|| anyhow!("UV install spec missing 'package' field"))?;
                (
                    "uv".to_string(),
                    vec![
                        "tool".to_string(),
                        "install".to_string(),
                        package.to_string(),
                    ],
                    spec.bins,
                )
            }
            other => return Err(anyhow!("Unsupported install kind: {}", other)),
        }
    };

    let args_ref: Vec<&str> = cmd_args.iter().map(String::as_str).collect();
    let output = run_install_command(&cmd_program, &args_ref).await?;

    let mut verification = String::new();
    for bin in &bins {
        if binary_in_path_public(bin) {
            verification.push_str(&format!("\n✓ {} found in PATH", bin));
        } else {
            verification.push_str(&format!("\n✗ {} not found in PATH", bin));
        }
    }

    bump_skill_version();
    Ok(format!("{}{}", output, verification))
}

async fn run_install_command(program: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run {} {}: {}", program, args.join(" "), e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(format!("{}{}", stdout, stderr))
    } else {
        Err(anyhow!(
            "{} {} failed (exit code {:?}):\n{}\n{}",
            program,
            args.join(" "),
            output.status.code(),
            stdout,
            stderr
        ))
    }
}

// ── Quick Import: preset skill source discovery ───────────────────
//
// Probes a small set of known third-party skill catalogs on the user's
// machine (Claude Code user-level + plugins, Anthropic agent-skills
// marketplace, OpenClaw and Hermes Agent clones) and returns a structured
// listing for the Settings / Onboarding "Quick Import" UI. Adding the
// returned paths to `extra_skills_dirs` is done with the existing
// `add_extra_skills_dir` flow — this RPC is read-only.

/// One discoverable skill catalog source. Always returned even when none of
/// its candidate paths exist, so the UI can render a complete row with the
/// expected paths grayed out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSkillSource {
    /// Stable id, used as the i18n lookup key suffix (e.g. "claude-code-user").
    pub id: String,
    /// i18n key for the human-readable label.
    pub label_key: String,
    /// Optional warning key — the UI surfaces a `⚠️` badge with this i18n
    /// string when present (e.g. OpenClaw "skills depend on external CLIs",
    /// Anthropic marketplace "Proprietary license").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_key: Option<String>,
    /// Concrete candidate paths probed for this source. Multiple paths can
    /// map to one logical source (Claude Code plugins scan two layouts;
    /// OW/HA repos may live in `~/Codes`, `~/git`, etc.).
    pub candidates: Vec<PresetCandidate>,
}

/// One concrete filesystem candidate inside a [`PresetSkillSource`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetCandidate {
    /// Absolute path. `~` is resolved on the server, not the browser.
    pub path: String,
    /// Whether the directory exists right now.
    pub exists: bool,
    /// Number of `SKILL.md`-bearing subdirectories found (recursive ≤ 2 levels).
    /// `0` when the path doesn't exist or contains no skills — the UI shows
    /// the row anyway so the user knows what was probed.
    pub skill_count: usize,
    /// True when this exact path is already in `extra_skills_dirs`.
    pub already_added: bool,
}

/// Probe known third-party skill catalog locations. Returns 5 sources in a
/// stable order regardless of what's installed locally — the UI renders
/// "not found" rows for absent ones.
pub fn discover_preset_skill_sources() -> Vec<PresetSkillSource> {
    let store = crate::config::cached_config();
    let extra: Vec<String> = store.extra_skills_dirs.clone();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            // No home → return all sources with empty candidate lists.
            return preset_sources_layout()
                .into_iter()
                .map(|(id, label, warning, _candidates)| PresetSkillSource {
                    id: id.to_string(),
                    label_key: label.to_string(),
                    warning_key: warning.map(|w| w.to_string()),
                    candidates: vec![],
                })
                .collect();
        }
    };

    let mut out = Vec::new();
    for (id, label, warning, candidate_layouts) in preset_sources_layout() {
        let mut candidates = Vec::new();
        for layout in candidate_layouts {
            let paths = layout.expand(&home);
            for path in paths {
                let exists = path.is_dir();
                let skill_count = if exists {
                    count_skills_in_dir(&path, 2)
                } else {
                    0
                };
                let path_str = path.to_string_lossy().to_string();
                let already_added = extra.iter().any(|d| d == &path_str);
                candidates.push(PresetCandidate {
                    path: path_str,
                    exists,
                    skill_count,
                    already_added,
                });
            }
        }
        // Deduplicate by path while preserving order.
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|c| seen.insert(c.path.clone()));
        out.push(PresetSkillSource {
            id: id.to_string(),
            label_key: label.to_string(),
            warning_key: warning.map(|w| w.to_string()),
            candidates,
        });
    }
    out
}

/// Layout describing how to compute one or more paths under a user's HOME.
/// Either fixed (one literal path) or globbed (e.g. `~/.claude/plugins/*/skills`).
enum CandidateLayout {
    Fixed(&'static str),
    Glob {
        prefix: &'static str,
        suffix: &'static str,
    },
}

impl CandidateLayout {
    fn expand(&self, home: &Path) -> Vec<PathBuf> {
        match self {
            CandidateLayout::Fixed(rel) => vec![home.join(rel)],
            CandidateLayout::Glob { prefix, suffix } => {
                let parent = home.join(prefix);
                let entries = match std::fs::read_dir(&parent) {
                    Ok(rd) => rd,
                    Err(_) => return vec![],
                };
                let mut out = Vec::new();
                for entry in entries.flatten() {
                    if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        let candidate = entry.path().join(suffix);
                        if candidate.is_dir() {
                            out.push(candidate);
                        }
                    }
                }
                out
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn preset_sources_layout() -> Vec<(
    &'static str,
    &'static str,
    Option<&'static str>,
    Vec<CandidateLayout>,
)> {
    vec![
        (
            "claude-code-user",
            "settings.skillsImport.cc.user",
            None,
            vec![CandidateLayout::Fixed(".claude/skills")],
        ),
        (
            "claude-code-plugins",
            "settings.skillsImport.cc.plugins",
            None,
            vec![
                CandidateLayout::Glob {
                    prefix: ".claude/plugins",
                    suffix: "skills",
                },
                CandidateLayout::Glob {
                    prefix: ".claude/plugins",
                    suffix: ".claude-plugin/skills",
                },
            ],
        ),
        (
            "anthropic-marketplace",
            "settings.skillsImport.cc.anthropic",
            Some("settings.skillsImport.warning.proprietary"),
            vec![CandidateLayout::Fixed(
                ".claude/plugins/marketplaces/anthropic-agent-skills/skills",
            )],
        ),
        (
            "openclaw",
            "settings.skillsImport.openclaw",
            Some("settings.skillsImport.warning.openclaw"),
            vec![
                CandidateLayout::Fixed("Codes/openclaw/skills"),
                CandidateLayout::Fixed("git/openclaw/skills"),
                CandidateLayout::Fixed("projects/openclaw/skills"),
                CandidateLayout::Fixed("openclaw/skills"),
            ],
        ),
        (
            "hermes-agent",
            "settings.skillsImport.hermes",
            None,
            vec![
                CandidateLayout::Fixed("Codes/hermes-agent/skills"),
                CandidateLayout::Fixed("git/hermes-agent/skills"),
                CandidateLayout::Fixed("projects/hermes-agent/skills"),
                CandidateLayout::Fixed("hermes-agent/skills"),
            ],
        ),
    ]
}

/// Count subdirectories containing a `SKILL.md`, recursing up to `max_depth`
/// levels deep (Hermes Agent groups skills in two-level category trees, so
/// depth 2 covers it; one level is enough for everything else).
fn count_skills_in_dir(dir: &Path, max_depth: usize) -> usize {
    fn walk(dir: &Path, depth: usize, max_depth: usize, count: &mut usize, budget: &mut usize) {
        if *budget == 0 || depth > max_depth {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.join("SKILL.md").is_file() {
                *count += 1;
                continue; // Don't recurse into a confirmed skill dir.
            }
            if depth < max_depth {
                walk(&path, depth + 1, max_depth, count, budget);
            }
        }
    }
    let mut count = 0usize;
    // Cap the walk to keep the discovery RPC snappy even when the user has
    // huge unrelated trees under the probed parents.
    let mut budget = 2000usize;
    walk(dir, 0, max_depth, &mut count, &mut budget);
    count
}

pub async fn trigger_skill_review_now(session_id: &str) -> Result<serde_json::Value> {
    let gate = auto_review::acquire_manual(session_id)
        .ok_or_else(|| anyhow::anyhow!("another review is already running for this session"))?;
    let report =
        auto_review::run_review_cycle(session_id, auto_review::ReviewTrigger::Manual, gate, None)
            .await?;
    Ok(serde_json::to_value(report)?)
}
