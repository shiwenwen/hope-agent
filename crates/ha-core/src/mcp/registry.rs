//! MCP registry — central state & orchestration for connected servers.
//!
//! The registry owns every live connection. It is initialized once per
//! process via [`McpManager::init_global`]; after that, the rest of the
//! codebase reaches it through [`McpManager::global`]. All mutation paths
//! (reconcile-from-config, reconnect, shutdown) go through this single
//! owner to avoid split-brain between transport state and catalog state.
//!
//! See `docs/architecture/mcp.md` for the full state machine.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwap;
use rmcp::service::RunningService;
use rmcp::{model, RoleClient};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, Semaphore};

use super::config::{McpGlobalSettings, McpServerConfig};
use super::errors::McpResult;
use crate::tools::ToolDefinition;

// ── Server State ─────────────────────────────────────────────────

/// Lifecycle state for a single MCP server. `Ready` embeds the catalog
/// snapshot taken at list time; subsequent `tools/list_changed`
/// notifications replace the snapshot in place without leaving `Ready`.
///
/// The `String` discriminant names returned by [`ServerState::label`] are
/// part of the EventBus contract and used by the frontend to color the
/// status dot; don't rename them casually.
#[derive(Debug, Default)]
pub enum ServerState {
    /// Config has `enabled=false`. No connection attempts are made; the
    /// tools don't appear in the catalog.
    Disabled,
    /// Enabled but not yet connected. First tool call (or the warm-up
    /// path for `eager=true`) transitions to `Connecting`.
    #[default]
    Idle,
    /// Handshake in progress. Tool calls queue briefly on a per-server
    /// Notify (bounded by `connect_timeout_secs`).
    Connecting,
    /// Connection established and catalog populated.
    Ready {
        tools: Vec<model::Tool>,
        resources: Vec<model::Resource>,
        prompts: Vec<model::Prompt>,
    },
    /// OAuth failed or expired; the GUI prompts re-auth. The URL is the
    /// most recently generated PKCE authorize endpoint.
    NeedsAuth { auth_url: String },
    /// Last connect attempt failed. `retry_at` is the unix ts the watchdog
    /// may try again (exponential backoff). User-triggered reconnect
    /// bypasses the wait.
    Failed { reason: String, retry_at: i64 },
}

impl ServerState {
    /// Short stable slug surfaced to the frontend: `disabled` / `idle` /
    /// `connecting` / `ready` / `needsAuth` / `failed`.
    pub fn label(&self) -> &'static str {
        match self {
            ServerState::Disabled => "disabled",
            ServerState::Idle => "idle",
            ServerState::Connecting => "connecting",
            ServerState::Ready { .. } => "ready",
            ServerState::NeedsAuth { .. } => "needsAuth",
            ServerState::Failed { .. } => "failed",
        }
    }

    /// Compact human string describing *why* we're in this state (e.g. the
    /// failure reason). Returns `None` for terminal-friendly states.
    pub fn reason(&self) -> Option<&str> {
        match self {
            ServerState::Failed { reason, .. } => Some(reason.as_str()),
            ServerState::NeedsAuth { auth_url } => Some(auth_url.as_str()),
            _ => None,
        }
    }
}

/// Snapshot returned to the frontend / `get_settings(category="mcp")`.
/// Decoupled from [`ServerState`] so the live struct can hold rmcp types
/// that aren't serde-friendly (like `model::Tool` with its raw schema).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusSnapshot {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport_kind: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub tool_count: usize,
    pub resource_count: usize,
    pub prompt_count: usize,
    pub consecutive_failures: u32,
    pub last_health_check_ts: i64,
}

// ── Server Handle ────────────────────────────────────────────────

