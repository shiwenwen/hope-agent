use anyhow::{bail, Result};
use arc_swap::ArcSwap;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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
        let initial = load_initial_config();
        ArcSwap::from_pointee(initial)
    })
}

#[derive(Debug, Clone)]
struct ConfigLoadFailure {
    path: PathBuf,
    error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigHealth {
    pub ok: bool,
    pub status: String,
    pub path: Option<String>,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl ConfigHealth {
    fn ok(path: Option<PathBuf>) -> Self {
        Self {
            ok: true,
            status: "ok".into(),
            path: path.map(|p| p.to_string_lossy().to_string()),
            error: None,
            message: None,
        }
    }

    fn failed(status: &str, path: Option<PathBuf>, error: impl ToString) -> Self {
        let error = error.to_string();
        let failure = ConfigLoadFailure {
            path: path.clone().unwrap_or_else(|| PathBuf::from("config.json")),
            error: error.clone(),
        };
        Self {
            ok: false,
            status: status.into(),
            path: path.map(|p| p.to_string_lossy().to_string()),
            error: Some(error),
            message: Some(load_failure_message(&failure)),
        }
    }
}

fn load_failure() -> &'static Mutex<Option<ConfigLoadFailure>> {
    static FAILURE: OnceLock<Mutex<Option<ConfigLoadFailure>>> = OnceLock::new();
    FAILURE.get_or_init(|| Mutex::new(None))
}

fn record_config_load_failure(path: &Path, error: impl ToString) {
    let mut slot = load_failure()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    *slot = Some(ConfigLoadFailure {
        path: path.to_path_buf(),
        error: error.to_string(),
    });
}

fn clear_config_load_failure() {
    let mut slot = load_failure()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    *slot = None;
}

fn current_config_load_failure() -> Option<ConfigLoadFailure> {
    load_failure()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

fn load_failure_message(failure: &ConfigLoadFailure) -> String {
    format!(
        "Refusing to use the default in-memory config because an existing config.json failed \
         to load at {:?}: {}. Repair config.json, restore an autosave, or restart after the \
         transient read error clears; Hope Agent will not overwrite the existing file with \
         defaults.",
        failure.path, failure.error
    )
}

fn ensure_no_initial_load_failure_for_write() -> Result<()> {
    if let Some(failure) = current_config_load_failure() {
        bail!("{}", load_failure_message(&failure));
    }
    Ok(())
}

/// Minimum spacing between ambient disk-read recovery attempts while a load
/// failure is recorded. Without this, *every* `load_config()` call (the
/// settings page alone issues ~20 on open) synchronously re-reads the
/// unreadable file on the caller's thread — a burst of blocking IO exactly
/// when the filesystem is already misbehaving. Within the cooldown callers
/// fail fast with the recorded error instead. The user-facing Retry path
/// (`config_health`) is intentionally not throttled.
const RECOVER_RETRY_COOLDOWN: Duration = Duration::from_secs(2);

fn last_recover_attempt() -> &'static Mutex<Option<Instant>> {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn recover_from_load_failure() -> Result<AppConfig> {
    recover_from_load_failure_inner(false)
}

/// Recovery that ignores [`RECOVER_RETRY_COOLDOWN`]. For the explicit,
/// user-driven Retry path only ([`config_health`]) — a person clicking Retry
/// expects an immediate fresh read, never a stale "still broken" answer just
/// because an ambient `load_config()` happened to attempt recovery in the last
/// 2 seconds.
fn recover_from_load_failure_forced() -> Result<AppConfig> {
    recover_from_load_failure_inner(true)
}

fn recover_from_load_failure_inner(force: bool) -> Result<AppConfig> {
    let Some(previous_failure) = current_config_load_failure() else {
        return Ok((*cached_config()).clone());
    };

    {
        let mut slot = last_recover_attempt()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !force {
            if let Some(prev) = *slot {
                if prev.elapsed() < RECOVER_RETRY_COOLDOWN {
                    // Re-check under the lock: another thread may have recovered
                    // successfully in the window, in which case the failure is
                    // already cleared and we should hand back the good config
                    // rather than a stale "broken" error.
                    if current_config_load_failure().is_none() {
                        return Ok((*cached_config()).clone());
                    }
                    bail!("{}", load_failure_message(&previous_failure));
                }
            }
        }
        *slot = Some(Instant::now());
    }

    match read_from_disk() {
        Ok(cfg) => {
            app_info!(
                "config",
                "load",
                "Recovered config.json after earlier load failure at {:?}",
                previous_failure.path
            );
            cache().store(Arc::new(cfg.clone()));
            clear_config_load_failure();
            Ok(cfg)
        }
        Err(e) => {
            let path = config_path().unwrap_or_else(|_| previous_failure.path.clone());
            record_config_load_failure(&path, e.to_string());
            let failure = current_config_load_failure().unwrap_or(previous_failure);
            app_error!(
                "config",
                "load",
                "config.json is still unreadable at {:?}: {}",
                failure.path,
                failure.error
            );
            bail!("{}", load_failure_message(&failure));
        }
    }
}

/// Return the current config health for startup UX and recovery screens.
///
/// If a previous startup read failed, this performs a fresh disk read so
/// transient Windows file locks can self-heal via a user-visible "Retry"
/// action. It never writes defaults over an existing unreadable file.
pub fn config_health() -> ConfigHealth {
    let path = match config_path() {
        Ok(path) => path,
        Err(e) => return ConfigHealth::failed("path_error", None, e),
    };

    // Ensure lazy startup has had a chance to record an existing-file load
    // failure before we report health.
    let _ = cached_config();

    if path.exists() {
        if let Err(e) = read_from_path(&path) {
            record_config_load_failure(&path, e.to_string());
        }
    }

    if current_config_load_failure().is_some() {
        match recover_from_load_failure_forced() {
            Ok(_) => ConfigHealth::ok(Some(path)),
            Err(_) => {
                let failure = current_config_load_failure().unwrap_or(ConfigLoadFailure {
                    path: path.clone(),
                    error: "unknown config load failure".into(),
                });
                ConfigHealth {
                    ok: false,
                    status: "load_failed".into(),
                    path: Some(failure.path.to_string_lossy().to_string()),
                    error: Some(failure.error.clone()),
                    message: Some(load_failure_message(&failure)),
                }
            }
        }
    } else {
        ConfigHealth::ok(Some(path))
    }
}

/// Populate the in-memory cache on first access.
///
/// **Data-loss guard.** A bare `read_from_disk().unwrap_or_default()` silently
/// turns *any* read/parse failure (a UTF-8 BOM from a Windows editor, a
/// transient AV/file lock, a truncated write) into a pristine default config.
/// The very next `save_config` — e.g. the onboarding-complete write — then
/// persists that default *over* the user's real `config.json`, permanently
/// destroying providers / MCP servers / onboarding state and looping the
/// first-run wizard on every launch (issue #326). So when an **existing** file
/// fails to load we (1) shout in the log, (2) copy it aside to a
/// `config.json.corrupt-<ts>` sidecar, and (3) enter a fail-closed guard that
/// blocks later writes until the real file can be loaded again.
fn load_initial_config() -> AppConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[config] cannot resolve config path: {e}");
            app_error!("config", "load", "Cannot resolve config path: {}", e);
            return AppConfig::default();
        }
    };

    if !path.exists() {
        app_info!(
            "config",
            "load",
            "No config.json at {:?}; starting from defaults (fresh install)",
            path
        );
        clear_config_load_failure();
        return AppConfig::default();
    }

    match read_from_disk() {
        Ok(cfg) => {
            app_info!("config", "load", "Loaded config.json from {:?}", path);
            clear_config_load_failure();
            cfg
        }
        Err(e) => {
            eprintln!("[config] FAILED to load existing {:?}: {e}", path);
            app_error!(
                "config",
                "load",
                "Failed to load existing config.json at {:?}: {} — backing it up to a .corrupt-<ts> \
                 sidecar and blocking config writes so the original settings stay recoverable",
                path,
                e
            );
            preserve_unreadable_config(&path);
            record_config_load_failure(&path, e.to_string());
            AppConfig::default()
        }
    }
}

