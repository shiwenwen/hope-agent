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

use anyhow::Result;

use super::{
    author, auto_review, bump_skill_version, check_all_skills_status, get_skill_content,
    is_masked_value, load_all_skills_with_budget, mask_value, SkillDetail, SkillStatus,
    SkillStatusEntry, SkillSummary,
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

pub async fn trigger_skill_review_now(session_id: &str) -> Result<serde_json::Value> {
    let gate = auto_review::acquire_manual(session_id)
        .ok_or_else(|| anyhow::anyhow!("another review is already running for this session"))?;
    let report =
        auto_review::run_review_cycle(session_id, auto_review::ReviewTrigger::Manual, gate, None)
            .await?;
    Ok(serde_json::to_value(report)?)
}
