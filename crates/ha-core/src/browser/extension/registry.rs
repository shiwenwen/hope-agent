use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

use super::BrowserBackendContext;

const LEASE_TTL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TabOwnerKind {
    User,
    Agent,
}

#[derive(Debug, Clone)]
pub(super) struct ElementLocator {
    pub ref_id: u32,
    pub role: String,
    pub text: String,
    pub selector: String,
}

#[derive(Debug, Clone)]
pub(super) struct FinalizeTabAction {
    pub tab_id: i64,
    pub owner_kind: TabOwnerKind,
    pub close: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ScopedFinalizeTabAction {
    pub scope: String,
    pub action: FinalizeTabAction,
}

#[derive(Debug, Clone)]
pub(super) struct RemovedTabLease {
    pub tab_id: i64,
    pub scope: String,
    pub owner_kind: TabOwnerKind,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ClaimOutcome {
    pub stolen_from: Vec<String>,
}

#[derive(Debug, Clone)]
struct ControlledTab {
    owner_kind: TabOwnerKind,
    url: Option<String>,
    title: Option<String>,
    lease_expires_at: i64,
}

#[derive(Debug, Default)]
struct ScopeState {
    active_tab_id: Option<i64>,
    controlled_tabs: HashMap<i64, ControlledTab>,
    element_refs: Vec<ElementLocator>,
    snapshot_url: Option<String>,
}

#[derive(Debug, Default)]
struct Registry {
    scopes: HashMap<String, ScopeState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PersistedRegistry {
    version: u32,
    saved_at: i64,
    scopes: Vec<PersistedScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PersistedScope {
    scope: String,
    active_tab_id: Option<i64>,
    tabs: Vec<PersistedTabLease>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PersistedTabLease {
    tab_id: i64,
    owner_kind: TabOwnerKind,
    lease_expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_seen_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_seen_title: Option<String>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(load_persisted_registry()))
}

// Tab-ownership scope key. Agent browser tool calls always carry a non-empty
// session_id (set by the engine), so they map to "session:{id}". The turn_id
// branch is currently inert (no caller populates turn_id) and the "global"
// fallback only applies to session-less internal probes (e.g. status), which
// are not concurrent multi-session tool dispatch — so the shared "global" scope
// cannot cross-contaminate real agent sessions.
pub(super) fn scope_key(ctx: &BrowserBackendContext) -> String {
    if let Some(session_id) = ctx.session_id.as_deref().filter(|s| !s.is_empty()) {
        return format!("session:{session_id}");
    }
    if let Some(turn_id) = ctx.turn_id.as_deref().filter(|s| !s.is_empty()) {
        return format!("turn:{turn_id}");
    }
    "global".to_string()
}

fn lock_registry() -> std::sync::MutexGuard<'static, Registry> {
    match registry().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn now_unix_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn new_controlled_tab(
    owner_kind: TabOwnerKind,
    url: Option<String>,
    title: Option<String>,
) -> ControlledTab {
    ControlledTab {
        owner_kind,
        url,
        title,
        lease_expires_at: now_unix_secs().saturating_add(LEASE_TTL_SECS),
    }
}

#[cfg(not(test))]
fn load_persisted_registry() -> Registry {
    match load_persisted_registry_inner() {
        Ok(registry) => registry,
        Err(e) => {
            app_warn!(
                "browser",
                "extension_registry",
                "failed to load persisted browser registry: {}",
                e
            );
            Registry::default()
        }
    }
}

#[cfg(test)]
fn load_persisted_registry() -> Registry {
    Registry::default()
}

#[cfg(not(test))]
fn load_persisted_registry_inner() -> Result<Registry> {
    use anyhow::Context as _;

    let path = crate::paths::browser_extension_registry_path()?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let persisted: PersistedRegistry =
        serde_json::from_str(&raw).with_context(|| format!("decoding {}", path.display()))?;
    let registry = registry_from_snapshot(persisted, now_unix_secs());
    Ok(registry)
}

fn persist_registry(registry: &Registry) {
    #[cfg(not(test))]
    {
        if let Err(e) = persist_registry_inner(registry) {
            app_warn!(
                "browser",
                "extension_registry",
                "failed to persist browser registry: {}",
                e
            );
        }
    }
    #[cfg(test)]
    {
        let _ = registry;
    }
}

#[cfg(not(test))]
fn persist_registry_inner(registry: &Registry) -> Result<()> {
    let path = crate::paths::browser_extension_registry_path()?;
    let snapshot = snapshot_from_registry(registry, now_unix_secs());
    let bytes = serde_json::to_vec_pretty(&snapshot)?;
    // Offload the fsync off the registry mutex: serialize under the caller's
    // lock (cheap), then write on a blocking thread so a slow disk doesn't
    // serialize every other session's claim/finalize behind the lock. The
    // registry file is a drop-rebuildable cache, so a last-writer outcome under
    // burst writes is acceptable.
    spawn_registry_write(path, bytes);
    Ok(())
}

#[cfg(not(test))]
fn spawn_registry_write(path: std::path::PathBuf, bytes: Vec<u8>) {
    let write = move || {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = crate::platform::write_atomic(&path, &bytes) {
            app_warn!(
                "browser",
                "extension_registry",
                "failed to persist browser registry: {}",
                e
            );
        }
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn_blocking(write);
        }
        Err(_) => write(),
    }
}

fn snapshot_from_registry(registry: &Registry, saved_at: i64) -> PersistedRegistry {
    let mut scopes = Vec::new();
    for (scope, state) in &registry.scopes {
        let mut tabs = state
            .controlled_tabs
            .iter()
            .map(|(tab_id, tab)| PersistedTabLease {
                tab_id: *tab_id,
                owner_kind: tab.owner_kind,
                lease_expires_at: tab.lease_expires_at,
                last_seen_url: tab.url.clone(),
                last_seen_title: tab.title.clone(),
            })
            .collect::<Vec<_>>();
        if tabs.is_empty() {
            continue;
        }
        tabs.sort_by_key(|tab| tab.tab_id);
        scopes.push(PersistedScope {
            scope: scope.clone(),
            active_tab_id: state.active_tab_id,
            tabs,
        });
    }
    scopes.sort_by(|a, b| a.scope.cmp(&b.scope));
    PersistedRegistry {
        version: 1,
        saved_at,
        scopes,
    }
}

fn registry_from_snapshot(snapshot: PersistedRegistry, now: i64) -> Registry {
    let mut registry = Registry::default();
    for persisted_scope in snapshot.scopes {
        let mut state = ScopeState::default();
        for tab in persisted_scope.tabs {
            if tab.lease_expires_at <= now {
                continue;
            }
            state.controlled_tabs.insert(
                tab.tab_id,
                ControlledTab {
                    owner_kind: tab.owner_kind,
                    url: tab.last_seen_url,
                    title: tab.last_seen_title,
                    lease_expires_at: tab.lease_expires_at,
                },
            );
        }
        state.active_tab_id = persisted_scope
            .active_tab_id
            .filter(|tab_id| state.controlled_tabs.contains_key(tab_id));
        if !state.controlled_tabs.is_empty() {
            registry.scopes.insert(persisted_scope.scope, state);
        }
    }
    registry
}

fn ensure_not_controlled_by_other(registry: &Registry, scope: &str, tab_id: i64) -> Result<()> {
    for (other_scope, state) in &registry.scopes {
        if other_scope != scope && state.controlled_tabs.contains_key(&tab_id) {
            bail!(
                "Chrome tab {} is already controlled by another Hope session. Release or finalize it before claiming.",
                tab_id
            );
        }
    }
    Ok(())
}

fn steal_from_other_scopes(registry: &mut Registry, scope: &str, tab_id: i64) -> Vec<String> {
    let mut stolen = Vec::new();
    for (other_scope, state) in registry.scopes.iter_mut() {
        if other_scope == scope {
            continue;
        }
        if state.controlled_tabs.remove(&tab_id).is_some() {
            stolen.push(other_scope.clone());
            if state.active_tab_id == Some(tab_id) {
                state.active_tab_id = None;
                state.element_refs.clear();
                state.snapshot_url = None;
            }
        }
    }
    stolen
}

pub(super) fn active_tab_id(ctx: &BrowserBackendContext) -> Option<i64> {
    let scope = scope_key(ctx);
    lock_registry()
        .scopes
        .get(&scope)
        .and_then(|state| state.active_tab_id)
}

pub(super) fn controlled_kind(ctx: &BrowserBackendContext, tab_id: i64) -> Option<TabOwnerKind> {
    let scope = scope_key(ctx);
    lock_registry()
        .scopes
        .get(&scope)
        .and_then(|state| state.controlled_tabs.get(&tab_id))
        .map(|tab| tab.owner_kind)
}

pub(super) fn claim_user_tab(
    ctx: &BrowserBackendContext,
    tab_id: i64,
    url: Option<String>,
    title: Option<String>,
    steal: bool,
) -> Result<ClaimOutcome> {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    let stolen_from = if steal {
        steal_from_other_scopes(&mut registry, &scope, tab_id)
    } else {
        ensure_not_controlled_by_other(&registry, &scope, tab_id)?;
        Vec::new()
    };
    let state = registry.scopes.entry(scope).or_default();
    // Preserve existing Agent ownership: selecting/claiming a tab the agent
    // already created (via tabs.new) must not downgrade it to User, or turn-end
    // finalize would leave it open — `close` only fires for Agent-owned tabs —
    // instead of closing the agent's own tab.
    let owner_kind = match state.controlled_tabs.get(&tab_id) {
        Some(existing) if existing.owner_kind == TabOwnerKind::Agent => TabOwnerKind::Agent,
        _ => TabOwnerKind::User,
    };
    state
        .controlled_tabs
        .insert(tab_id, new_controlled_tab(owner_kind, url, title));
    state.active_tab_id = Some(tab_id);
    state.element_refs.clear();
    state.snapshot_url = None;
    persist_registry(&registry);
    Ok(ClaimOutcome { stolen_from })
}

pub(super) fn record_agent_tab(
    ctx: &BrowserBackendContext,
    tab_id: i64,
    url: Option<String>,
    title: Option<String>,
) -> Result<()> {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    ensure_not_controlled_by_other(&registry, &scope, tab_id)?;
    let state = registry.scopes.entry(scope).or_default();
    state
        .controlled_tabs
        .insert(tab_id, new_controlled_tab(TabOwnerKind::Agent, url, title));
    state.active_tab_id = Some(tab_id);
    state.element_refs.clear();
    state.snapshot_url = None;
    persist_registry(&registry);
    Ok(())
}

pub(super) fn activate_controlled_tab(ctx: &BrowserBackendContext, tab_id: i64) -> Result<()> {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    let state = registry.scopes.entry(scope).or_default();
    if !state.controlled_tabs.contains_key(&tab_id) {
        bail!(
            "Chrome tab {} is not controlled by this Hope session. Use tabs.claim first.",
            tab_id
        );
    }
    state.active_tab_id = Some(tab_id);
    state.element_refs.clear();
    state.snapshot_url = None;
    persist_registry(&registry);
    Ok(())
}

pub(super) fn update_snapshot_refs(
    ctx: &BrowserBackendContext,
    tab_id: i64,
    refs: Vec<ElementLocator>,
    url: String,
) -> Result<()> {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    let state = registry.scopes.entry(scope).or_default();
    if !state.controlled_tabs.contains_key(&tab_id) {
        bail!(
            "Chrome tab {} is not controlled by this Hope session. Use tabs.claim or tabs.new first.",
            tab_id
        );
    }
    state.active_tab_id = Some(tab_id);
    state.element_refs = refs;
    state.snapshot_url = Some(url);
    if let Some(tab) = state.controlled_tabs.get_mut(&tab_id) {
        tab.url = Some(state.snapshot_url.clone().unwrap_or_default());
        tab.lease_expires_at = now_unix_secs().saturating_add(LEASE_TTL_SECS);
    }
    persist_registry(&registry);
    Ok(())
}

pub(super) fn clear_refs(ctx: &BrowserBackendContext) {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    if let Some(state) = registry.scopes.get_mut(&scope) {
        state.element_refs.clear();
        state.snapshot_url = None;
        persist_registry(&registry);
    }
}

pub(super) fn selector_for_ref(ctx: &BrowserBackendContext, ref_id: u32) -> Result<ElementLocator> {
    let scope = scope_key(ctx);
    lock_registry()
        .scopes
        .get(&scope)
        .and_then(|state| state.element_refs.iter().find(|r| r.ref_id == ref_id))
        .cloned()
        .ok_or_else(|| anyhow!("Element ref={} not found. Take a fresh snapshot.", ref_id))
}

pub(super) fn find_ref_by_role_text(
    ctx: &BrowserBackendContext,
    role: &str,
    text: &str,
) -> Option<ElementLocator> {
    let scope = scope_key(ctx);
    let needle = text.trim();
    let registry = lock_registry();
    let refs = &registry.scopes.get(&scope)?.element_refs;
    refs.iter()
        .find(|r| r.role == role && r.text.trim() == needle)
        .or_else(|| {
            refs.iter().find(|r| {
                r.role == role
                    && !needle.is_empty()
                    && (r.text.contains(needle) || needle.contains(r.text.trim()))
            })
        })
        .cloned()
}

pub(super) fn release_tab(ctx: &BrowserBackendContext, tab_id: i64) -> Result<TabOwnerKind> {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    let state = registry.scopes.entry(scope).or_default();
    let Some(tab) = state.controlled_tabs.remove(&tab_id) else {
        bail!(
            "Chrome tab {} is not controlled by this Hope session. Nothing was released.",
            tab_id
        );
    };
    if state.active_tab_id == Some(tab_id) {
        state.active_tab_id = None;
        state.element_refs.clear();
        state.snapshot_url = None;
    }
    persist_registry(&registry);
    Ok(tab.owner_kind)
}

pub(super) fn remove_closed_tab(ctx: &BrowserBackendContext, tab_id: i64) {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    if let Some(state) = registry.scopes.get_mut(&scope) {
        state.controlled_tabs.remove(&tab_id);
        if state.active_tab_id == Some(tab_id) {
            state.active_tab_id = None;
            state.element_refs.clear();
            state.snapshot_url = None;
        }
        persist_registry(&registry);
    }
}

pub(super) fn remove_tab_from_all_scopes(tab_id: i64) -> Vec<RemovedTabLease> {
    let mut removed = Vec::new();
    let mut registry = lock_registry();
    for (scope, state) in registry.scopes.iter_mut() {
        if let Some(tab) = state.controlled_tabs.remove(&tab_id) {
            if state.active_tab_id == Some(tab_id) {
                state.active_tab_id = None;
                state.element_refs.clear();
                state.snapshot_url = None;
            }
            removed.push(RemovedTabLease {
                tab_id,
                scope: scope.clone(),
                owner_kind: tab.owner_kind,
            });
        }
    }
    if !removed.is_empty() {
        persist_registry(&registry);
    }
    removed
}

pub(super) fn reconcile_live_tabs(live_tab_ids: &HashSet<i64>) -> Vec<RemovedTabLease> {
    let mut removed = Vec::new();
    let mut registry = lock_registry();
    for (scope, state) in registry.scopes.iter_mut() {
        let stale_tab_ids = state
            .controlled_tabs
            .keys()
            .copied()
            .filter(|tab_id| !live_tab_ids.contains(tab_id))
            .collect::<Vec<_>>();
        for tab_id in stale_tab_ids {
            if let Some(tab) = state.controlled_tabs.remove(&tab_id) {
                if state.active_tab_id == Some(tab_id) {
                    state.active_tab_id = None;
                    state.element_refs.clear();
                    state.snapshot_url = None;
                }
                removed.push(RemovedTabLease {
                    tab_id,
                    scope: scope.clone(),
                    owner_kind: tab.owner_kind,
                });
            }
        }
        if state.controlled_tabs.is_empty() {
            state.active_tab_id = None;
            state.element_refs.clear();
            state.snapshot_url = None;
        }
    }
    registry
        .scopes
        .retain(|_, state| !state.controlled_tabs.is_empty());
    if !removed.is_empty() {
        persist_registry(&registry);
    }
    removed
}

pub(super) fn finalize_scope(
    ctx: &BrowserBackendContext,
    keep: &HashSet<i64>,
) -> Vec<FinalizeTabAction> {
    let scope = scope_key(ctx);
    let mut registry = lock_registry();
    let Some(mut state) = registry.scopes.remove(&scope) else {
        return Vec::new();
    };
    let actions = state
        .controlled_tabs
        .drain()
        .map(|(tab_id, tab)| FinalizeTabAction {
            tab_id,
            owner_kind: tab.owner_kind,
            close: tab.owner_kind == TabOwnerKind::Agent && !keep.contains(&tab_id),
        })
        .collect();
    persist_registry(&registry);
    actions
}

pub(super) fn finalize_all_scopes() -> Vec<ScopedFinalizeTabAction> {
    let mut registry = lock_registry();
    if registry.scopes.is_empty() {
        return Vec::new();
    }

    let scopes = std::mem::take(&mut registry.scopes);
    let mut actions = Vec::new();
    for (scope, mut state) in scopes {
        actions.extend(state.controlled_tabs.drain().map(|(tab_id, tab)| {
            ScopedFinalizeTabAction {
                scope: scope.clone(),
                action: FinalizeTabAction {
                    tab_id,
                    owner_kind: tab.owner_kind,
                    close: tab.owner_kind == TabOwnerKind::Agent,
                },
            }
        }));
    }
    persist_registry(&registry);
    actions
}

#[cfg(test)]
pub(super) fn reset_for_tests() {
    lock_registry().scopes.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn ctx(session_id: &str) -> BrowserBackendContext {
        BrowserBackendContext {
            session_id: Some(session_id.to_string()),
            ..BrowserBackendContext::default()
        }
    }

    #[test]
    fn claim_is_mutually_exclusive_across_sessions() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        claim_user_tab(&ctx("a"), 42, None, None, false).unwrap();
        let err = claim_user_tab(&ctx("b"), 42, None, None, false).unwrap_err();
        assert!(err.to_string().contains("already controlled"));
    }