/// Best-effort copy of an unreadable config to a timestamped `.corrupt-<ts>`
/// sidecar next to the original so a transient read failure can never silently
/// erase the user's settings. Never panics; logs and moves on.
fn preserve_unreadable_config(path: &std::path::Path) {
    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(format!(".corrupt-{ts}"));
    let sidecar = PathBuf::from(sidecar);
    match std::fs::copy(path, &sidecar) {
        Ok(_) => app_warn!(
            "config",
            "load",
            "Preserved unreadable config.json → {:?} for recovery",
            sidecar
        ),
        Err(e) => app_warn!(
            "config",
            "load",
            "Could not preserve unreadable config.json → {:?}: {}",
            sidecar,
            e
        ),
    }
}

fn read_from_disk() -> Result<AppConfig> {
    let path = config_path()?;
    read_from_path(&path)
}

fn read_from_path(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let data = std::fs::read_to_string(path)?;
    parse_config_str(&data)
}

/// Parse `config.json` text into [`AppConfig`].
///
/// Tolerates a leading UTF-8 BOM (`U+FEFF`): Windows editors such as Notepad
/// prepend one when a user hand-edits the file, and `serde_json` otherwise
/// rejects it as an invalid leading character — which used to nuke the whole
/// config (issue #326).
fn parse_config_str(data: &str) -> Result<AppConfig> {
    let trimmed = data.strip_prefix('\u{feff}').unwrap_or(data);
    let config: AppConfig = serde_json::from_str(trimmed)?;
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

/// Test-only: replace the in-memory cache without touching disk. Lets unit
/// tests that read `cached_config()` start from a known empty state instead
/// of inheriting the developer's `~/.hope-agent/config.json` (which would
/// otherwise leak provider lists, active models, etc. into tests on the
/// developer machine).
#[cfg(test)]
pub fn replace_cache_for_test(config: AppConfig) {
    cache().store(Arc::new(config));
    clear_config_load_failure();
}

/// Load an owned copy of the app config. Clones the cached snapshot;
/// use when you need to mutate and then call [`save_config`]. Read-only
/// callers should use [`cached_config`] instead.
pub fn load_config() -> Result<AppConfig> {
    // `cached_config()` initializes the cache lazily; if that initialization had
    // to fall back to defaults, immediately try to recover from disk before
    // handing callers a mutable snapshot.
    let snapshot = cached_config();
    if current_config_load_failure().is_some() {
        return recover_from_load_failure();
    }
    Ok((*snapshot).clone())
}

/// Persist the app config to disk and refresh the in-memory cache.
///
/// Callers must pass the full, mutated config — this function does not merge
/// with the existing on-disk content.
pub fn save_config(config: &AppConfig) -> Result<()> {
    save_config_with_change(config, "app", None)
}

fn save_config_with_change(
    config: &AppConfig,
    change_category: &str,
    change_source: Option<&str>,
) -> Result<()> {
    let path = config_path()?;
    ensure_no_initial_load_failure_for_write()?;
    if path.exists() {
        if let Err(e) = read_from_path(&path) {
            eprintln!("[config] refusing to overwrite unreadable {:?}: {e}", path);
            app_error!(
                "config",
                "save_config",
                "Refusing to overwrite unreadable existing config.json at {:?}: {}",
                path,
                e
            );
            preserve_unreadable_config(&path);
            record_config_load_failure(&path, e.to_string());
            ensure_no_initial_load_failure_for_write()?;
        }
    }
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
    clear_config_load_failure();

    // Notify subscribers (frontend hot-reload hooks, in-process listeners).
    // Best-effort: the bus may not be initialized in tests or CLI-only modes.
    if let Some(bus) = crate::globals::get_event_bus() {
        let mut payload = serde_json::json!({ "category": change_category });
        if let Some(source) = change_source {
            payload["source"] = serde_json::json!(source);
        }
        bus.emit("config:changed", payload);
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
    let mut snapshot = load_config()?;
    let result = f(&mut snapshot)?;
    save_config_with_change(&snapshot, reason.0, Some(reason.1))?;
    // ConfigChange hook (observation): fire with the real category + source.
    crate::hooks::fire_config_change(reason.0, reason.1);
    Ok(result)
}

/// Async wrapper for [`mutate_config`]: runs the whole clone → mutate →
/// persist → publish cycle on tokio's blocking pool.
///
/// [`mutate_config`] holds the global write lock across synchronous file IO
/// (pre-write validation read, autosave backup copy, `fs::write`). Called
/// inline from an async fn that pins a tokio worker for the full duration —
/// and if the IO stalls (antivirus, cloud-synced home dir), pinned workers
/// accumulate until the runtime starves. **Async contexts must use this
/// wrapper** so config writes only ever tie up expendable blocking-pool
/// threads (see `crate::blocking`).
pub async fn mutate_config_async<F, T>(reason: (&str, &str), f: F) -> Result<T>
where
    F: FnOnce(&mut AppConfig) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let category = reason.0.to_string();
    let source = reason.1.to_string();
    // Label with the caller's closure type, not the wrapper below.
    let label = std::any::type_name::<F>();
    crate::blocking::run_blocking_labeled(label, move || mutate_config((&category, &source), f))
        .await
}

/// Force a fresh disk read into the cache. Use after an out-of-band write
/// to `config.json` (e.g. [`crate::backup::restore_backup`]) so hot-path
/// readers don't keep serving the stale snapshot.
pub fn reload_cache_from_disk() -> Result<()> {
    let fresh = read_from_disk()?;
    cache().store(Arc::new(fresh));
    clear_config_load_failure();
    // Notify subscribers that the cache was force-reloaded (e.g. rollback).
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": "app", "source": "reload" }),
        );
    }
    Ok(())
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn plain_json_parses() {
        let cfg = parse_config_str(r#"{"providers":[],"theme":"dark"}"#).expect("parse");
        assert_eq!(cfg.theme, "dark");
    }

    #[test]
    fn utf8_bom_is_tolerated() {
        // Windows Notepad prepends EF BB BF on save; serde_json otherwise
        // rejects it and the whole config would be discarded (issue #326).
        let with_bom = format!("\u{feff}{}", r#"{"providers":[],"theme":"light"}"#);
        let cfg = parse_config_str(&with_bom).expect("BOM-prefixed config should parse");
        assert_eq!(cfg.theme, "light");
    }

    #[test]
    fn pretty_printed_config_with_bom_roundtrips() {
        let original = AppConfig {
            theme: "dark".into(),
            ..AppConfig::default()
        };
        let pretty = serde_json::to_string_pretty(&original).expect("serialize");
        let with_bom = format!("\u{feff}{pretty}");
        let parsed = parse_config_str(&with_bom).expect("parse pretty + BOM");
        assert_eq!(parsed.theme, "dark");
    }

    #[test]
    fn initial_load_failure_blocks_default_overwrite() {
        let tmp = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", tmp.path())], || {
            clear_config_load_failure();

            let path = tmp.path().join("config.json");
            let original = r#"{"providers":[],"mcpServers":[{"name":"keep-me"}]}"#;
            std::fs::write(&path, original).expect("write original config");

            struct ClearGuard;
            impl Drop for ClearGuard {
                fn drop(&mut self) {
                    clear_config_load_failure();
                }
            }
            let _guard = ClearGuard;

            record_config_load_failure(&path, "simulated startup read failure");
            let mut replacement = AppConfig::default();
            replacement.onboarding.completed_version = crate::config::CURRENT_ONBOARDING_VERSION;

            let err = save_config(&replacement).expect_err("save must fail closed");
            assert!(err.to_string().contains("Refusing to use the default"));
            assert_eq!(
                std::fs::read_to_string(&path).expect("read original config"),
                original
            );
        });
    }
}
