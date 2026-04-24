//! MCP subsystem error taxonomy.
//!
//! Classification matters: the LLM-facing tool wrapper and the GUI status
//! panel both branch on the variant. Keep the set tight; if you find yourself
//! adding a new variant for every new code path, fold it into an existing one.

use std::fmt;

#[derive(Debug)]
pub enum McpError {
    /// Server is configured but not currently connected (disabled, idle,
    /// connecting, in backoff, or unknown server_id). Recoverable — the
    /// manager or the user can bring it back up.
    NotReady { server: String, reason: String },
    /// Transport-level failure (stdio pipe broken, HTTP 5xx, WS close, DNS,
    /// TLS). The underlying error is preserved as a string because rmcp and
    /// our own io errors don't share a common trait object.
    Transport { server: String, source: String },
    /// MCP-level protocol / JSON-RPC error returned by the server.
    Protocol {
        server: String,
        code: Option<i32>,
        message: String,
    },
    /// OAuth / credential problem — token missing, refresh failed, 401.
    /// Triggers the `NeedsAuth` state so the GUI can prompt re-auth.
    Auth { server: String, message: String },
    /// A single tool call exceeded `call_timeout_secs`.
    Timeout {
        server: String,
        tool: String,
        secs: u64,
    },
    /// The tool returned `isError=true` in its result content.
    ToolFailed {
        server: String,
        tool: String,
        message: String,
    },
    /// SSRF / trust policy blocked the transport URL before connecting.
    Blocked { server: String, reason: String },
    /// Configuration invariant violation (duplicate name, malformed URL,
    /// empty command, etc.). Should be caught at save time but the runtime
    /// path re-validates defensively.
    Config(String),
}

impl McpError {
    /// Short stable slug for logging source fields: `mcp`,`<server>:<slug>`.
    pub fn kind(&self) -> &'static str {
        match self {
            McpError::NotReady { .. } => "not_ready",
            McpError::Transport { .. } => "transport",
            McpError::Protocol { .. } => "protocol",
            McpError::Auth { .. } => "auth",
            McpError::Timeout { .. } => "timeout",
            McpError::ToolFailed { .. } => "tool_failed",
            McpError::Blocked { .. } => "blocked",
            McpError::Config(_) => "config",
        }
    }

    /// Server name the error is attributed to, if any. `None` for pure
    /// config errors that predate server registration.
    pub fn server(&self) -> Option<&str> {
        match self {
            McpError::NotReady { server, .. }
            | McpError::Transport { server, .. }
            | McpError::Protocol { server, .. }
            | McpError::Auth { server, .. }
            | McpError::Timeout { server, .. }
            | McpError::ToolFailed { server, .. }
            | McpError::Blocked { server, .. } => Some(server.as_str()),
            McpError::Config(_) => None,
        }
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpError::NotReady { server, reason } => {
                write!(f, "MCP server '{server}' not ready: {reason}")
            }
            McpError::Transport { server, source } => {
                write!(f, "MCP server '{server}' transport error: {source}")
            }
            McpError::Protocol {
                server,
                code,
                message,
            } => match code {
                Some(c) => write!(f, "MCP server '{server}' protocol error {c}: {message}"),
                None => write!(f, "MCP server '{server}' protocol error: {message}"),
            },
            McpError::Auth { server, message } => {
                write!(f, "MCP server '{server}' auth error: {message}")
            }
            McpError::Timeout { server, tool, secs } => {
                write!(f, "MCP tool '{server}::{tool}' timed out after {secs}s")
            }
            McpError::ToolFailed {
                server,
                tool,
                message,
            } => {
                write!(f, "MCP tool '{server}::{tool}' failed: {message}")
            }
            McpError::Blocked { server, reason } => {
                write!(f, "MCP server '{server}' blocked: {reason}")
            }
            McpError::Config(msg) => write!(f, "MCP config error: {msg}"),
        }
    }
}

impl std::error::Error for McpError {}

pub type McpResult<T> = std::result::Result<T, McpError>;
