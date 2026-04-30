//! MCP server configuration schema.
//!
//! All types here are pure serde — the runtime state (connection, catalog,
//! retry counters) lives in `registry.rs`. Keep this file free of rmcp
//! imports so the config layer can be deserialized in contexts where the
//! runtime isn't initialized (e.g. unit tests, `oc-settings` read path).

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
/// transports where the server advertises OAuth; stdio transports ignore it.
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
    120
}

fn default_health_check_interval_secs() -> u64 {
    60
}

fn default_per_server_max_concurrent_calls() -> u32 {
    4
}

pub(crate) fn default_true() -> bool {
    true
}

// ── Server Config ────────────────────────────────────────────────

/// One entry in `AppConfig.mcp_servers`. Persisted to `config.json`.
///
/// Validation (done at save time — see `validate()` below):
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
    #[serde(default = "default_true")]
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
    /// Eager-connect at app startup. Defaults to lazy (connect on first
    /// tool call).
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

impl McpServerConfig {
    /// Returns `Err(McpError::Config(..))` on any invariant violation. Called
    /// at save time by the settings panel / import path; `McpManager::init`
    /// re-runs it defensively on each entry so legacy data can be quarantined.
    pub fn validate(&self) -> crate::mcp::errors::McpResult<()> {
        use crate::mcp::errors::McpError;
        if !is_valid_name(&self.name) {
            return Err(McpError::Config(format!(
                "invalid server name '{}': must match ^[a-z0-9_-]{{1,32}}$",
                self.name
            )));
        }
        if self.id.is_empty() {
            return Err(McpError::Config("server id must not be empty".into()));
        }
        match &self.transport {
            McpTransportSpec::Stdio { command, .. } if command.trim().is_empty() => {
                return Err(McpError::Config(format!(
                    "server '{}': stdio command must not be empty",
                    self.name
                )));
            }
            McpTransportSpec::StreamableHttp { url }
            | McpTransportSpec::Sse { url }
            | McpTransportSpec::WebSocket { url }
                if url.trim().is_empty() =>
            {
                return Err(McpError::Config(format!(
                    "server '{}': transport URL must not be empty",
                    self.name
                )));
            }
            _ => {}
        }
        if self.auto_approve && matches!(self.trust_level, McpTrustLevel::Untrusted) {
            return Err(McpError::Config(format!(
                "server '{}': auto_approve requires trust_level=trusted",
                self.name
            )));
        }
        Ok(())
    }
}

/// Name regex: lowercase letters, digits, underscore, hyphen; 1–32 chars.
/// Hand-rolled to avoid pulling a regex just for one check at save time.
pub fn is_valid_name(s: &str) -> bool {
    let len = s.len();
    if !(1..=32).contains(&len) {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
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
    #[serde(default = "default_true")]
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

// ── Env Placeholder Expansion ────────────────────────────────────

/// Expand `${VAR}` / `$VAR` placeholders in a value string using a lookup.
///
/// Rules (kept narrow on purpose):
/// * `${VAR}` — braced form, always honored.
/// * `$VAR` — unbraced, honored when followed by alphanumerics/underscore.
/// * An unknown variable resolves to the empty string. Callers can detect
///   this by comparing pre/post or by pre-validating keys.
/// * `$$` is an escape for a literal `$`.
///
/// We don't use `std::env::var()` directly — callers pass their own
/// lookup so project-scoped env blocks can override without touching
/// the process environment.
pub fn expand_placeholders<F>(input: &str, mut lookup: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'$' {
            out.push(b as char);
            i += 1;
            continue;
        }
        // `$$` → literal `$`
        if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        // `${...}`
        if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = bytes[i + 2..].iter().position(|&c| c == b'}') {
                let name = &input[i + 2..i + 2 + end];
                if let Some(v) = lookup(name) {
                    out.push_str(&v);
                }
                i += 2 + end + 1;
                continue;
            }
        }
        // `$VAR` (bare)
        let name_start = i + 1;
        let mut name_end = name_start;
        while name_end < bytes.len() {
            let c = bytes[name_end];
            if c.is_ascii_alphanumeric() || c == b'_' {
                name_end += 1;
            } else {
                break;
            }
        }
        if name_end > name_start {
            let name = &input[name_start..name_end];
            if let Some(v) = lookup(name) {
                out.push_str(&v);
            }
            i = name_end;
        } else {
            out.push('$');
            i += 1;
        }
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_regex_accepts_valid() {
        assert!(is_valid_name("a"));
        assert!(is_valid_name("my-server_01"));
        assert!(is_valid_name(&"a".repeat(32)));
    }

    #[test]
    fn name_regex_rejects_invalid() {
        assert!(!is_valid_name(""));
        assert!(!is_valid_name(&"a".repeat(33)));
        assert!(!is_valid_name("Foo")); // uppercase
        assert!(!is_valid_name("with space"));
        assert!(!is_valid_name("dot.separator"));
    }

    #[test]
    fn validate_rejects_auto_approve_on_untrusted() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
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
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: true, // conflict
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_minimal_stdio() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
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
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_command() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
            name: "foo".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "  ".into(),
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
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn expand_braced_placeholders() {
        let s = expand_placeholders("${FOO}/bar/${BAZ}", |k| match k {
            "FOO" => Some("x".into()),
            "BAZ" => Some("y".into()),
            _ => None,
        });
        assert_eq!(s, "x/bar/y");
    }

    #[test]
    fn expand_bare_placeholders() {
        let s = expand_placeholders("$HOME/hope", |k| {
            if k == "HOME" {
                Some("/Users/test".into())
            } else {
                None
            }
        });
        assert_eq!(s, "/Users/test/hope");
    }

    #[test]
    fn expand_escaped_dollar() {
        let s = expand_placeholders("price: $$5 (${X})", |k| {
            if k == "X" {
                Some("five".into())
            } else {
                None
            }
        });
        assert_eq!(s, "price: $5 (five)");
    }

    #[test]
    fn expand_unknown_vars_become_empty() {
        let s = expand_placeholders("hi ${UNDEF}!", |_| None);
        assert_eq!(s, "hi !");
    }

    #[test]
    fn transport_kind_labels() {
        assert_eq!(
            McpTransportSpec::Stdio {
                command: "x".into(),
                args: vec![],
                cwd: None,
            }
            .kind_label(),
            "stdio"
        );
        assert_eq!(
            McpTransportSpec::StreamableHttp {
                url: "https://x".into()
            }
            .kind_label(),
            "http"
        );
        assert_eq!(
            McpTransportSpec::Sse {
                url: "https://x".into()
            }
            .kind_label(),
            "sse"
        );
        assert_eq!(
            McpTransportSpec::WebSocket {
                url: "wss://x".into()
            }
            .kind_label(),
            "ws"
        );
    }

    #[test]
    fn global_settings_default_enabled() {
        let g = McpGlobalSettings::default();
        assert!(g.enabled);
        assert_eq!(g.max_concurrent_calls, 8);
        assert_eq!(g.backoff_initial_secs, 5);
        assert_eq!(g.backoff_max_secs, 300);
    }

    #[test]
    fn deserialize_transport_variants() {
        let stdio: McpTransportSpec =
            serde_json::from_str(r#"{"kind":"stdio","command":"foo","args":["-x"]}"#).unwrap();
        assert!(matches!(stdio, McpTransportSpec::Stdio { .. }));
        let http: McpTransportSpec =
            serde_json::from_str(r#"{"kind":"streamableHttp","url":"https://example.com/mcp"}"#)
                .unwrap();
        assert!(matches!(http, McpTransportSpec::StreamableHttp { .. }));
    }
}
