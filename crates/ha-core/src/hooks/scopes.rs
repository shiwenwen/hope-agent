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
//! global config per cwd at dispatch. Project/local files are loaded only when
//! their canonical workspace path and exact contents match an explicit trust
//! record; caches are invalidated by the trusted hashes + global generation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use arc_swap::ArcSwap;

use super::config::HookWorkspaceTrust;
use super::config::{HooksConfig, HooksConfigExt as _};
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
const MAX_SCOPE_FILE_BYTES: u64 = 1024 * 1024;
const MISSING_SCOPE_HASH: &str = "missing";

fn read_scope_file(path: &Path) -> Option<HooksConfig> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_SCOPE_FILE_BYTES {
        app_warn!(
            "hooks",
            "scopes",
            "ignoring hooks file larger than {} bytes: {}",
            MAX_SCOPE_FILE_BYTES,
            path.display()
        );
        return None;
    }
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

fn scope_file_hash(path: &Path) -> anyhow::Result<String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MISSING_SCOPE_HASH.to_string())
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        anyhow::bail!("Hook scope file must not be a symlink: {}", path.display());
    }
    if !metadata.is_file() {
        anyhow::bail!("Hook scope path is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_SCOPE_FILE_BYTES {
        anyhow::bail!(
            "Hook scope file exceeds {} bytes: {}",
            MAX_SCOPE_FILE_BYTES,
            path.display()
        );
    }
    if path.canonicalize()? != path {
        anyhow::bail!(
            "Hook scope file contains a path alias or symlink: {}",
            path.display()
        );
    }
    Ok(blake3::hash(&std::fs::read(path)?).to_hex().to_string())
}

fn scope_paths(canonical_workspace: &Path) -> (PathBuf, PathBuf) {
    let dir = canonical_workspace.join(".hope-agent");
    (dir.join("hooks.json"), dir.join("hooks.local.json"))
}

/// Canonicalize and bind one workspace approval to the exact project/local
/// Hook contents visible at approval time. Paths containing aliases or
/// symlinks are rejected instead of inheriting another path's trust.
pub fn build_workspace_trust(path: &Path) -> anyhow::Result<HookWorkspaceTrust> {
    if !path.is_absolute() {
        anyhow::bail!("Hook workspace path must be absolute");
    }
    let canonical = path.canonicalize()?;
    if canonical != path {
        anyhow::bail!(
            "Hook workspace path contains an alias or symlink: {}",
            path.display()
        );
    }
    if !canonical.is_dir() {
        anyhow::bail!("Hook workspace is not a directory: {}", path.display());
    }
    let (project, local) = scope_paths(&canonical);
    let project_hash = scope_file_hash(&project)?;
    let local_hash = scope_file_hash(&local)?;
    if project_hash == MISSING_SCOPE_HASH && local_hash == MISSING_SCOPE_HASH {
        anyhow::bail!(
            "Workspace has no .hope-agent/hooks.json or hooks.local.json: {}",
            path.display()
        );
    }
    read_trusted_scope_file(&project, &project_hash)
        .map_err(|error| anyhow::anyhow!("Invalid project Hook file: {error}"))?;
    read_trusted_scope_file(&local, &local_hash)
        .map_err(|error| anyhow::anyhow!("Invalid local Hook file: {error}"))?;
    let canonical_path = canonical
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Hook workspace path is not valid UTF-8"))?
        .to_string();
    Ok(HookWorkspaceTrust {
        canonical_path,
        project_hash,
        local_hash,
    })
}

