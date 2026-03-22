use anyhow::Result;
use std::path::PathBuf;

// ── Root Directory ───────────────────────────────────────────────

/// Returns the root directory for all OpenComputer data: ~/.opencomputer/
pub fn root_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".opencomputer"))
}

// ── Config ───────────────────────────────────────────────────────

/// Global config file path: ~/.opencomputer/config.json
pub fn config_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("config.json"))
}

// ── Agents ───────────────────────────────────────────────────────

/// Agents root directory: ~/.opencomputer/agents/
pub fn agents_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("agents"))
}

/// Specific agent directory: ~/.opencomputer/agents/{id}/
pub fn agent_dir(id: &str) -> Result<PathBuf> {
    Ok(agents_dir()?.join(id))
}

// ── User Config ─────────────────────────────────────────────────

/// User config file path: ~/.opencomputer/user.json
pub fn user_config_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("user.json"))
}

// ── Credentials ──────────────────────────────────────────────────

/// Credentials directory: ~/.opencomputer/credentials/
pub fn credentials_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("credentials"))
}

/// OAuth auth token path: ~/.opencomputer/credentials/auth.json
pub fn auth_path() -> Result<PathBuf> {
    Ok(credentials_dir()?.join("auth.json"))
}

// ── Skills ───────────────────────────────────────────────────────

/// Skills directory: ~/.opencomputer/skills/
pub fn skills_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("skills"))
}

// ── Agent Home ───────────────────────────────────────────────────

/// Main agent home directory: ~/.opencomputer/home/
pub fn home_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("home"))
}

/// Named agent home directory: ~/.opencomputer/{name}-home/
pub fn agent_home_dir(name: &str) -> Result<PathBuf> {
    Ok(root_dir()?.join(format!("{}-home", name)))
}

// ── Attachments ──────────────────────────────────────────────────

/// Attachments directory for a session: ~/.opencomputer/attachments/{session_id}/
pub fn attachments_dir(session_id: &str) -> Result<PathBuf> {
    Ok(root_dir()?.join("attachments").join(session_id))
}

// ── Avatars ──────────────────────────────────────────────────────

/// Avatars directory: ~/.opencomputer/avatars/
pub fn avatars_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("avatars"))
}

// ── Logs ──────────────────────────────────────────────────────────

/// Logs database path: ~/.opencomputer/logs.db
pub fn logs_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("logs.db"))
}

/// Logs directory for plain text log files: ~/.opencomputer/logs/
pub fn logs_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("logs"))
}

// ── Share ────────────────────────────────────────────────────────

/// Shared directory for inter-agent data: ~/.opencomputer/share/
#[allow(dead_code)]
pub fn share_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("share"))
}

// ── Cron ────────────────────────────────────────────────────────

/// Cron database path: ~/.opencomputer/cron.db
pub fn cron_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("cron.db"))
}

// ── Memory ──────────────────────────────────────────────────────

/// Memory database path: ~/.opencomputer/memory.db
pub fn memory_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("memory.db"))
}

/// Embedding model cache directory: ~/.opencomputer/models/
pub fn models_cache_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("models"))
}

// ── Browser Profiles ────────────────────────────────────────────

/// Browser profiles root directory: ~/.opencomputer/browser-profiles/
pub fn browser_profiles_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("browser-profiles"))
}

/// Specific browser profile directory: ~/.opencomputer/browser-profiles/{profile_name}/
pub fn browser_profile_dir(profile_name: &str) -> Result<PathBuf> {
    Ok(browser_profiles_dir()?.join(profile_name))
}

// ── Crash Journal ──────────────────────────────────────────────────

/// Crash journal file path: ~/.opencomputer/crash_journal.json
pub fn crash_journal_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("crash_journal.json"))
}

// ── Backups ────────────────────────────────────────────────────────

/// Backups directory: ~/.opencomputer/backups/
pub fn backups_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("backups"))
}

// ── Directory Initialization ─────────────────────────────────────

/// Ensure all required directories exist.
pub fn ensure_dirs() -> Result<()> {
    let dirs_to_create = [
        root_dir()?,
        credentials_dir()?,
        skills_dir()?,
        agents_dir()?,
        home_dir()?,
        avatars_dir()?,
        share_dir()?,
        logs_dir()?,
        models_cache_dir()?,
        browser_profiles_dir()?,
        backups_dir()?,
    ];
    for dir in &dirs_to_create {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}


