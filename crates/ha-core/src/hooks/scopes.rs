//! Multi-scope hook resolution (design §4).
//!
//! Hooks layer across four scopes, all UNIONed (every matching hook runs —
//! there is no override precedence):
//! - **user**: `~/.hope-agent/config.json` `hooks` (the cached `AppConfig`).
//! - **managed**: a system-wide file (`/etc/hope-agent/hooks.json` on unix),
//!   for enterprise-pushed hooks.
//! - **project**: `<session working dir>/.hope-agent/hooks.json`, checked into
//!   the repo so a team shares hooks.
//! - **local**: `<session working dir>/.hope-agent/hooks.local.json`,
//!   git-ignored per-developer additions.
//!
//! user + managed are global and compiled into [`registry::global`]. project +
//! local depend on the session's working dir, so they are merged on top of the
//! global config per cwd at dispatch, cached and invalidated by file mtime + a
//! global-reload generation counter.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use arc_swap::ArcSwap;

use super::config::HooksConfig;
use super::registry::{self, HookRegistry};
use super::types::HookEvent;

/// Bumped on every global reload so per-cwd caches rebuild against fresh
/// user/managed hooks.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The merged user+managed hooks config, kept so per-cwd merges have the global
/// base without re-reading the `AppConfig` / managed file each time.
fn global_config() -> &'static ArcSwap<HooksConfig> {
    static CELL: OnceLock<ArcSwap<HooksConfig>> = OnceLock::new();
    CELL.get_or_init(|| ArcSwap::from_pointee(HooksConfig::default()))
}

/// Update the cached merged user+managed config and bump the generation so
/// per-cwd caches invalidate. Called by [`registry::reload_from_config`].
pub fn set_global_config(cfg: HooksConfig) {
    global_config().store(Arc::new(cfg));
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// System-wide managed hooks file (enterprise scope).
pub fn managed_path() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/etc/hope-agent/hooks.json")
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
        base.join("hope-agent").join("hooks.json")
    }
}

/// Read + parse a scope file into a `HooksConfig`. `None` when the file is
/// absent or unparseable (a parse error is logged, never fatal).
fn read_scope_file(path: &Path) -> Option<HooksConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<HooksConfig>(&text) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            app_warn!(
                "hooks",
                "scopes",
                "ignoring unparseable hooks file {}: {}",
                path.display(),
                e
            );
            None
        }
    }
}

/// Load the managed scope (called during the global reload).
pub fn load_managed() -> HooksConfig {
    read_scope_file(&managed_path()).unwrap_or_default()
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

struct CwdEntry {
    generation: u64,
    project_mtime: Option<SystemTime>,
    local_mtime: Option<SystemTime>,
    registry: Arc<HookRegistry>,
}

fn cwd_cache() -> &'static Mutex<HashMap<PathBuf, CwdEntry>> {
    static CELL: OnceLock<Mutex<HashMap<PathBuf, CwdEntry>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the effective registry for a session working dir: the global
/// (user+managed) registry merged with the cwd's project + local hooks.
///
/// Returns the global registry directly when there is no working dir or no
/// project/local files exist (the common case — cheap, after up to two
/// `stat`s). Cached per cwd, invalidated by file mtime + the global generation.
pub fn resolve_for_cwd(working_dir: Option<&Path>) -> Arc<HookRegistry> {
    let cfg = crate::config::cached_config();
    resolve_for_cwd_inner(
        working_dir,
        cfg.disable_all_hooks,
        cfg.hooks_allow_project_scope,
    )
}

/// A process-shared empty registry returned whenever the master kill switch is
/// on. Reusing one `Arc` keeps the disable path allocation-free and lets it
/// short-circuit synchronously — independent of whether the async
/// `reload_from_config` listener has cleared the global registry yet.
fn empty_registry() -> Arc<HookRegistry> {
    static CELL: OnceLock<Arc<HookRegistry>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(HookRegistry::empty())).clone()
}