/// Per-server live runtime state. Wrapped in an `Arc` so the watchdog
/// task, the invoke path, and the CRUD endpoints can all hold references
/// without fighting over ownership.
pub struct ServerHandle {
    /// Last-known config. Replaced atomically on reconcile when the user
    /// edits non-connection-critical fields (description, allowed_tools,
    /// etc.). Transport-critical changes trigger disconnect + rebuild.
    pub config: RwLock<McpServerConfig>,
    pub state: Mutex<ServerState>,
    /// None until the handshake completes. The rmcp `RunningService`
    /// holds the spawned service loop + cancellation token.
    pub client: Mutex<Option<RunningService<RoleClient, ()>>>,
    /// Serializes connection attempts for this server. Without this,
    /// concurrent first-use tool calls can all observe `Idle` / `Connecting`
    /// and spawn duplicate subprocesses or handshakes.
    pub connect_lock: Mutex<()>,
    /// Set when this handle has been removed or replaced by a config
    /// reconcile. In-flight work may still hold an Arc, but it must not
    /// reconnect or publish catalogs back into the global manager.
    retired: AtomicBool,
    /// Per-server in-flight cap. Initialized from
    /// `config.max_concurrent_calls` at construction time; callers take a
    /// permit around each `call_tool`.
    pub semaphore: Arc<Semaphore>,
    /// Incremented on every health-check failure or transport error.
    /// Reset to 0 on a successful `ping` / tool call.
    pub consecutive_failures: AtomicU32,
    /// Unix ts of the last health check (success or failure). Used by
    /// the GUI status panel + watchdog spacing.
    pub last_health_check_ts: AtomicI64,
}

impl ServerHandle {
    pub fn new(config: McpServerConfig) -> Self {
        let permits = config.max_concurrent_calls.max(1) as usize;
        Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            consecutive_failures: AtomicU32::new(0),
            last_health_check_ts: AtomicI64::new(0),
            config: RwLock::new(config),
            state: Mutex::new(ServerState::Idle),
            client: Mutex::new(None),
            connect_lock: Mutex::new(()),
            retired: AtomicBool::new(false),
        }
    }

    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }

    pub fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    /// Clone the rmcp `Peer<RoleClient>` handle used for RPCs
    /// (`tools/call`, `resources/read`, `prompts/get`, `ping`). Returns
    /// [`McpError::NotReady`] when the server hasn't finished its
    /// handshake. Callers should grab the peer, drop the client mutex,
    /// then await the RPC so a slow RPC doesn't block concurrent
    /// readers of the same handle.
    pub async fn peer(&self) -> super::errors::McpResult<rmcp::Peer<RoleClient>> {
        if let Some(running) = self.client.lock().await.as_ref() {
            return Ok(running.peer().clone());
        }
        let server = self.config.read().await.name.clone();
        Err(super::errors::McpError::NotReady {
            server,
            reason: "not connected".into(),
        })
    }

    /// Minimal serializable snapshot for frontends / settings dumps.
    /// Clones only the counts — the raw `Tool` vec stays inside the Mutex.
    pub async fn snapshot(&self) -> ServerStatusSnapshot {
        let cfg = self.config.read().await.clone();
        let state = self.state.lock().await;
        let (tool_count, resource_count, prompt_count) = match &*state {
            ServerState::Ready {
                tools,
                resources,
                prompts,
            } => (tools.len(), resources.len(), prompts.len()),
            _ => (0, 0, 0),
        };
        ServerStatusSnapshot {
            id: cfg.id.clone(),
            name: cfg.name.clone(),
            enabled: cfg.enabled,
            transport_kind: cfg.transport.kind_label().to_string(),
            state: state.label().to_string(),
            reason: state.reason().map(|s| s.to_string()),
            tool_count,
            resource_count,
            prompt_count,
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            last_health_check_ts: self.last_health_check_ts.load(Ordering::Relaxed),
        }
    }
}

// ── Tool Index Entry ─────────────────────────────────────────────

/// Reverse mapping from the namespaced tool name `mcp__<server>__<tool>`
/// back to the owning server and the original MCP tool name. Populated
/// after every `tools/list` refresh; consulted on every dispatch.
#[derive(Debug, Clone)]
pub struct ToolIndexEntry {
    pub server_id: String,
    pub server_name: String,
    pub original_tool_name: String,
}

