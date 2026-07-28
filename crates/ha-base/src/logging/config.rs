use anyhow::Result;
use std::path::PathBuf;

use super::types::LogConfig;

// ── Config Persistence ───────────────────────────────────────────

const LOG_CONFIG_FILE: &str = "log_config.json";

pub fn load_log_config() -> Result<LogConfig> {
    let path = crate::paths::root_dir()?.join(LOG_CONFIG_FILE);
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(LogConfig::default())
    }
}

pub fn save_log_config(config: &LogConfig) -> Result<()> {
    let path = crate::paths::root_dir()?.join(LOG_CONFIG_FILE);
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    Ok(())
}

// ── Database Path Helper ─────────────────────────────────────────

pub fn db_path() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("logs.db"))
}
