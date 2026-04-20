//! Persist / load the [`OnboardingState`] sub-object on `AppConfig`.
//!
//! These helpers intentionally go through [`crate::config::save_config`] so
//! every change produces an autosave snapshot. Callers can pair each call
//! with [`crate::backup::scope_save_reason`] for a more descriptive label.

use anyhow::Result;
use serde_json::Value;

use crate::config::{load_config, save_config, OnboardingState, CURRENT_ONBOARDING_VERSION};

/// Return the current onboarding state, patching in a "legacy completed"
/// signal for users who pre-date the wizard.
///
/// Heuristic: if `completed_version == 0` but the config already has at
/// least one provider, assume the user completed the pre-wizard "provider
/// setup" in a prior version and treat them as onboarded at version `1`.
/// This avoids re-showing the wizard to existing users on upgrade — they
/// can still trigger it explicitly from Settings → "Re-run setup wizard".
///
/// The inferred value is **not written back**: a silent config mutation
/// would produce confusing autosave snapshots. Callers that observe
/// `state.completed_version` should route through this function.
pub fn get_state() -> Result<OnboardingState> {
    let cfg = load_config()?;
    Ok(infer_legacy_completed(
        &cfg.onboarding,
        !cfg.providers.is_empty(),
    ))
}

/// Pure helper that applies the legacy-inference heuristic. Split out so the
/// front-end can share the rule via a dedicated Tauri command without
/// re-reading config.
pub fn infer_legacy_completed(raw: &OnboardingState, has_providers: bool) -> OnboardingState {
    // Only infer for users who have NEVER seen the wizard. Once a user
    // has completed it at least once, `ever_completed` stays true across
    // `reset()` calls, so an explicit rerun correctly lands back in the
    // wizard even though providers already exist.
    if raw.completed_version == 0 && !raw.ever_completed && has_providers && raw.draft.is_none() {
        OnboardingState {
            completed_version: CURRENT_ONBOARDING_VERSION,
            completed_at: raw.completed_at.clone(),
            skipped_steps: raw.skipped_steps.clone(),
            draft: None,
            draft_step: 0,
            ever_completed: true,
        }
    } else {
        raw.clone()
    }
}

/// Persist the draft blob for a wizard that was exited mid-way.
pub fn save_draft(step: u32, draft: Value) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "draft");
    let mut cfg = load_config()?;
    cfg.onboarding.draft = Some(draft);
    cfg.onboarding.draft_step = step;
    save_config(&cfg)
}

/// Mark the wizard as completed at the current version. Clears draft state.
pub fn mark_completed() -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "complete");
    let mut cfg = load_config()?;
    cfg.onboarding.completed_version = CURRENT_ONBOARDING_VERSION;
    cfg.onboarding.completed_at = Some(chrono::Utc::now().to_rfc3339());
    cfg.onboarding.draft = None;
    cfg.onboarding.draft_step = 0;
    cfg.onboarding.ever_completed = true;
    save_config(&cfg)
}

/// Record that the user skipped a named step. Duplicate keys are ignored.
pub fn mark_skipped(step_key: &str) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "skip");
    let mut cfg = load_config()?;
    if !cfg.onboarding.skipped_steps.iter().any(|s| s == step_key) {
        cfg.onboarding.skipped_steps.push(step_key.to_string());
    }
    save_config(&cfg)
}

/// Reset onboarding to "never completed" so the wizard shows again on next
/// launch. Does not touch providers, user config, or any other step data.
///
/// `ever_completed` is pinned true — rerun implies the wizard was seen
/// before (even if a prior version didn't persist the flag), so the
/// legacy-upgrade heuristic can't short-circuit it.
pub fn reset() -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "reset");
    let mut cfg = load_config()?;
    cfg.onboarding = OnboardingState {
        ever_completed: true,
        ..OnboardingState::default()
    };
    save_config(&cfg)
}
