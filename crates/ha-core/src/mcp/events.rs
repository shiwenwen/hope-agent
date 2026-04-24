//! MCP → frontend event names and helpers.
//!
//! Event names are part of the cross-process contract with the Tauri webview
//! and the HTTP/WS server — treat them as stable once shipped. New event
//! types get a new name; don't repurpose old ones.

use serde_json::json;

/// Server connection / lifecycle state changed (Disabled / Idle / Connecting
/// / Ready / NeedsAuth / Failed). Payload:
/// `{ id, name, state, reason? }`.
pub const EV_SERVER_STATUS_CHANGED: &str = "mcp:server_status_changed";

/// A server's tool/resource/prompt catalog was refreshed. Payload:
/// `{ id, name, tools, resources, prompts }` (all counts).
pub const EV_CATALOG_REFRESHED: &str = "mcp:catalog_refreshed";

/// OAuth flow is required. Payload: `{ id, name, auth_url }`.
pub const EV_AUTH_REQUIRED: &str = "mcp:auth_required";

/// OAuth flow completed. Payload: `{ id, name, ok: bool, error? }`.
pub const EV_AUTH_COMPLETED: &str = "mcp:auth_completed";

/// Full server list shape changed (added / removed / reordered / renamed).
/// No payload needed; consumers call `mcp_list_servers()` for the new list.
pub const EV_SERVERS_CHANGED: &str = "mcp:servers_changed";

/// A server emitted a log line (merged stderr + internal lifecycle).
/// Payload: `{ id, name, level: "info"|"warn"|"error", line }`.
/// Log panel streams this for live tail; also gets persisted through the
/// standard `app_*!` macros for cold storage.
pub const EV_SERVER_LOG: &str = "mcp:server_log";

/// Helper: emit a server status event through the global bus (no-op if the
/// bus isn't initialized yet — e.g. during unit tests).
pub fn emit_server_status(id: &str, name: &str, state: &str, reason: Option<&str>) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            EV_SERVER_STATUS_CHANGED,
            json!({
                "id": id,
                "name": name,
                "state": state,
                "reason": reason,
            }),
        );
    }
}

/// Helper: emit the "servers list shape changed" event. Call after CRUD.
pub fn emit_servers_changed() {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(EV_SERVERS_CHANGED, json!({}));
    }
}

/// Helper: emit a catalog-refreshed summary after `tools/list` + friends.
pub fn emit_catalog_refreshed(
    id: &str,
    name: &str,
    tools: usize,
    resources: usize,
    prompts: usize,
) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            EV_CATALOG_REFRESHED,
            json!({
                "id": id,
                "name": name,
                "tools": tools,
                "resources": resources,
                "prompts": prompts,
            }),
        );
    }
}

/// Helper: OAuth authorize URL ready.
pub fn emit_auth_required(id: &str, name: &str, auth_url: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            EV_AUTH_REQUIRED,
            json!({
                "id": id,
                "name": name,
                "authUrl": auth_url,
            }),
        );
    }
}

/// Helper: OAuth flow concluded.
pub fn emit_auth_completed(id: &str, name: &str, ok: bool, error: Option<&str>) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            EV_AUTH_COMPLETED,
            json!({
                "id": id,
                "name": name,
                "ok": ok,
                "error": error,
            }),
        );
    }
}