// ── Manager ──────────────────────────────────────────────────────

static MANAGER: OnceLock<McpManager> = OnceLock::new();

/// Global MCP subsystem handle. Constructed once at app start; every
/// reader uses [`McpManager::global`].
pub struct McpManager {
    pub(crate) servers: RwLock<HashMap<String /* server_id */, Arc<ServerHandle>>>,
    pub(crate) tool_index: RwLock<HashMap<String /* mcp__name__tool */, ToolIndexEntry>>,
    /// Sync-readable snapshot of every namespaced MCP tool's
    /// [`ToolDefinition`]. Rebuilt by the catalog-refresh path whenever a
    /// server's tool list changes. Callers on the synchronous tool-schema
    /// assembly path (which can't await) read this via `ArcSwap::load()`.
    pub(crate) cached_tool_defs: ArcSwap<Vec<ToolDefinition>>,
    /// Global cross-server cap; enforced on top of per-server semaphore.
    pub(crate) global_semaphore: Arc<Semaphore>,
    /// Actual semaphore capacity after live growth. We do not shrink a tokio
    /// semaphore in place, so this tracks the real high-water capacity rather
    /// than the latest configured value.
    pub(crate) global_capacity: AtomicU32,
    pub(crate) global_settings: RwLock<McpGlobalSettings>,
}

