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

// ── Credentials ──────────────────────────────────────────────────

/// Credentials directory: ~/.opencomputer/credentials/
pub fn credentials_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("credentials"))
}

/// OAuth auth token path: ~/.opencomputer/credentials/auth.json
pub fn auth_path() -> Result<PathBuf> {
    Ok(credentials_dir()?.join("auth.json"))
}

// ── Agent Home ───────────────────────────────────────────────────

/// Main agent home directory: ~/.opencomputer/home/
pub fn home_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("home"))
}

/// Named agent home directory: ~/.opencomputer/{name}-home/
#[allow(dead_code)]
pub fn agent_home_dir(name: &str) -> Result<PathBuf> {
    Ok(root_dir()?.join(format!("{}-home", name)))
}

// ── Share ────────────────────────────────────────────────────────

/// Shared directory for inter-agent data: ~/.opencomputer/share/
#[allow(dead_code)]
pub fn share_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("share"))
}

// ── Directory Initialization ─────────────────────────────────────

/// Ensure all required directories exist.
pub fn ensure_dirs() -> Result<()> {
    let dirs_to_create = [
        root_dir()?,
        credentials_dir()?,
        home_dir()?,
        share_dir()?,
    ];
    for dir in &dirs_to_create {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}


