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

use anyhow::{anyhow, Result};

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

pub async fn trigger_skill_review_now(session_id: &str) -> Result<serde_json::Value> {
    let gate = auto_review::acquire_manual(session_id)
        .ok_or_else(|| anyhow::anyhow!("another review is already running for this session"))?;
    let report =
        auto_review::run_review_cycle(session_id, auto_review::ReviewTrigger::Manual, gate, None)
            .await?;
    Ok(serde_json::to_value(report)?)
}