impl McpManager {
    /// One-shot initializer. Called from `src-tauri/src/lib.rs::run()` and
    /// `crates/ha-server/src/main.rs` before any tool dispatch runs.
    /// No-op after first call; safe to call from multiple entry points.
    pub fn init_global(global: McpGlobalSettings, servers: Vec<McpServerConfig>) -> &'static Self {
        MANAGER.get_or_init(|| {
            let permits = global.max_concurrent_calls.max(1) as usize;
            let mut map = HashMap::new();
            for cfg in servers {
                if !server_effectively_enabled(&global, &cfg) {
                    continue;
                }
                if cfg.validate().is_err() {
                    crate::app_warn!(
                        "mcp",
                        "init",
                        "Skipping invalid server config: name={}",
                        cfg.name
                    );
                    continue;
                }
                map.insert(cfg.id.clone(), Arc::new(ServerHandle::new(cfg)));
            }
            Self {
                servers: RwLock::new(map),
                tool_index: RwLock::new(HashMap::new()),
                cached_tool_defs: ArcSwap::from_pointee(Vec::new()),
                global_semaphore: Arc::new(Semaphore::new(permits)),
                global_capacity: AtomicU32::new(global.max_concurrent_calls.max(1)),
                global_settings: RwLock::new(global),
            }
        })
    }

    /// Read the global singleton. Returns `None` before `init_global` ran
    /// (e.g. early unit tests). Every caller must tolerate the `None`
    /// branch gracefully — MCP is always an *add-on* capability; the rest
    /// of the app must keep working without it.
    pub fn global() -> Option<&'static Self> {
        MANAGER.get()
    }

    /// Fetch a handle by server id.
    pub async fn get_by_id(&self, id: &str) -> Option<Arc<ServerHandle>> {
        self.servers.read().await.get(id).cloned()
    }

    /// Fetch a handle by server name (the `mcp__<name>__...` part).
    pub async fn get_by_name(&self, name: &str) -> Option<Arc<ServerHandle>> {
        let servers = self.servers.read().await;
        for handle in servers.values() {
            let cfg = handle.config.read().await;
            if cfg.name == name {
                return Some(handle.clone());
            }
        }
        None
    }

    /// Resolve the reverse tool-name map for dispatch.
    pub async fn lookup_tool(&self, namespaced_name: &str) -> Option<ToolIndexEntry> {
        self.tool_index.read().await.get(namespaced_name).cloned()
    }

    /// Best-effort server lookup: id first, then name. Used by the
    /// `mcp_resource` / `mcp_prompt` tool handlers so the LLM can
    /// reference a server by either form. Note: a server whose `name`
    /// happens to equal another server's UUID would shadow — prevented
    /// by the `^[a-z0-9_-]{1,32}$` name validator (UUIDs are 36 chars
    /// with hyphens, never valid server names).
    pub async fn locate(&self, name_or_id: &str) -> Option<Arc<ServerHandle>> {
        if let Some(h) = self.get_by_id(name_or_id).await {
            return Some(h);
        }
        self.get_by_name(name_or_id).await
    }

    /// Snapshots of every registered server, for the settings panel /
    /// `get_settings(category="mcp")`. Clones the handle Arcs up front
    /// so we don't hold `servers` read lock while awaiting each
    /// per-server snapshot — a slow `state.lock()` would otherwise
    /// block every concurrent reader.
    pub async fn snapshot_all(&self) -> Vec<ServerStatusSnapshot> {
        let handles: Vec<Arc<ServerHandle>> = {
            let servers = self.servers.read().await;
            servers.values().cloned().collect()
        };
        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            out.push(handle.snapshot().await);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// True iff the MCP subsystem is globally enabled.
    pub async fn is_enabled(&self) -> bool {
        self.global_settings.read().await.enabled
    }

    /// Read a clone of the current global settings.
    pub async fn settings(&self) -> McpGlobalSettings {
        self.global_settings.read().await.clone()
    }

    /// Sync snapshot of every namespaced MCP tool's [`ToolDefinition`].
    /// Cheap to call — returns an `Arc` view of the current cache. The
    /// cache is updated atomically by `client::refresh_catalog` via
    /// `store_cached_tool_defs`.
    pub fn mcp_tool_definitions(&self) -> Arc<Vec<ToolDefinition>> {
        self.cached_tool_defs.load_full()
    }

    /// Replace the entire tool-definition cache. Called by
    /// `client::refresh_catalog` after a `tools/list` round completes.
    pub(crate) fn store_cached_tool_defs(&self, defs: Vec<ToolDefinition>) {
        self.cached_tool_defs.store(Arc::new(defs));
    }

    /// Compare the live server set with a freshly-loaded config and
    /// minimally rebuild: add new effective servers, remove disabled /
    /// denied / deleted entries, and replace config on unchanged ids.
    /// Existing transport state is kept for still-effective servers; removed
    /// servers are disconnected after the registry lock is released.
    ///
    /// Transport / credential / trust / concurrency edits replace the live
    /// handle so old connections and semaphores cannot linger after save.
    pub async fn reconcile(
        &self,
        new_settings: McpGlobalSettings,
        new_servers: Vec<McpServerConfig>,
    ) -> McpResult<()> {
        // Update global settings first — the semaphore is only grown,
        // never shrunk live (shrinking would starve in-flight calls).
        {
            let actual_permits = self.global_capacity.load(Ordering::Acquire).max(1) as usize;
            let new_permits = new_settings.max_concurrent_calls.max(1) as usize;
            if new_permits > actual_permits {
                self.global_semaphore
                    .add_permits(new_permits - actual_permits);
                self.global_capacity
                    .store(new_settings.max_concurrent_calls.max(1), Ordering::Release);
            }
            let mut g = self.global_settings.write().await;
            *g = new_settings.clone();
        }

        let mut removed = Vec::new();
        let active_handles = {
            let mut servers = self.servers.write().await;
            let mut active_ids: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for cfg in new_servers {
                if !server_effectively_enabled(&new_settings, &cfg) {
                    if let Some(handle) = servers.remove(&cfg.id) {
                        handle.retire();
                        removed.push(handle);
                    }
                    continue;
                }
                if cfg.validate().is_err() {
                    crate::app_warn!(
                        "mcp",
                        "reconcile",
                        "Skipping invalid config: name={}",
                        cfg.name
                    );
                    if let Some(handle) = servers.remove(&cfg.id) {
                        handle.retire();
                        removed.push(handle);
                    }
                    continue;
                }

                active_ids.insert(cfg.id.clone());
                if let Some(existing) = servers.get(&cfg.id).cloned() {
                    let old_cfg = existing.config.read().await.clone();
                    if connection_rebuild_required(&old_cfg, &cfg) {
                        let fresh = Arc::new(ServerHandle::new(cfg.clone()));
                        servers.insert(cfg.id.clone(), fresh);
                        existing.retire();
                        removed.push(existing);
                    } else {
                        // Replace the config; state/client stay. Ready catalogs
                        // below are immediately re-indexed against this new config
                        // so allow/deny/deferred edits do not leave stale runtime
                        // visibility.
                        *existing.config.write().await = cfg;
                    }
                } else {
                    servers.insert(cfg.id.clone(), Arc::new(ServerHandle::new(cfg)));
                }
            }

            let stale_ids: Vec<String> = servers
                .keys()
                .filter(|id| !active_ids.contains(*id))
                .cloned()
                .collect();
            for id in stale_ids {
                if let Some(handle) = servers.remove(&id) {
                    handle.retire();
                    removed.push(handle);
                }
            }

            servers.values().cloned().collect::<Vec<_>>()
        };

        for handle in removed {
            let _ = super::client::disconnect(&handle).await;
        }

        self.rebuild_ready_catalog_cache(active_handles).await;

        super::events::emit_servers_changed();
        Ok(())
    }

    async fn rebuild_ready_catalog_cache(&self, handles: Vec<Arc<ServerHandle>>) {
        let mut next_index = HashMap::new();
        let mut next_defs = Vec::new();

        for handle in handles {
            let cfg = handle.config.read().await.clone();
            let tools = {
                let state = handle.state.lock().await;
                match &*state {
                    ServerState::Ready { tools, .. } => tools.clone(),
                    _ => continue,
                }
            };

            let namespaced_names = super::catalog::assign_namespaced_tool_names(
                &cfg.name,
                tools.iter().map(|t| t.name.as_ref()),
            );
            for (tool, namespaced) in tools.iter().zip(namespaced_names) {
                let original = tool.name.to_string();
                if !super::catalog::tool_allowed_by_server_config(&cfg, &original) {
                    continue;
                }

                next_index.insert(
                    namespaced.clone(),
                    ToolIndexEntry {
                        server_id: cfg.id.clone(),
                        server_name: cfg.name.clone(),
                        original_tool_name: original,
                    },
                );
                next_defs.push(super::catalog::rmcp_tool_to_definition_with_name(
                    &cfg, &tool, namespaced,
                ));
            }
        }

        next_defs.sort_by(|a, b| a.name.cmp(&b.name));
        *self.tool_index.write().await = next_index;
        self.cached_tool_defs.store(Arc::new(next_defs));
    }
}

