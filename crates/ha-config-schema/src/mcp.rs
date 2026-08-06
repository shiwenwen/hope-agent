//! MCP server configuration schema (`AppConfig.mcp_servers` / `AppConfig.mcp_global`).
//!
//! All types here are pure serde — the runtime state (connection, catalog,
//! retry counters) lives in ha-core's `mcp/registry.rs`. Keep this file free
//! of rmcp imports so the config layer can be deserialized in contexts where
//! the runtime isn't initialized (e.g. unit tests, `ha-settings` read path).
//!
//! Save-time validation (`validate_server_config`) and the env placeholder
//! expander stay in ha-core's `mcp/config.rs` — they belong to the subsystem,
//! not the schema.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── Transport ────────────────────────────────────────────────────

/// Which wire protocol to use when talking to the server.
///
/// `Stdio` spawns a local child process and frames JSON-RPC over its
/// stdin/stdout pipes. `StreamableHttp` is the spec's preferred remote
/// transport (spec date 2025-03-26). `Sse` is the legacy Server-Sent
/// Events transport kept for compatibility with servers that haven't
/// migrated yet. `WebSocket` is non-spec but several deployments use it;
/// we implement it with a `tokio-tungstenite` wrapper.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum McpTransportSpec {
    /// Local subprocess. `command` is the executable; we do NOT run it
    /// through a shell. Args are passed as a separate argv vector.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// Working directory; `None` means inherit the app's cwd.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// Streamable HTTP (POST + optional GET-SSE on the same URL).
    StreamableHttp { url: String },
    /// Legacy SSE transport. Prefer `StreamableHttp` for new servers.
    Sse { url: String },
    /// WebSocket — custom, matches what claude-code exposes via
    /// `mcpWebSocketTransport`.
    WebSocket { url: String },
}

impl McpTransportSpec {
    /// Human-readable label used by logs and the GUI badge.
    pub fn kind_label(&self) -> &'static str {
        match self {
            McpTransportSpec::Stdio { .. } => "stdio",
            McpTransportSpec::StreamableHttp { .. } => "http",
            McpTransportSpec::Sse { .. } => "sse",
            McpTransportSpec::WebSocket { .. } => "ws",
        }
    }

    /// True iff the transport dials a network endpoint (and therefore must
    /// pass through SSRF + trust gating before `connect()`).
    pub fn is_networked(&self) -> bool {
        !matches!(self, McpTransportSpec::Stdio { .. })
    }
}

// ── Trust Level ──────────────────────────────────────────────────

/// Governs default permissions for the server. `Trusted` is a deliberate
/// acknowledgement by the user that this server is safe to grant auto-approve
/// / relaxed SSRF; it's not a pre-baked allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpTrustLevel {
    /// Default for new servers — every tool call runs through the approval
    /// gate; networked transports use the strict SSRF policy.
    #[default]
    Untrusted,
    /// User has explicitly marked this server as trusted. `auto_approve` may
    /// now be enabled; networked transports use the default SSRF policy.
    Trusted,
}

// ── OAuth ────────────────────────────────────────────────────────

/// Per-server OAuth 2.1 + PKCE configuration. Only populated for networked
/// transports where the server advertises OAuth; stdio transports reject it.
/// The discovered endpoints (`.well-known/oauth-authorization-server`) may
/// override any `None` fields at connect time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthConfig {
    /// Pre-registered OAuth client id. `None` triggers Dynamic Client
    /// Registration (RFC 7591) if the server supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Optional client secret for confidential clients. Most public MCP
    /// servers use PKCE without a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Override the authorization endpoint. `None` → discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    /// Override the token endpoint. `None` → discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    /// Requested scopes. Empty = server default.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Extra parameters forwarded on the authorization request (rare —
    /// e.g. `audience` for some deployments).
    #[serde(default)]
    pub extra_params: BTreeMap<String, String>,
}

// ── Default helpers ──────────────────────────────────────────────

fn default_connect_timeout_secs() -> u64 {
    30
}

fn default_call_timeout_secs() -> u64 {
    0
}

fn default_health_check_interval_secs() -> u64 {
    60
}

fn default_per_server_max_concurrent_calls() -> u32 {
    4
}

// ── Server Config ────────────────────────────────────────────────

