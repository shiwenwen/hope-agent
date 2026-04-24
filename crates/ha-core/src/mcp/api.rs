//! High-level MCP user commands shared by the Tauri shell and the HTTP server.
//!
//! These are the "what the user clicked in the GUI / hit via REST" entry
//! points. Both the `invoke_handler!` macro in `src-tauri/src/lib.rs` and
//! the axum router in `crates/ha-server/src/routes/mcp.rs` delegate
//! straight through — keeping the business logic in ha-core per the
//! project's three-crate rule.
//!
//! Every function here returns something that serializes cleanly with
//! serde: the Tauri side re-wraps `Result<T, String>` for IPC, the HTTP
//! side wraps `AppError` for status-code mapping.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::config::{cached_config, mutate_config};

use super::catalog::namespaced_tool_name;
use super::client;
use super::config::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};
use super::registry::{McpManager, ServerStatusSnapshot};

// ── Serializable DTOs ────────────────────────────────────────────

/// One row in the MCP settings panel. Joins the persisted config with
/// the live status snapshot so the GUI renders it all in one shot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSummary {
    #[serde(flatten)]
    pub config: McpServerConfig,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub tool_count: usize,
    pub resource_count: usize,
    pub prompt_count: usize,
    pub consecutive_failures: u32,
    pub last_health_check_ts: i64,
}

impl McpServerSummary {
    fn from_parts(config: McpServerConfig, snap: Option<ServerStatusSnapshot>) -> Self {
        let snap = snap.unwrap_or(ServerStatusSnapshot {
            id: config.id.clone(),
            name: config.name.clone(),
            enabled: config.enabled,
            transport_kind: config.transport.kind_label().to_string(),
            state: if config.enabled { "idle" } else { "disabled" }.to_string(),
            reason: None,
            tool_count: 0,
            resource_count: 0,
            prompt_count: 0,
            consecutive_failures: 0,
            last_health_check_ts: 0,
        });
        Self {
            state: snap.state,
            reason: snap.reason,
            tool_count: snap.tool_count,
            resource_count: snap.resource_count,
            prompt_count: snap.prompt_count,
            consecutive_failures: snap.consecutive_failures,
            last_health_check_ts: snap.last_health_check_ts,
            config: redact_for_response(config),
        }
    }
}

/// Strip every field on an [`McpServerConfig`] that could leak a secret
/// when serialized back to the GUI / HTTP client: the OAuth client
/// secret, each env value, and every header value (we keep the keys so
/// the editor shows which variables / headers were configured).
///
/// `#[serde(flatten)]` on `McpServerSummary.config` means any new
/// sensitive field added to `McpServerConfig` rides along automatically;
/// bake the redaction here so that "add field" stays the single mental
/// model for schema evolution.
fn redact_for_response(mut cfg: McpServerConfig) -> McpServerConfig {
    const REDACTED: &str = "<redacted>";
    if let Some(oauth) = cfg.oauth.as_mut() {
        if oauth.client_secret.is_some() {
            oauth.client_secret = Some(REDACTED.into());
        }
    }
    for v in cfg.env.values_mut() {
        *v = REDACTED.into();
    }
    for v in cfg.headers.values_mut() {
        *v = REDACTED.into();
    }
    cfg
}

/// Minimal tool descriptor returned by `list_server_tools` for the GUI's
/// whitelist / blacklist checkbox list. We deliberately drop the raw
/// JSON schema here to keep the payload small; a later `inspect_tool`
/// invoke can return it if the user wants to preview inputs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolSummary {
    pub name: String,
    pub namespaced_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One line of MCP log output surfaced by the per-server log viewer.
/// Pulled live from the AppLogger SQLite store.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLogLine {
    pub ts: i64,
    pub level: String,
    pub source: String,
    pub message: String,
}

/// Result of a Claude Desktop config import. `mcpServers` keys are each
/// tried independently — the per-key errors go back in `skipped` with
/// a reason so the GUI can show them inline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub imported: Vec<String>,
    pub skipped: Vec<ImportSkip>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSkip {
    pub name: String,
    pub reason: String,
}

// ── Shared helpers ───────────────────────────────────────────────

/// Shape the frontend sends when editing — most fields are the same as
/// [`McpServerConfig`] but `id` is assigned by the backend on `add`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerDraft {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    pub transport: McpTransportSpec,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub connect_timeout_secs: Option<u64>,
    #[serde(default)]
    pub call_timeout_secs: Option<u64>,
    #[serde(default)]
    pub health_check_interval_secs: Option<u64>,
    #[serde(default)]
    pub max_concurrent_calls: Option<u32>,
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default)]
    pub trust_level: McpTrustLevel,
    #[serde(default)]
    pub eager: bool,
    #[serde(default)]
    pub project_paths: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub trust_acknowledged_at: Option<String>,
}

