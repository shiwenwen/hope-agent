//! Pure data writers shared by GUI and CLI wizards.
//!
//! Each `apply_*` takes an already-validated input struct and persists the
//! change through the normal config / user_config / agent_loader path —
//! that means every call hits the autosave snapshot machinery and produces
//! a rollback-able backup tagged `onboarding/<step>`.
//!
//! The functions live in core so `src-tauri` commands and the CLI wizard
//! share a single source of truth.

use anyhow::{Context, Result};

use crate::agent_config::AgentConfig;
use crate::agent_loader::{ensure_default_agent, save_agent_config, DEFAULT_AGENT_ID};
use crate::config::{load_config, save_config, ApprovalTimeoutAction};
use crate::onboarding::presets::PersonalityPreset;
use crate::user_config::{load_user_config, save_user_config_to_disk};

/// Step 1 — language. Writes to both `user.language` and `config.language`
/// so legacy paths that read from either keep working.
pub fn apply_language(language: &str) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "language");
    let mut cfg = load_config()?;
    cfg.language = language.to_string();
    save_config(&cfg)?;

    let _g2 = crate::backup::scope_save_reason("onboarding", "language");
    let mut user = load_user_config()?;
    user.language = Some(language.to_string());
    save_user_config_to_disk(&user)?;
    Ok(())
}

/// Step 3 — profile. Fields are all optional so the wizard can partial-save.
#[derive(Debug, Clone, Default)]
pub struct ProfileStepInput {
    pub name: Option<String>,
    pub timezone: Option<String>,
    pub ai_experience: Option<String>,
    pub response_style: Option<String>,
}

pub fn apply_profile(input: ProfileStepInput) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "profile");
    let mut user = load_user_config()?;
    merge_optional(&mut user.name, input.name);
    merge_optional(&mut user.timezone, input.timezone);
    merge_optional(&mut user.ai_experience, input.ai_experience);
    merge_optional(&mut user.response_style, input.response_style);
    save_user_config_to_disk(&user)?;
    Ok(())
}

/// Merge a wizard field into a user-config field.
///
/// The wizard UI treats an unchanged text field as "" (its initial
/// React state), indistinguishable from an explicit "clear". We bias
/// toward preserving existing data: both `None` and empty-string
/// leave the target untouched. Users who genuinely want to wipe a
/// profile value do it from Settings → Profile, where the UI
/// pre-populates with the current value and a clear edit is unambiguous.
fn merge_optional(target: &mut Option<String>, new_value: Option<String>) {
    if let Some(v) = new_value {
        if !v.is_empty() {
            *target = Some(v);
        }
    }
}

/// Step 4 — personality preset. Writes only to the default agent; users who
/// later create additional agents manage those independently.
pub fn apply_personality_preset(preset: PersonalityPreset) -> Result<()> {
    ensure_default_agent().context("ensure default agent exists")?;

    let dir = crate::paths::agent_dir(DEFAULT_AGENT_ID)?;
    let cfg_path = dir.join("agent.json");
    let mut config: AgentConfig = if cfg_path.exists() {
        let data = std::fs::read_to_string(&cfg_path)
            .with_context(|| format!("read {}", cfg_path.display()))?;
        serde_json::from_str(&data).with_context(|| format!("parse {}", cfg_path.display()))?
    } else {
        AgentConfig::default()
    };

    config.personality = preset.to_config();
    save_agent_config(DEFAULT_AGENT_ID, &config)
}

/// Step 5 — safety. Only the two approval fields are exposed; Dangerous
/// Mode is deliberately unreachable from the wizard.
#[derive(Debug, Clone)]
pub struct SafetyStepInput {
    /// If `false`, future approvals auto-proceed with no timeout.
    pub approvals_enabled: bool,
}

pub fn apply_safety(input: SafetyStepInput) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "safety");
    let mut cfg = load_config()?;
    if input.approvals_enabled {
        if cfg.permission.approval_timeout_action == ApprovalTimeoutAction::Proceed {
            cfg.permission.approval_timeout_action = ApprovalTimeoutAction::Deny;
        }
        if cfg.permission.approval_timeout_secs == 0 {
            cfg.permission.approval_timeout_secs = 300;
        }
    } else {
        cfg.permission.approval_timeout_action = ApprovalTimeoutAction::Proceed;
        cfg.permission.approval_timeout_secs = 0;
    }
    save_config(&cfg)
}

/// Step 6 — skills. `disabled` overwrites the existing disabled list, which
/// is what the wizard expects: it round-trips the current list to the UI
/// and writes back the edited version.
pub fn apply_skills(disabled: Vec<String>) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "skills");
    let mut cfg = load_config()?;
    cfg.disabled_skills = disabled;
    save_config(&cfg)
}

/// Step 7 — server. `bind_addr` of `None` keeps current; same for `api_key`.
/// Pass `Some(String::new())` to clear `api_key`.
#[derive(Debug, Clone, Default)]
pub struct ServerStepInput {
    pub bind_addr: Option<String>,
    pub api_key: Option<String>,
}

pub fn apply_server(input: ServerStepInput) -> Result<()> {
    let _g = crate::backup::scope_save_reason("onboarding", "server");
    let mut cfg = load_config()?;
    if let Some(addr) = input.bind_addr {
        if !addr.is_empty() {
            cfg.server.bind_addr = addr;
        }
    }
    match input.api_key {
        Some(k) if k.is_empty() => cfg.server.api_key = None,
        Some(k) => cfg.server.api_key = Some(k),
        None => {}
    }
    save_config(&cfg)
}

/// Generate a fresh `hope_<uuid_no_dashes>` api key. Bound here (instead of
/// in the commands layer) so GUI + CLI + tests share the same format.
pub fn generate_api_key() -> String {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("hope_{}", uuid)
}