fn trusted_scope_hashes(
    working_dir: &Path,
    trusts: &[HookWorkspaceTrust],
) -> Option<(PathBuf, String, String)> {
    if !working_dir.is_absolute() {
        return None;
    }
    let canonical = working_dir.canonicalize().ok()?;
    if canonical != working_dir {
        return None;
    }
    let canonical_str = canonical.to_str()?;
    let trust = trusts
        .iter()
        .find(|trust| trust.canonical_path == canonical_str)?;
    let (project, local) = scope_paths(&canonical);
    let project_hash = scope_file_hash(&project).ok()?;
    let local_hash = scope_file_hash(&local).ok()?;
    if project_hash != trust.project_hash || local_hash != trust.local_hash {
        return None;
    }
    Some((canonical, project_hash, local_hash))
}

/// Read and parse exactly the bytes that were approved. This second hash check
/// closes the mutation window between trust lookup and registry compilation:
/// changed contents fail closed instead of being compiled under an older hash.
fn read_trusted_scope_file(
    path: &Path,
    expected_hash: &str,
) -> anyhow::Result<Option<HooksConfig>> {
    if expected_hash == MISSING_SCOPE_HASH {
        return match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Ok(_) => anyhow::bail!("Previously missing Hook scope now exists"),
            Err(error) => Err(error.into()),
        };
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SCOPE_FILE_BYTES
    {
        anyhow::bail!("Trusted Hook scope is not a bounded regular file");
    }
    let bytes = std::fs::read(path)?;
    if blake3::hash(&bytes).to_hex().as_str() != expected_hash {
        anyhow::bail!("Trusted Hook scope content changed");
    }
    if path.canonicalize()?.as_path() != path {
        anyhow::bail!("Trusted Hook scope contains a path alias or symlink");
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}

struct CwdEntry {
    generation: u64,
    project_hash: String,
    local_hash: String,
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
/// project/local files are trusted. Cached per canonical cwd, invalidated by
/// exact content hashes + the global generation.
pub fn resolve_for_cwd(working_dir: Option<&Path>) -> Arc<HookRegistry> {
    let cfg = crate::config::cached_config();
    resolve_for_cwd_inner(
        working_dir,
        cfg.disable_all_hooks,
        &cfg.hook_workspace_trusts,
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
    trusted_workspaces: &[HookWorkspaceTrust],
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
    // Project/local scope is trusted per canonical workspace and exact file
    // content. A different cwd, symlink alias, moved directory, or content
    // change falls back to user/managed Hooks without reading project code.
    let Some((canonical_cwd, project_hash, local_hash)) =
        trusted_scope_hashes(cwd, trusted_workspaces)
    else {
        return registry::global();
    };
    let (project, local) = scope_paths(&canonical_cwd);
    let generation = GENERATION.load(Ordering::SeqCst);
    {
        let cache = cwd_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&canonical_cwd) {
            if entry.generation == generation
                && entry.project_hash == project_hash
                && entry.local_hash == local_hash
            {
                return entry.registry.clone();
            }
        }
    }
    // Rebuild: global base + project + local (union). Both files are verified
    // again from the exact bytes that will be parsed, so either scope changing
    // during resolution invalidates the whole workspace approval.
    let (Ok(project_config), Ok(local_config)) = (
        read_trusted_scope_file(&project, &project_hash),
        read_trusted_scope_file(&local, &local_hash),
    ) else {
        return registry::global();
    };
    let mut merged = (*global_config().load_full()).clone();
    if let Some(cfg) = project_config {
        merged.merge_from(cfg);
    }
    if let Some(cfg) = local_config {
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
        canonical_cwd,
        CwdEntry {
            generation,
            project_hash,
            local_hash,
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

/// Cwd-free pre-gate: `true` when NO handler can possibly fire for `event`,
/// answerable without knowing the session working dir.
///
/// [`any_handlers_for`] needs a cwd, and getting one costs a
/// `sessions.working_dir` lookup — which the synchronous `fire_*` helpers pay
/// *before* they reach the gate, because they must build the hook input (and
/// its `cwd`) first. That made "hooks cost nothing when none are configured"
/// only half true: an unconfigured install still did a DB read per
/// `FileChanged` / `Notification` / `PermissionDenied` / … fire. This gate
/// restores the promise for the default configuration.
///
/// **Exactness**: with no trusted workspace the global registry *is* the
/// effective registry for every cwd, so the answer is exact. With any trust
/// record present, a matching workspace could still contribute a handler, so
/// this conservatively returns `false` and the caller falls through to the real
/// cwd-aware gate. It is therefore only ever allowed to skip work, never to
/// decide that a handler runs.
pub fn definitely_no_handlers_for(event: HookEvent) -> bool {
    let cfg = crate::config::cached_config();
    definitely_no_handlers_for_inner(event, cfg.disable_all_hooks, &cfg.hook_workspace_trusts)
}

/// Inner form with the two config flags injected, so the soundness invariant is
/// unit-testable without touching the global cached config (mirrors
/// [`resolve_for_cwd_inner`]).
fn definitely_no_handlers_for_inner(
    event: HookEvent,
    disable_all_hooks: bool,
    trusted_workspaces: &[HookWorkspaceTrust],
) -> bool {
    if disable_all_hooks {
        return true;
    }
    if !trusted_workspaces.is_empty() {
        return false;
    }
    !resolve_for_cwd_inner(None, false, &[]).has_handlers_for(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir()
            .canonicalize()
            .expect("canonical temporary directory")
            .join(format!("{label}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn managed_path_is_absolute() {
        assert!(managed_path().is_absolute());
    }

    #[test]
    fn cwd_free_pregate_only_ever_skips_work() {
        // The pre-gate is an optimization: it may only ever SKIP work, never
        // decide that a handler runs. There is exactly ONE configuration in
        // which it could break that — a trusted project/local scope, where a
        // `.hope-agent/hooks.json` under some cwd can carry a handler the global
        // registry has never seen and which the pre-gate deliberately does not
        // go looking for. So build precisely that state and check both gates.
        //
        // (An earlier version of this test looped over events asserting
        // `pregate(e) => !cwd_gate(e)` with project scope OFF. That is a
        // TAUTOLOGY: with no trusts `resolve_for_cwd_inner(Some(cwd), …)`
        // early-returns `registry::global()` before touching the filesystem, so
        // both sides reduce to `registry::global().has_handlers_for(e)` and the
        // assertion cannot fail for any registry contents — a mutation making
        // the pre-gate unconditionally claim "no handlers" left it green.)
        let dir = temp_path("ha-hooks-pregate");
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"FileChanged":[{"hooks":[{"type":"command","command":"echo hi"}]}]}"#,
        )
        .unwrap();
        // Empty global config, so the only possible match is the project file.
        set_global_config(HooksConfig::default());
        let trust = build_workspace_trust(&dir).unwrap();

        assert!(
            resolve_for_cwd_inner(Some(&dir), false, std::slice::from_ref(&trust))
                .has_handlers_for(HookEvent::FileChanged),
            "setup: the cwd-aware gate must see the project-scope handler"
        );
        assert!(
            !definitely_no_handlers_for_inner(
                HookEvent::FileChanged,
                false,
                std::slice::from_ref(&trust),
            ),
            "UNSOUND: the pre-gate skipped an event that a project-scope handler \
             would have fired — it must decline to answer whenever project scope is on"
        );

        // Kill switch → definitely nothing, without consulting the registry.
        assert!(definitely_no_handlers_for_inner(
            HookEvent::PreToolUse,
            true,
            &[]
        ));

        let _ = std::fs::remove_dir_all(&dir);

        // The remaining property — that a CONFIGURED handler still fires THROUGH
        // the pre-gate on the default (project-scope-off) config — cannot be
        // asserted here without mutating the process-shared global registry.
        // It is pinned end-to-end by the `fire_*` liveness section of
        // `crates/ha-core/tests/hooks_e2e.rs`, which a broken pre-gate fails.
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
        // With this workspace trusted, a `.hope-agent/hooks.json` in the working
        // dir contributes hooks even when the global (user+managed) scope has
        // none for that event.
        let dir = temp_path("ha-hooks-scope");
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo hi"}]}]}"#,
        )
        .unwrap();

        // Start from an empty global config so the match must come from project.
        // The content-bound trust is injected directly so the test never
        // mutates the process-global cached config.
        set_global_config(HooksConfig::default());
        let trust = build_workspace_trust(&dir).unwrap();
        let reg = resolve_for_cwd_inner(Some(&dir), false, std::slice::from_ref(&trust));
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
        let reg = resolve_for_cwd_inner(None, true, &[]);
        assert!(
            !reg.has_handlers_for(HookEvent::PreToolUse),
            "kill switch must short-circuit before the no-cwd return"
        );

        // Cwd present path: same outcome — empty.
        let dir = temp_path("ha-hooks-kill");
        std::fs::create_dir_all(&dir).unwrap();
        let reg2 = resolve_for_cwd_inner(Some(&dir), true, &[]);
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
        // The default (no trusted workspaces) must ignore a repo's
        // checked-in hooks entirely — the supply-chain guard. Resolution returns
        // the global registry Arc unchanged, without ever reading the file.
        let dir = temp_path("ha-hooks-gate");
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"rm -rf /"}]}]}"#,
        )
        .unwrap();

        let reg = resolve_for_cwd_inner(Some(&dir), false, &[]);
        assert!(
            Arc::ptr_eq(&reg, &registry::global()),
            "gated-off resolution returns the global registry, not a merged one"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workspace_trust_is_invalidated_by_content_change() {
        let dir = temp_path("ha-hooks-content-trust");
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        let hooks = proj.join("hooks.json");
        std::fs::write(
            &hooks,
            r#"{"PreToolUse":[{"hooks":[{"type":"command","command":"echo approved"}]}]}"#,
        )
        .unwrap();
        set_global_config(HooksConfig::default());
        let trust = build_workspace_trust(&dir).unwrap();
        assert!(
            resolve_for_cwd_inner(Some(&dir), false, std::slice::from_ref(&trust))
                .has_handlers_for(HookEvent::PreToolUse)
        );

        std::fs::write(
            hooks,
            r#"{"PreToolUse":[{"hooks":[{"type":"command","command":"echo changed"}]}]}"#,
        )
        .unwrap();
        let reg = resolve_for_cwd_inner(Some(&dir), false, std::slice::from_ref(&trust));
        assert!(Arc::ptr_eq(&reg, &registry::global()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_an_approved_workspace_does_not_move_its_trust() {
        let original = temp_path("ha-hooks-original-trust");
        let moved = temp_path("ha-hooks-moved-trust");
        let proj = original.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"PreToolUse":[{"hooks":[{"type":"command","command":"echo hi"}]}]}"#,
        )
        .unwrap();
        let trust = build_workspace_trust(&original).unwrap();
        std::fs::rename(&original, &moved).unwrap();
        let reg = resolve_for_cwd_inner(Some(&moved), false, std::slice::from_ref(&trust));
        assert!(Arc::ptr_eq(&reg, &registry::global()));
        let _ = std::fs::remove_dir_all(&moved);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_workspace_alias_cannot_inherit_trust() {
        use std::os::unix::fs::symlink;

        let dir = temp_path("ha-hooks-real-trust");
        let alias = temp_path("ha-hooks-alias-trust");
        let proj = dir.join(".hope-agent");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("hooks.json"),
            r#"{"PreToolUse":[{"hooks":[{"type":"command","command":"echo hi"}]}]}"#,
        )
        .unwrap();
        let trust = build_workspace_trust(&dir).unwrap();
        symlink(&dir, &alias).unwrap();
        let reg = resolve_for_cwd_inner(Some(&alias), false, std::slice::from_ref(&trust));
        assert!(Arc::ptr_eq(&reg, &registry::global()));
        std::fs::remove_file(alias).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
