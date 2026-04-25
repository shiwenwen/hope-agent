use anyhow::Result;
use std::path::{Path, PathBuf};

const CONFIG_FILENAME: &str = "openclaw.json";
const LEGACY_CONFIG_FILENAME: &str = "clawdbot.json";

/// Resolve OpenClaw's state directory.
///
/// Precedence:
/// 1. `OPENCLAW_STATE_DIR` env var (used by tests and power users)
/// 2. `~/.openclaw/` if it exists and contains `openclaw.json` or `clawdbot.json`
/// 3. `~/.clawdbot/` (legacy pre-rebrand) if it exists and has either config file
/// 4. `~/.openclaw/` (canonical) — even when empty, as the "configured fallback"
///
/// Returning the canonical path when nothing exists keeps callers like
/// `scan_openclaw_full` deterministic; they then fail config-load with a
/// "not found" message that the UI maps to the "no OpenClaw detected" branch.
pub fn resolve_openclaw_state_dir() -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("OPENCLAW_STATE_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(expand_tilde(trimmed));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    let candidates = [home.join(".openclaw"), home.join(".clawdbot")];
    for dir in &candidates {
        if dir_has_config(dir) {
            return Ok(dir.clone());
        }
    }
    Ok(home.join(".openclaw"))
}

/// Path to the OpenClaw config file inside `state_dir`. Prefers
/// `openclaw.json`, falls back to legacy `clawdbot.json` only when the modern
/// filename is missing.
pub fn resolve_openclaw_config_path(state_dir: &Path) -> PathBuf {
    let modern = state_dir.join(CONFIG_FILENAME);
    if modern.exists() {
        return modern;
    }
    let legacy = state_dir.join(LEGACY_CONFIG_FILENAME);
    if legacy.exists() {
        return legacy;
    }
    modern
}

fn dir_has_config(dir: &Path) -> bool {
    dir.join(CONFIG_FILENAME).exists() || dir.join(LEGACY_CONFIG_FILENAME).exists()
}

/// `PathBuf`-flavored thin wrapper over the shared `tools::expand_tilde`.
pub fn expand_tilde(path: &str) -> PathBuf {
    PathBuf::from(crate::tools::expand_tilde(path))
}

/// Default workspace directory inside the resolved state dir (matches
/// OpenClaw's `~/.openclaw/workspace`). Used as fallback when an agent has no
/// explicit `workspace` override.
pub fn default_workspace(state_dir: &Path) -> PathBuf {
    state_dir.join("workspace")
}

/// OpenClaw stores per-agent auth profiles at
/// `{state_dir}/agents/{agent_id}/agent/auth-profiles.json`.
pub fn agent_dir(state_dir: &Path, agent_id: &str) -> PathBuf {
    state_dir.join("agents").join(agent_id).join("agent")
}

pub fn auth_profiles_path(state_dir: &Path, agent_id: &str) -> PathBuf {
    agent_dir(state_dir, agent_id).join("auth-profiles.json")
}