fn server_effectively_enabled(global: &McpGlobalSettings, cfg: &McpServerConfig) -> bool {
    global.enabled && cfg.enabled && !global.denied_servers.contains(&cfg.name)
}

fn connection_rebuild_required(old: &McpServerConfig, new: &McpServerConfig) -> bool {
    old.transport != new.transport
        || old.env != new.env
        || old.headers != new.headers
        || old.oauth != new.oauth
        || old.trust_level != new.trust_level
        || old.max_concurrent_calls.max(1) != new.max_concurrent_calls.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::config::{McpServerConfig, McpTransportSpec, McpTrustLevel};

    fn sample_stdio_cfg(id: &str, name: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            name: name.into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        }
    }

    fn sample_tool(name: &str) -> model::Tool {
        model::Tool::new(
            name.to_string(),
            format!("{name} tool"),
            std::sync::Arc::new(serde_json::Map::new()),
        )
    }

    fn test_manager(configs: Vec<McpServerConfig>) -> McpManager {
        let global = McpGlobalSettings::default();
        let mut map = HashMap::new();
        for cfg in configs {
            map.insert(cfg.id.clone(), Arc::new(ServerHandle::new(cfg)));
        }
        McpManager {
            servers: RwLock::new(map),
            tool_index: RwLock::new(HashMap::new()),
            cached_tool_defs: ArcSwap::from_pointee(Vec::new()),
            global_semaphore: Arc::new(Semaphore::new(global.max_concurrent_calls.max(1) as usize)),
            global_capacity: AtomicU32::new(global.max_concurrent_calls.max(1)),
            global_settings: RwLock::new(global),
        }
    }

    async fn seed_ready_catalog(manager: &McpManager, id: &str, tools: Vec<model::Tool>) {
        let handle = manager.get_by_id(id).await.expect("server exists");
        *handle.state.lock().await = ServerState::Ready {
            tools,
            resources: vec![],
            prompts: vec![],
        };
        manager.rebuild_ready_catalog_cache(vec![handle]).await;
    }

    fn cached_tool_names(manager: &McpManager) -> Vec<String> {
        manager
            .mcp_tool_definitions()
            .iter()
            .map(|d| d.name.clone())
            .collect()
    }

    #[tokio::test]
    async fn server_handle_snapshot_idle_state() {
        let h = ServerHandle::new(sample_stdio_cfg("id-1", "alpha"));
        let snap = h.snapshot().await;
        assert_eq!(snap.state, "idle");
        assert_eq!(snap.tool_count, 0);
        assert_eq!(snap.transport_kind, "stdio");
    }

    #[test]
    fn server_state_labels_stable() {
        // Frontend depends on these strings — guard them.
        assert_eq!(ServerState::Disabled.label(), "disabled");
        assert_eq!(ServerState::Idle.label(), "idle");
        assert_eq!(ServerState::Connecting.label(), "connecting");
        assert_eq!(
            ServerState::Ready {
                tools: vec![],
                resources: vec![],
                prompts: vec![]
            }
            .label(),
            "ready"
        );
        assert_eq!(
            ServerState::NeedsAuth {
                auth_url: "x".into()
            }
            .label(),
            "needsAuth"
        );
        assert_eq!(
            ServerState::Failed {
                reason: "x".into(),
                retry_at: 0
            }
            .label(),
            "failed"
        );
    }

    #[tokio::test]
    async fn reconcile_reindexes_ready_catalog_after_allow_change() {
        let mut cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        seed_ready_catalog(
            &manager,
            "id-alpha",
            vec![sample_tool("read"), sample_tool("write")],
        )
        .await;
        assert!(manager.lookup_tool("mcp__alpha__write").await.is_some());

        cfg.allowed_tools = vec!["read".into()];
        manager
            .reconcile(McpGlobalSettings::default(), vec![cfg])
            .await
            .unwrap();

        assert_eq!(cached_tool_names(&manager), vec!["mcp__alpha__read"]);
        assert!(manager.lookup_tool("mcp__alpha__read").await.is_some());
        assert!(manager.lookup_tool("mcp__alpha__write").await.is_none());
    }

    #[tokio::test]
    async fn ready_catalog_uses_collision_safe_tool_names() {
        let cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        seed_ready_catalog(
            &manager,
            "id-alpha",
            vec![sample_tool("foo-bar"), sample_tool("foo.bar")],
        )
        .await;

        assert_eq!(
            cached_tool_names(&manager),
            vec!["mcp__alpha__foo_bar", "mcp__alpha__foo_bar_2"]
        );
        assert_eq!(
            manager
                .lookup_tool("mcp__alpha__foo_bar")
                .await
                .unwrap()
                .original_tool_name,
            "foo-bar"
        );
        assert_eq!(
            manager
                .lookup_tool("mcp__alpha__foo_bar_2")
                .await
                .unwrap()
                .original_tool_name,
            "foo.bar"
        );
    }

    #[tokio::test]
    async fn reconcile_rebuilds_handle_for_connection_critical_edits() {
        let mut cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        seed_ready_catalog(&manager, "id-alpha", vec![sample_tool("read")]).await;
        let old_handle = manager.get_by_id("id-alpha").await.unwrap();
        assert!(manager.lookup_tool("mcp__alpha__read").await.is_some());

        cfg.env.insert("TOKEN".into(), "new-token".into());
        manager
            .reconcile(McpGlobalSettings::default(), vec![cfg.clone()])
            .await
            .unwrap();

        let handle = manager.get_by_id("id-alpha").await.unwrap();
        assert!(old_handle.is_retired());
        assert_eq!(handle.snapshot().await.state, "idle");
        assert_eq!(handle.config.read().await.env["TOKEN"], "new-token");
        assert!(manager.mcp_tool_definitions().is_empty());
        assert!(manager.lookup_tool("mcp__alpha__read").await.is_none());
    }

    #[tokio::test]
    async fn reconcile_does_not_wait_for_retired_connect_lock() {
        let cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        let old_handle = manager.get_by_id("id-alpha").await.unwrap();
        let _connect_guard = old_handle.connect_lock.lock().await;

        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            manager.reconcile(McpGlobalSettings::default(), vec![]),
        )
        .await
        .expect("reconcile should not wait for a retired handle's connect lock")
        .unwrap();

        assert!(old_handle.is_retired());
        assert!(manager.get_by_id("id-alpha").await.is_none());
    }

    #[tokio::test]
    async fn global_semaphore_does_not_grow_after_shrink_then_partial_raise() {
        let cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        assert_eq!(manager.global_semaphore.available_permits(), 8);

        manager
            .reconcile(
                McpGlobalSettings {
                    max_concurrent_calls: 4,
                    ..Default::default()
                },
                vec![cfg.clone()],
            )
            .await
            .unwrap();
        assert_eq!(manager.global_capacity.load(Ordering::Acquire), 8);
        assert_eq!(manager.global_semaphore.available_permits(), 8);

        manager
            .reconcile(
                McpGlobalSettings {
                    max_concurrent_calls: 6,
                    ..Default::default()
                },
                vec![cfg.clone()],
            )
            .await
            .unwrap();
        assert_eq!(manager.global_capacity.load(Ordering::Acquire), 8);
        assert_eq!(manager.global_semaphore.available_permits(), 8);

        manager
            .reconcile(
                McpGlobalSettings {
                    max_concurrent_calls: 10,
                    ..Default::default()
                },
                vec![cfg],
            )
            .await
            .unwrap();
        assert_eq!(manager.global_capacity.load(Ordering::Acquire), 10);
        assert_eq!(manager.global_semaphore.available_permits(), 10);
    }

    #[tokio::test]
    async fn reconcile_removes_denied_server_and_catalog_entries() {
        let cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        seed_ready_catalog(&manager, "id-alpha", vec![sample_tool("read")]).await;

        let global = McpGlobalSettings {
            denied_servers: vec!["alpha".into()],
            ..Default::default()
        };
        manager.reconcile(global, vec![cfg]).await.unwrap();

        assert!(manager.get_by_id("id-alpha").await.is_none());
        assert!(manager.mcp_tool_definitions().is_empty());
        assert!(manager.lookup_tool("mcp__alpha__read").await.is_none());
    }

    #[tokio::test]
    async fn reconcile_global_disable_clears_catalog_and_reenable_starts_idle() {
        let cfg = sample_stdio_cfg("id-alpha", "alpha");
        let manager = test_manager(vec![cfg.clone()]);
        seed_ready_catalog(&manager, "id-alpha", vec![sample_tool("read")]).await;

        let disabled = McpGlobalSettings {
            enabled: false,
            ..Default::default()
        };
        manager
            .reconcile(disabled, vec![cfg.clone()])
            .await
            .unwrap();

        assert!(manager.get_by_id("id-alpha").await.is_none());
        assert!(manager.mcp_tool_definitions().is_empty());

        manager
            .reconcile(McpGlobalSettings::default(), vec![cfg])
            .await
            .unwrap();

        let handle = manager
            .get_by_id("id-alpha")
            .await
            .expect("server restored");
        assert_eq!(handle.snapshot().await.state, "idle");
        assert!(manager.mcp_tool_definitions().is_empty());
        assert!(manager.lookup_tool("mcp__alpha__read").await.is_none());
    }
}