    #[test]
    fn claim_can_explicitly_steal_from_another_session() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let a = ctx("a");
        let b = ctx("b");
        claim_user_tab(&a, 42, None, None, false).unwrap();
        update_snapshot_refs(
            &a,
            42,
            vec![ElementLocator {
                ref_id: 1,
                role: "button".to_string(),
                text: "Submit".to_string(),
                selector: "#submit".to_string(),
            }],
            "https://example.test".to_string(),
        )
        .unwrap();

        let outcome = claim_user_tab(&b, 42, None, None, true).unwrap();
        assert_eq!(outcome.stolen_from, vec!["session:a".to_string()]);
        assert_eq!(active_tab_id(&a), None);
        assert_eq!(active_tab_id(&b), Some(42));
        assert!(selector_for_ref(&a, 1).is_err());
    }

    #[test]
    fn finalize_closes_agent_tabs_and_releases_user_tabs() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let ctx = ctx("a");
        claim_user_tab(&ctx, 10, None, None, false).unwrap();
        record_agent_tab(&ctx, 11, None, None).unwrap();
        record_agent_tab(&ctx, 12, None, None).unwrap();

        let keep = HashSet::from([12]);
        let mut actions = finalize_scope(&ctx, &keep);
        actions.sort_by_key(|a| a.tab_id);

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].tab_id, 10);
        assert!(!actions[0].close);
        assert_eq!(actions[1].tab_id, 11);
        assert!(actions[1].close);
        assert_eq!(actions[2].tab_id, 12);
        assert!(!actions[2].close);
        assert!(active_tab_id(&ctx).is_none());
    }

    #[test]
    fn remove_tab_from_all_scopes_clears_matching_leases() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let a = ctx("a");
        let b = ctx("b");
        claim_user_tab(&a, 10, None, None, false).unwrap();
        record_agent_tab(&b, 11, None, None).unwrap();

        let removed = remove_tab_from_all_scopes(10);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].tab_id, 10);
        assert_eq!(removed[0].scope, "session:a");
        assert_eq!(removed[0].owner_kind, TabOwnerKind::User);
        assert_eq!(active_tab_id(&a), None);
        assert_eq!(active_tab_id(&b), Some(11));
    }

    #[test]
    fn find_ref_by_role_text_matches_exact_then_fuzzy() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let ctx = ctx("a");
        claim_user_tab(&ctx, 10, None, None, false).unwrap();
        update_snapshot_refs(
            &ctx,
            10,
            vec![
                ElementLocator {
                    ref_id: 1,
                    role: "button".to_string(),
                    text: "Submit order".to_string(),
                    selector: "#submit".to_string(),
                },
                ElementLocator {
                    ref_id: 2,
                    role: "textbox".to_string(),
                    text: "Email".to_string(),
                    selector: "#email".to_string(),
                },
            ],
            "https://example.test".to_string(),
        )
        .unwrap();

        let exact = find_ref_by_role_text(&ctx, "button", "Submit order").unwrap();
        assert_eq!(exact.selector, "#submit");

        let fuzzy = find_ref_by_role_text(&ctx, "button", "Submit").unwrap();
        assert_eq!(fuzzy.selector, "#submit");

        assert!(find_ref_by_role_text(&ctx, "link", "Submit").is_none());
    }

    #[test]
    fn persisted_snapshot_excludes_ephemeral_refs() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let ctx = ctx("persist");
        claim_user_tab(
            &ctx,
            10,
            Some("https://example.test".to_string()),
            Some("Example".to_string()),
            false,
        )
        .unwrap();
        update_snapshot_refs(
            &ctx,
            10,
            vec![ElementLocator {
                ref_id: 99,
                role: "button".to_string(),
                text: "Submit".to_string(),
                selector: "#submit".to_string(),
            }],
            "https://example.test/form".to_string(),
        )
        .unwrap();

        let registry = lock_registry();
        let snapshot = snapshot_from_registry(&registry, 1000);
        assert_eq!(snapshot.scopes.len(), 1);
        assert_eq!(snapshot.scopes[0].active_tab_id, Some(10));
        assert_eq!(snapshot.scopes[0].tabs.len(), 1);
        assert_eq!(
            snapshot.scopes[0].tabs[0].last_seen_url.as_deref(),
            Some("https://example.test/form")
        );
        drop(registry);
        reset_for_tests();
    }

    #[test]
    fn registry_load_prunes_expired_leases_and_invalid_active_tab() {
        let snapshot = PersistedRegistry {
            version: 1,
            saved_at: 100,
            scopes: vec![PersistedScope {
                scope: "session:a".to_string(),
                active_tab_id: Some(1),
                tabs: vec![
                    PersistedTabLease {
                        tab_id: 1,
                        owner_kind: TabOwnerKind::User,
                        lease_expires_at: 99,
                        last_seen_url: None,
                        last_seen_title: None,
                    },
                    PersistedTabLease {
                        tab_id: 2,
                        owner_kind: TabOwnerKind::Agent,
                        lease_expires_at: 200,
                        last_seen_url: Some("https://example.test".to_string()),
                        last_seen_title: Some("Example".to_string()),
                    },
                ],
            }],
        };

        let registry = registry_from_snapshot(snapshot, 100);
        let state = registry.scopes.get("session:a").unwrap();
        assert_eq!(state.active_tab_id, None);
        assert!(!state.controlled_tabs.contains_key(&1));
        assert!(state.controlled_tabs.contains_key(&2));
    }

    #[test]
    fn finalize_all_scopes_releases_every_registered_tab() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let a = ctx("a");
        let b = ctx("b");
        claim_user_tab(&a, 10, None, None, false).unwrap();
        record_agent_tab(&b, 11, None, None).unwrap();

        let mut actions = finalize_all_scopes();
        actions.sort_by_key(|action| action.action.tab_id);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].scope, "session:a");
        assert_eq!(actions[0].action.tab_id, 10);
        assert!(!actions[0].action.close);
        assert_eq!(actions[1].scope, "session:b");
        assert_eq!(actions[1].action.tab_id, 11);
        assert!(actions[1].action.close);
        assert!(active_tab_id(&a).is_none());
        assert!(active_tab_id(&b).is_none());
    }

    #[test]
    fn reconcile_live_tabs_prunes_stale_leases() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let a = ctx("a");
        let b = ctx("b");
        claim_user_tab(&a, 10, None, None, false).unwrap();
        record_agent_tab(&a, 11, None, None).unwrap();
        record_agent_tab(&b, 12, None, None).unwrap();

        let mut removed = reconcile_live_tabs(&HashSet::from([12]));
        removed.sort_by_key(|lease| lease.tab_id);

        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].tab_id, 10);
        assert_eq!(removed[0].scope, "session:a");
        assert_eq!(removed[1].tab_id, 11);
        assert_eq!(removed[1].scope, "session:a");
        assert!(active_tab_id(&a).is_none());
        assert_eq!(active_tab_id(&b), Some(12));
    }
}