/// One entry in `AppConfig.mcp_servers`. Persisted to `config.json`.
///
/// Validation (done at save time — see ha-core's
/// `mcp::config::validate_server_config`):
/// * `name` must match `^[a-z0-9_-]{1,32}$` and be unique inside the list.
/// * `id` must be a UUID v4.
/// * Networked transports must have a non-empty URL; `Stdio` must have a
///   non-empty `command`.
///
/// Note: `allowed_tools` / `denied_tools` refer to the *original* MCP tool
/// name (pre-namespace prefix). Catalog generation prefixes them with
/// `mcp__<server_name>__` before feeding the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    /// Stable UUID v4. Never renamed; used for credential file names and
    /// EventBus payloads. If migrating an old config missing `id`, the
    /// loader assigns a fresh one and writes back.
    pub id: String,
    /// User-visible name — forms the `mcp__<name>__<tool>` namespace.
    /// Immutable after creation (rename requires remove + re-add) to avoid
    /// invalidating references in agent filters / logs.
    pub name: String,
    /// `false` means "disabled — don't connect, don't expose tools".
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    pub transport: McpTransportSpec,
    /// Environment variables injected into the subprocess (stdio) or sent
    /// as headers placeholders (http/sse/ws — keys are case-sensitive).
    /// Values support `${ENV_VAR}` placeholders expanded at connect time.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// HTTP-only: extra request headers. Token-bearing headers (e.g.
    /// `Authorization`) are redacted in logs via `redact_sensitive`.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Optional OAuth config for networked transports. `None` means the
    /// server is either public or expects a pre-baked header token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthConfig>,
    /// Whitelist of *original* MCP tool names. Empty = allow all.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Blacklist of *original* MCP tool names (takes precedence over
    /// `allowed_tools`).
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    /// Per MCP tool-call timeout in seconds. 0 = no call-level timeout.
    #[serde(default = "default_call_timeout_secs")]
    pub call_timeout_secs: u64,
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
    /// Per-server semaphore cap; prevents a single slow server from hogging
    /// the global pool.
    #[serde(default = "default_per_server_max_concurrent_calls")]
    pub max_concurrent_calls: u32,
    /// Opt-in: skip the tool-level approval gate for this server's tools.
    /// Only honored when `trust_level = Trusted` (defense-in-depth).
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default)]
    pub trust_level: McpTrustLevel,
    /// Eager-connect immediately after MCP subsystem initialization. Defaults
    /// to lazy discovery on the first `tool_search` / resource / prompt call.
    #[serde(default)]
    pub eager: bool,
    /// When true, this server's dynamic MCP tools are not sent eagerly in
    /// every LLM request. They remain discoverable via `tool_search`.
    /// Defaults to false: MCP tools are injected eagerly unless the user
    /// explicitly opts this server into deferred loading.
    #[serde(default)]
    pub deferred_tools: bool,
    /// Only active when the current session's project root matches one of
    /// these absolute paths. Empty = active everywhere (global scope).
    #[serde(default)]
    pub project_paths: Vec<String>,
    /// Optional free-form description shown in the GUI + mixed into the
    /// `tool_search` BM25 index. Never injected into the tool schema
    /// (that's what individual tool descriptions are for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional user-chosen icon name (Lucide); frontend falls back to a
    /// default Plug icon when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Seconds since UNIX epoch.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    /// ISO 8601 timestamp of the last time the user ACKed the trust prompt
    /// on the Add Server dialog. Acts as audit trail; absence means the
    /// server predates the prompt and the GUI will re-prompt on next edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_acknowledged_at: Option<String>,
}

// ── Global Settings ──────────────────────────────────────────────

fn default_global_max_concurrent_calls() -> u32 {
    8
}

fn default_backoff_initial_secs() -> u64 {
    5
}

fn default_backoff_max_secs() -> u64 {
    300
}

fn default_consecutive_failure_circuit_breaker() -> u32 {
    10
}

fn default_auto_reconnect_after_circuit_secs() -> u64 {
    1800
}

/// Top-level `AppConfig.mcp_global` — knobs shared by every server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpGlobalSettings {
    /// Master switch. `false` → the manager is never initialized; the
    /// dispatch path short-circuits with `NotReady` before spawning any
    /// connection. Default `true`.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Global cross-server in-flight call cap.
    #[serde(default = "default_global_max_concurrent_calls")]
    pub max_concurrent_calls: u32,
    /// Initial backoff on reconnect. Doubles each failure up to `backoff_max_secs`.
    #[serde(default = "default_backoff_initial_secs")]
    pub backoff_initial_secs: u64,
    #[serde(default = "default_backoff_max_secs")]
    pub backoff_max_secs: u64,
    /// Consecutive failures before tripping the circuit breaker. `0`
    /// disables the breaker (reconnect forever).
    #[serde(default = "default_consecutive_failure_circuit_breaker")]
    pub consecutive_failure_circuit_breaker: u32,
    /// After circuit-breaker trip, how long until we try again on our own
    /// (user can still hit Reconnect manually at any time).
    #[serde(default = "default_auto_reconnect_after_circuit_secs")]
    pub auto_reconnect_after_circuit_secs: u64,
    /// Deny list of server names (policy override; predates addition by
    /// the GUI). Enterprise deployments can ship this pre-populated.
    #[serde(default)]
    pub denied_servers: Vec<String>,
}

impl Default for McpGlobalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent_calls: default_global_max_concurrent_calls(),
            backoff_initial_secs: default_backoff_initial_secs(),
            backoff_max_secs: default_backoff_max_secs(),
            consecutive_failure_circuit_breaker: default_consecutive_failure_circuit_breaker(),
            auto_reconnect_after_circuit_secs: default_auto_reconnect_after_circuit_secs(),
            denied_servers: Vec::new(),
        }
    }
}