impl McpServerDraft {
    /// Bake the draft into a persisted [`McpServerConfig`]. `now_secs` is
    /// injected so the test matrix can freeze time.
    pub fn into_config(self, now_secs: i64, existing: Option<&McpServerConfig>) -> McpServerConfig {
        let id = self
            .id
            .or_else(|| existing.map(|e| e.id.clone()))
            .unwrap_or_else(|| uuid::Uuid::new_v4().as_hyphenated().to_string());
        let created_at = existing.map(|e| e.created_at).unwrap_or(now_secs);
        McpServerConfig {
            id,
            name: self.name,
            enabled: self.enabled,
            transport: self.transport,
            env: self.env,
            headers: self.headers,
            oauth: self.oauth,
            allowed_tools: self.allowed_tools,
            denied_tools: self.denied_tools,
            connect_timeout_secs: self.connect_timeout_secs.unwrap_or(30),
            call_timeout_secs: self.call_timeout_secs.unwrap_or(120),
            health_check_interval_secs: self.health_check_interval_secs.unwrap_or(60),
            max_concurrent_calls: self.max_concurrent_calls.unwrap_or(4),
            auto_approve: self.auto_approve,
            trust_level: self.trust_level,
            eager: self.eager,
            project_paths: self.project_paths,
            description: self.description,
            icon: self.icon,
            created_at,
            updated_at: now_secs,
            trust_acknowledged_at: self.trust_acknowledged_at,
        }
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Read the persisted server list, joined with the live status snapshots.
async fn snapshots_by_id() -> std::collections::HashMap<String, ServerStatusSnapshot> {
    let Some(mgr) = McpManager::global() else {
        return Default::default();
    };
    mgr.snapshot_all()
        .await
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect()
}

async fn reconcile_from_cache() -> Result<()> {
    let Some(mgr) = McpManager::global() else {
        return Ok(());
    };
    let cfg = cached_config();
    mgr.reconcile(cfg.mcp_global.clone(), cfg.mcp_servers.clone())
        .await
        .map_err(|e| anyhow!("{e}"))
}

// ── CRUD commands ────────────────────────────────────────────────

/// List every server in `AppConfig.mcp_servers`, joined with its current
/// live status. Order follows the config array (user-reorderable).
pub async fn list_servers() -> Vec<McpServerSummary> {
    let cfg = cached_config();
    let snaps = snapshots_by_id().await;
    cfg.mcp_servers
        .iter()
        .cloned()
        .map(|c| {
            let snap = snaps.get(&c.id).cloned();
            McpServerSummary::from_parts(c, snap)
        })
        .collect()
}

/// Resolve a single server's live status. Returns `None` when the id
/// is unknown; the HTTP layer maps that to 404.
pub async fn get_server_status(id: &str) -> Option<ServerStatusSnapshot> {
    let mgr = McpManager::global()?;
    let handle = mgr.get_by_id(id).await?;
    Some(handle.snapshot().await)
}

/// Read the top-level global settings (enabled flag, concurrency caps,
/// etc.). Cheap — reads the live cache.
pub fn get_global_settings() -> McpGlobalSettings {
    cached_config().mcp_global.clone()
}

/// Replace the global settings. Triggers a reconcile so the watchdog
/// picks up new backoff / concurrency values on the next tick.
pub async fn update_global_settings(new_settings: McpGlobalSettings) -> Result<()> {
    mutate_config(("mcp.global", "settings_panel"), |cfg| {
        cfg.mcp_global = new_settings.clone();
        Ok(())
    })?;
    reconcile_from_cache().await
}

/// Add a new server. Runs `validate()` before persisting — invalid
/// drafts come back as a 400-shaped anyhow error. Uniqueness on `name`
/// is re-checked **inside** the mutate_config closure so two concurrent
/// add requests can't race past a stale cached snapshot.
pub async fn add_server(draft: McpServerDraft) -> Result<McpServerSummary> {
    let now = now_secs();
    let cfg = draft.into_config(now, None);
    cfg.validate().map_err(|e| anyhow!("{e}"))?;

    let saved = cfg.clone();
    mutate_config(("mcp.add", "settings_panel"), |store| {
        if store.mcp_servers.iter().any(|s| s.name == saved.name) {
            return Err(anyhow!(
                "A server named '{}' already exists; choose a different name",
                saved.name
            ));
        }
        store.mcp_servers.push(saved.clone());
        Ok(())
    })?;
    reconcile_from_cache().await?;
    Ok(McpServerSummary::from_parts(cfg, None))
}

/// Update an existing server in place. Keeps `id` / `created_at` but
/// refreshes `updated_at`.
pub async fn update_server(id: &str, draft: McpServerDraft) -> Result<McpServerSummary> {
    let now = now_secs();
    let existing = cached_config()
        .mcp_servers
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or_else(|| anyhow!("MCP server '{id}' not found"))?;

    let new_cfg = draft.into_config(now, Some(&existing));
    new_cfg.validate().map_err(|e| anyhow!("{e}"))?;

    let saved = new_cfg.clone();
    mutate_config(("mcp.update", "settings_panel"), |store| {
        // Rename + uniqueness re-checked atomically against the live
        // snapshot so two concurrent edits can't collide.
        if store
            .mcp_servers
            .iter()
            .any(|s| s.id != saved.id && s.name == saved.name)
        {
            return Err(anyhow!(
                "A server named '{}' already exists; choose a different name",
                saved.name
            ));
        }
        let Some(slot) = store.mcp_servers.iter_mut().find(|s| s.id == id) else {
            return Err(anyhow!("MCP server '{id}' not found"));
        };
        *slot = saved.clone();
        Ok(())
    })?;
    reconcile_from_cache().await?;
    let snap = snapshots_by_id().await.get(id).cloned();
    Ok(McpServerSummary::from_parts(new_cfg, snap))
}

/// Remove a server by id. Deletes its config entry, triggers reconcile
/// (which shuts down the connection + flushes the tool_index), and
/// (future phase) wipes `~/.hope-agent/credentials/mcp/<id>.json` on
/// disk to avoid orphan credentials.
pub async fn remove_server(id: &str) -> Result<()> {
    mutate_config(("mcp.remove", "settings_panel"), |store| {
        let before = store.mcp_servers.len();
        store.mcp_servers.retain(|s| s.id != id);
        if store.mcp_servers.len() == before {
            return Err(anyhow!("MCP server '{id}' not found"));
        }
        Ok(())
    })?;
    reconcile_from_cache().await
}

/// Reorder the server list to match the supplied id array. Ids not in
/// the target array are appended at the end in their prior order (lets
/// the GUI send a partial reorder).
pub async fn reorder_servers(new_order: Vec<String>) -> Result<()> {
    mutate_config(("mcp.reorder", "settings_panel"), |store| {
        let mut by_id: std::collections::HashMap<String, McpServerConfig> = store
            .mcp_servers
            .drain(..)
            .map(|s| (s.id.clone(), s))
            .collect();
        for id in &new_order {
            if let Some(cfg) = by_id.remove(id) {
                store.mcp_servers.push(cfg);
            }
        }
        // Append any ids not in the reorder payload (defensive against
        // a stale client submitting a truncated list).
        for (_, cfg) in by_id.into_iter() {
            store.mcp_servers.push(cfg);
        }
        Ok(())
    })
}

// ── Connection + diagnostics ─────────────────────────────────────

/// Force a fresh connection + catalog refresh. Used by:
/// * the "Test connection" button on the add/edit dialog
/// * the "Reconnect" action in the server list row
/// Returns the post-attempt status snapshot (state may be `ready`,
/// `failed`, or `needsAuth`).
pub async fn test_connection(id: &str) -> Result<ServerStatusSnapshot> {
    let mgr = McpManager::global().ok_or_else(|| anyhow!("MCP subsystem not initialized"))?;
    let handle = mgr
        .get_by_id(id)
        .await
        .ok_or_else(|| anyhow!("MCP server '{id}' not found"))?;

    // Drop any existing connection first — "test connection" should
    // always exercise the full spawn path, not hit a warm cache.
    client::disconnect(&handle).await.ok();
    let outcome = client::connect_now(mgr, handle.clone()).await;
    let snap = handle.snapshot().await;
    // Swallow the connect error — the snapshot's `state` + `reason`
    // already encode the failure for the GUI.
    let _ = outcome;
    Ok(snap)
}

/// User-triggered reconnect (same wiring as [`test_connection`] but
/// conceptually distinct: called on an already-registered server that
/// the watchdog isn't getting back online, typically from a "Retry"
/// button on the Failed-state badge).
pub async fn reconnect_server(id: &str) -> Result<ServerStatusSnapshot> {
    test_connection(id).await
}

/// Return the up-to-date tool list for a server. If the server is in
/// `Ready` state, the cached tools are returned immediately; otherwise
/// a connect + list round is triggered first (same path as a first tool
/// call), so the GUI can always offer a current whitelist picker.
pub async fn list_server_tools(id: &str) -> Result<Vec<McpToolSummary>> {
    let mgr = McpManager::global().ok_or_else(|| anyhow!("MCP subsystem not initialized"))?;
    let handle = mgr
        .get_by_id(id)
        .await
        .ok_or_else(|| anyhow!("MCP server '{id}' not found"))?;
    client::ensure_connected(mgr, handle.clone())
        .await
        .map_err(|e| anyhow!("{e}"))?;

    let state = handle.state.lock().await;
    let tools = match &*state {
        super::registry::ServerState::Ready { tools, .. } => tools.clone(),
        _ => Vec::new(),
    };
    let cfg_name = handle.config.read().await.name.clone();
    Ok(tools
        .iter()
        .map(|t| {
            let name = t.name.to_string();
            let description = t
                .description
                .as_ref()
                .map(|d| d.to_string())
                .filter(|s| !s.is_empty());
            McpToolSummary {
                namespaced_name: namespaced_tool_name(&cfg_name, &name),
                name,
                description,
            }
        })
        .collect())
}

/// Tail the last `limit` log rows produced by this server, pulled from
/// the shared AppLogger SQLite store. Returns an empty vec when the
/// logger hasn't been initialized (e.g. hermetic tests) or the server
/// id doesn't resolve. Log rows emitted by the MCP subsystem all use
/// `category="mcp"` and `source="<server_name>:<event>"`, so we filter
/// on category + post-filter on a source prefix in Rust (LogFilter
/// doesn't expose a "source starts_with" predicate today).
pub async fn get_recent_logs(id: &str, limit: usize) -> Result<Vec<McpLogLine>> {
    let Some(mgr) = McpManager::global() else {
        return Ok(Vec::new());
    };
    let Some(handle) = mgr.get_by_id(id).await else {
        return Ok(Vec::new());
    };
    let name_prefix = format!("{}:", handle.config.read().await.name);
    let Some(db) = crate::globals::get_log_db() else {
        return Ok(Vec::new());
    };

    // Pull a slightly larger page than `limit` to absorb the other-server
    // rows that share the `mcp` category; clamp tightly afterwards.
    let filter = crate::logging::LogFilter {
        categories: Some(vec!["mcp".to_string()]),
        ..Default::default()
    };
    let page_size = ((limit as u32).saturating_mul(4)).clamp(20, 1000);
    let result = db
        .query(&filter, 0, page_size)
        .map_err(|e| anyhow!("log query failed: {e}"))?;

    let mut out = Vec::with_capacity(limit);
    for row in result.logs {
        if !row.source.starts_with(&name_prefix) {
            continue;
        }
        let ts = chrono::DateTime::parse_from_rfc3339(&row.timestamp)
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        out.push(McpLogLine {
            ts,
            level: row.level,
            source: row.source,
            message: row.message,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

/// Parse a `claude_desktop_config.json`-shaped blob and upsert every
/// `mcpServers` entry as a new hope-agent MCP server. Existing servers
/// matching `name` are skipped (user must delete them first to avoid
/// silent overwrites). Returns per-entry imported / skipped breakdown.
pub async fn import_claude_desktop_config(raw_json: &str) -> Result<ImportSummary> {
    #[derive(Deserialize)]
    struct Outer {
        #[serde(rename = "mcpServers")]
        mcp_servers: BTreeMap<String, ClaudeDesktopServer>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ClaudeDesktopServer {
        Stdio {
            command: String,
            #[serde(default)]
            args: Vec<String>,
            #[serde(default)]
            env: BTreeMap<String, String>,
            #[serde(default)]
            cwd: Option<String>,
        },
        Remote {
            url: String,
            #[serde(default, rename = "type")]
            transport_type: Option<String>,
            #[serde(default)]
            headers: BTreeMap<String, String>,
        },
    }

    let outer: Outer = serde_json::from_str(raw_json).map_err(|e| anyhow!("Invalid JSON: {e}"))?;

    // Step 1 — validate each entry OUTSIDE of `mutate_config`. Name
    // normalization, regex validation, and per-entry config construction
    // are pure functions; doing them up front lets us fail fast per
    // entry and keeps the mutate_config closure short.
    let mut prepared: Vec<(String, McpServerConfig)> = Vec::new();
    let mut skipped = Vec::new();
    let now = now_secs();
    for (raw_name, server) in outer.mcp_servers {
        let name = normalize_name_for_import(&raw_name);
        if !super::config::is_valid_name(&name) {
            skipped.push(ImportSkip {
                name: raw_name.clone(),
                reason: format!("invalid server name '{name}' (must match ^[a-z0-9_-]{{1,32}}$)"),
            });
            continue;
        }
        let (transport, headers, env_map) = match server {
            ClaudeDesktopServer::Stdio {
                command,
                args,
                env,
                cwd,
            } => (
                McpTransportSpec::Stdio { command, args, cwd },
                BTreeMap::new(),
                env,
            ),
            ClaudeDesktopServer::Remote {
                url,
                transport_type,
                headers,
            } => {
                let kind = transport_type
                    .as_deref()
                    .map(str::to_lowercase)
                    .unwrap_or_else(|| "streamableHttp".into());
                let t = match kind.as_str() {
                    "sse" => McpTransportSpec::Sse { url },
                    "ws" | "websocket" => McpTransportSpec::WebSocket { url },
                    _ => McpTransportSpec::StreamableHttp { url },
                };
                (t, headers, BTreeMap::new())
            }
        };
        let draft = McpServerDraft {
            id: None,
            name: name.clone(),
            enabled: true,
            transport,
            env: env_map,
            headers,
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: None,
            call_timeout_secs: None,
            health_check_interval_secs: None,
            max_concurrent_calls: None,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            project_paths: vec![],
            description: Some(format!(
                "Imported from claude_desktop_config.json ({raw_name})"
            )),
            icon: None,
            trust_acknowledged_at: None,
        };
        let cfg = draft.into_config(now, None);
        if let Err(e) = cfg.validate() {
            skipped.push(ImportSkip {
                name: raw_name,
                reason: e.to_string(),
            });
            continue;
        }
        prepared.push((raw_name, cfg));
    }

    if prepared.is_empty() {
        return Ok(ImportSummary {
            imported: Vec::new(),
            skipped,
        });
    }

    // Step 2 — one atomic `mutate_config` call for every valid entry.
    // Collisions against the live list (or among the prepared batch
    // itself) are resolved inside the closure so nothing races past a
    // stale cached snapshot. One reconcile + one `mcp:servers_changed`
    // event at the end regardless of batch size — no storm of per-entry
    // reconciles on a big import.
    let batch = prepared.clone();
    let skipped_from_mutate: Vec<ImportSkip> = mutate_config(
        ("mcp.import", "settings_panel"),
        |store| -> Result<Vec<ImportSkip>> {
            let mut inner_skipped = Vec::new();
            for (raw_name, cfg) in batch {
                if store.mcp_servers.iter().any(|s| s.name == cfg.name) {
                    inner_skipped.push(ImportSkip {
                        name: raw_name,
                        reason: "a server with this name already exists".into(),
                    });
                    continue;
                }
                store.mcp_servers.push(cfg);
            }
            Ok(inner_skipped)
        },
    )?;

    // Figure out which prepared entries made it by diffing against the
    // skip list returned by the mutate closure.
    let skipped_names: std::collections::HashSet<&str> = skipped_from_mutate
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    let imported: Vec<String> = prepared
        .into_iter()
        .filter(|(raw_name, _)| !skipped_names.contains(raw_name.as_str()))
        .map(|(_, cfg)| cfg.name)
        .collect();
    skipped.extend(skipped_from_mutate);

    reconcile_from_cache().await?;
    Ok(ImportSummary { imported, skipped })
}

/// Lowercase + sanitize the key from `mcpServers` so it fits our name
/// regex. "Brave Search" → "brave_search", "MyPostgresDB" → "mypostgresdb".
fn normalize_name_for_import(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(32));
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if c.is_whitespace() {
            out.push('_');
        }
        if out.len() >= 32 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_name_for_import_cases() {
        assert_eq!(normalize_name_for_import("Brave Search"), "brave_search");
        assert_eq!(normalize_name_for_import("my-server.v1"), "my-serverv1");
        assert_eq!(normalize_name_for_import(""), "");
        let long = "x".repeat(100);
        assert_eq!(normalize_name_for_import(&long).len(), 32);
    }

    #[test]
    fn draft_preserves_id_on_update() {
        let draft = McpServerDraft {
            id: None,
            name: "foo".into(),
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
            connect_timeout_secs: None,
            call_timeout_secs: None,
            health_check_interval_secs: None,
            max_concurrent_calls: None,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            project_paths: vec![],
            description: None,
            icon: None,
            trust_acknowledged_at: None,
        };
        let existing = McpServerConfig {
            id: "keep-me".into(),
            name: "old".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "echo".into(),
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
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 100,
            updated_at: 100,
            trust_acknowledged_at: None,
        };
        let cfg = draft.into_config(200, Some(&existing));
        assert_eq!(cfg.id, "keep-me");
        assert_eq!(cfg.created_at, 100);
        assert_eq!(cfg.updated_at, 200);
    }
}
