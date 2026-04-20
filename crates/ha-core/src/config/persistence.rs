use anyhow::Result;
use arc_swap::ArcSwap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

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

    // Notify subscribers (frontend hot-reload hooks, in-process listeners).
    // Best-effort: the bus may not be initialized in tests or CLI-only modes.
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit("config:changed", serde_json::json!({ "category": "app" }));
    }
    Ok(())
}

/// Serialize all "read-modify-write" config edits process-wide. Reads stay
/// lock-free via [`cached_config`]; writers take this lock for the duration of
/// the clone → mutate → persist → publish cycle to prevent lost updates when
/// two save handlers race.
fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Single entry-point for every config mutation. Takes the global write lock,
/// clones the latest cached snapshot, applies `f`, persists to disk, and
/// atomically publishes the new snapshot so any `cached_config()` call made
/// after `mutate_config` returns sees the change.
///
/// `reason` is a `(category, source)` pair recorded in the autosave snapshot
/// and `config:changed` event so user-visible rollbacks and frontend hot-reload
/// hooks can tell *what* changed.
///
/// # Example
/// ```ignore
/// use ha_core::config::mutate_config;
/// mutate_config(("image_generate", "settings-ui"), |cfg| {
///     cfg.image_generate = new_image_config;
///     Ok(())
/// })?;
/// ```
pub fn mutate_config<F, T>(reason: (&str, &str), f: F) -> Result<T>
where
    F: FnOnce(&mut AppConfig) -> Result<T>,
{
    let _write_guard = write_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _reason_guard = crate::backup::scope_save_reason(reason.0, reason.1);
    let mut snapshot = (*cached_config()).clone();
    let result = f(&mut snapshot)?;
    save_config(&snapshot)?;
    Ok(result)
}

/// Force a fresh disk read into the cache. Use after an out-of-band write
/// to `config.json` (e.g. [`crate::backup::restore_backup`]) so hot-path
/// readers don't keep serving the stale snapshot.
pub fn reload_cache_from_disk() -> Result<()> {
    let fresh = read_from_disk()?;
    cache().store(Arc::new(fresh));
    // Notify subscribers that the cache was force-reloaded (e.g. rollback).
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": "app", "source": "reload" }),
        );
    }
    Ok(())
}
