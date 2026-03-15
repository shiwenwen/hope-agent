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

// ── Legacy Data Migration ────────────────────────────────────────

/// Migrate data from legacy paths to the new directory structure.
/// This is idempotent — it only moves files when the source exists
/// and the destination does not.
pub fn migrate_legacy_data() -> Result<()> {
    // 1. Migrate providers.json from dirs::config_dir()/open-computer/providers.json
    //    → ~/.opencomputer/config.json
    if let Some(config_dir) = dirs::config_dir() {
        let legacy_providers = config_dir.join("open-computer").join("providers.json");
        let new_config = config_path()?;
        migrate_file(&legacy_providers, &new_config)?;
    }

    // 2. Migrate auth.json from ~/.opencomputer/auth.json (root level)
    //    → ~/.opencomputer/credentials/auth.json
    let root = root_dir()?;
    let legacy_auth = root.join("auth.json");
    let new_auth = auth_path()?;
    migrate_file(&legacy_auth, &new_auth)?;

    Ok(())
}

/// Move a single file from src to dst if src exists and dst does not.
fn migrate_file(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    if src.exists() && !dst.exists() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::rename(src, dst).is_err() {
            // rename can fail across filesystems; fall back to copy + delete
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)?;
        }
        log::info!("Migrated {:?} → {:?}", src, dst);
    }
    Ok(())
}
