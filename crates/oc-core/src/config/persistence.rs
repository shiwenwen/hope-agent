use anyhow::Result;
use arc_swap::ArcSwap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::paths;

use super::AppConfig;

// ── Persistence ───────────────────────────────────────────────────

fn config_path() -> Result<PathBuf> {
    paths::config_path()
}

/// Process-wide in-memory snapshot of the app config.
///
/// Populated lazily on first access and refreshed atomically on every
/// successful [`save_config`]. All reads are lock-free acquire loads — this is
/// why [`cached_config`] is safe to call from hot paths (tool execution, chat
/// loops, memory lookups, channel workers) without any synchronization cost.
fn cache() -> &'static ArcSwap<AppConfig> {
    static CACHE: OnceLock<ArcSwap<AppConfig>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let initial = read_from_disk().unwrap_or_default();
        ArcSwap::from_pointee(initial)
    })
}

fn read_from_disk() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let data = std::fs::read_to_string(&path)?;
    let config: AppConfig = serde_json::from_str(&data)?;
    Ok(config)
}

/// Shared read-only snapshot of the app config. **Lock-free, zero data
/// clone** — one atomic acquire load plus an `Arc` refcount bump.
///
/// Use this in hot paths and read-only accesses. The returned `Arc` is a
/// point-in-time snapshot; a concurrent [`save_config`] will not affect it.
pub fn cached_config() -> Arc<AppConfig> {
    cache().load_full()
}

/// Load an owned copy of the app config. Clones the cached snapshot;
/// use when you need to mutate and then call [`save_config`]. Read-only
/// callers should use [`cached_config`] instead.
pub fn load_config() -> Result<AppConfig> {
    Ok((*cached_config()).clone())
}

/// Persist the app config to disk and refresh the in-memory cache.
///
/// Callers must pass the full, mutated config — this function does not merge
/// with the existing on-disk content.
pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Debug: log channel account IDs on every save to detect accidental overwrite
    let account_ids: Vec<&str> = config
        .channels
        .accounts
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    app_debug!(
        "config",
        "save_config",
        "Saving config with {} channel account(s): {:?}",
        account_ids.len(),
        account_ids
    );
    // Autosave the pre-change file so every settings edit is rollback-able.
    // Failures are logged inside the helper and never block the write.
    crate::backup::snapshot_before_write(&path, "config");

    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;

    // Atomically publish the new snapshot so subsequent cached_config() calls
    // see the refreshed state without touching disk.
    cache().store(Arc::new(config.clone()));
    Ok(())
}

/// Force a fresh disk read into the cache. Use after an out-of-band write
/// to `config.json` (e.g. [`crate::backup::restore_backup`]) so hot-path
/// readers don't keep serving the stale snapshot.
pub fn reload_cache_from_disk() -> Result<()> {
    let fresh = read_from_disk()?;
    cache().store(Arc::new(fresh));
    Ok(())
}