/// Inner resolution with the two config flags injected, so unit tests can
/// exercise the project-scope gate without touching the global cached config.
fn resolve_for_cwd_inner(
    working_dir: Option<&Path>,
    disable_all_hooks: bool,
    allow_project_scope: bool,
) -> Arc<HookRegistry> {
    // Master kill switch FIRST — it has to short-circuit BEFORE the `Some(cwd)`
    // check so a no-working-dir event (e.g. Notification with empty cwd) also
    // honors it, and it has to return an empty registry rather than the cached
    // global one. `reload_from_config` runs on the async `config:changed`
    // listener; in the window between the user flipping `disable_all_hooks=true`
    // and the listener firing, `registry::global()` still carries the old
    // handlers, so returning it here would silently run hooks the user just
    // disabled. Adversarial review HIGH.
    if disable_all_hooks {
        return empty_registry();
    }
    let Some(cwd) = working_dir else {
        return registry::global();
    };
    // Project/local scope is opt-in (supply-chain guard): a repo's checked-in
    // hooks must not auto-execute just because the session cwd points at it.
    // Off (the default) → only the global user/managed scope applies.
    if !allow_project_scope {
        return registry::global();
    }
    let project = cwd.join(".hope-agent").join("hooks.json");
    let local = cwd.join(".hope-agent").join("hooks.local.json");
    let project_mtime = mtime(&project);
    let local_mtime = mtime(&local);
    if project_mtime.is_none() && local_mtime.is_none() {
        return registry::global();
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    {
        let cache = cwd_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(cwd) {
            if entry.generation == generation
                && entry.project_mtime == project_mtime
                && entry.local_mtime == local_mtime
            {
                return entry.registry.clone();
            }
        }
    }
    // Rebuild: global base + project + local (union).
    let mut merged = (*global_config().load_full()).clone();
    if let Some(cfg) = read_scope_file(&project) {
        merged.merge_from(cfg);
    }
    if let Some(cfg) = read_scope_file(&local) {
        merged.merge_from(cfg);
    }
    let reg = Arc::new(HookRegistry::from_config(&merged));
    let mut cache = cwd_cache().lock().unwrap_or_else(|e| e.into_inner());
    // Bound the cache so a long-lived process visiting many working dirs can't
    // grow it without limit.
    const CAP: usize = 256;
    if cache.len() >= CAP {
        cache.clear();
    }
    cache.insert(
        cwd.to_path_buf(),
        CwdEntry {
            generation,
            project_mtime,
            local_mtime,
            registry: reg.clone(),
        },
    );
    reg
}

/// Whether any scope (global, or the cwd's project/local) has a handler for the
/// event. The single gate every fire path uses so project-only hooks fire even
/// when the user/managed scopes are empty.
pub fn any_handlers_for(event: HookEvent, working_dir: Option<&Path>) -> bool {
    resolve_for_cwd(working_dir).has_handlers_for(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_path_is_absolute() {
        assert!(managed_path().is_absolute());
    }

    #[test]
    fn no_working_dir_exercises_global_path() {
        // No cwd → resolves to the global registry with no IO or panic; the
        // gate is callable for any event. (Asserting a concrete bool would be
        // flaky — the global registry is process-shared across tests.)
        let _ = resolve_for_cwd(None);
        let _ = any_handlers_for(HookEvent::PreToolUse, None);
    }

    #[test]
    fn project_scope_loads_when_allowed() {
        // With project scope allowed, a `.hope-agent/hooks.json` in the working
        // dir contributes hooks even when the global (user+managed) scope has
        // none for that event.
        let dir = std::env::temp_dir().join(format!("ha-hooks-scope-{}", uuid::Uuid::new_v4()));
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo hi"}]}]}"#,
        )
        .unwrap();

        // Start from an empty global config so the match must come from project.
        // `allow_project_scope = true` (last arg) is injected directly so the
        // test never mutates the process-global cached config.
        set_global_config(HooksConfig::default());
        let reg = resolve_for_cwd_inner(Some(&dir), false, true);
        assert!(reg.has_handlers_for(HookEvent::PreToolUse));
        // `matcher:"Bash"` normalizes to `exec` at compile (the matcher alias
        // map), so the dispatched internal name `exec` is what matches the
        // project group.
        assert!(
            !reg.matching_handlers(HookEvent::PreToolUse, Some("exec"))
                .is_empty(),
            "project Bash matcher contributes a handler"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disable_all_hooks_returns_empty_synchronously_regardless_of_cwd() {
        // `reload_from_config` is async (config:changed listener). Between the
        // user flipping the kill switch and the listener firing, the global
        // registry still carries old handlers — so the kill switch HAS to be
        // honored here, in the synchronous read path, by returning an empty
        // registry. Covers both no-cwd (e.g. Notification) and cwd-present
        // events.
        //
        // Seed the global config with a non-empty match-anything PreToolUse
        // handler so any leak would show up immediately.
        let cfg: HooksConfig = serde_json::from_str(
            r#"{
                "PreToolUse": [
                    { "hooks": [
                        { "type": "command", "command": "true" }
                    ]}
                ]
            }"#,
        )
        .expect("seed kill-switch test config");
        set_global_config(cfg);

        // No cwd path: must NOT return the (populated) global registry — empty.
        let reg = resolve_for_cwd_inner(None, true, false);
        assert!(
            !reg.has_handlers_for(HookEvent::PreToolUse),
            "kill switch must short-circuit before the no-cwd return"
        );

        // Cwd present path: same outcome — empty.
        let dir = std::env::temp_dir().join(format!("ha-hooks-kill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let reg2 = resolve_for_cwd_inner(Some(&dir), true, true);
        assert!(
            !reg2.has_handlers_for(HookEvent::PreToolUse),
            "kill switch must short-circuit on the cwd-present branch too"
        );

        // The empty registry is shared (cheap), so two disable-mode calls
        // hand back the same Arc — no per-call allocation.
        assert!(Arc::ptr_eq(&reg, &reg2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_scope_gated_off_returns_global() {
        // The default (`allow_project_scope = false`) must ignore a repo's
        // checked-in hooks entirely — the supply-chain guard. Resolution returns
        // the global registry Arc unchanged, without ever reading the file.
        let dir = std::env::temp_dir().join(format!("ha-hooks-gate-{}", uuid::Uuid::new_v4()));
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"rm -rf /"}]}]}"#,
        )
        .unwrap();

        let reg = resolve_for_cwd_inner(Some(&dir), false, false);
        assert!(
            Arc::ptr_eq(&reg, &registry::global()),
            "gated-off resolution returns the global registry, not a merged one"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
