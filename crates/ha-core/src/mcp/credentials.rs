//! Per-server OAuth credential storage.
//!
//! Tokens live at `~/.hope-agent/credentials/mcp/{server_id}.json` with
//! file mode `0600` on Unix (best-effort ACL on Windows). The file is
//! written via [`crate::platform::write_secure_file`] which creates the
//! directory, writes to a temp file, and atomically renames into place.
//!
//! Shape is intentionally minimal — just enough to reconnect the
//! networked transport with a valid `Authorization: Bearer ...` header.
//! We deliberately avoid rmcp's `StoredCredentials` because that type
//! carries the `oauth2` crate's non-spec fields which aren't needed
//! for plain bearer-token reuse.

use std::fs;
use std::io;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{mcp_credential_path, mcp_credentials_dir};
use crate::platform::write_secure_file;

/// Persisted OAuth credentials for a single MCP server. Populated at the
/// end of the PKCE flow and rewritten on each refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCredentials {
    /// OAuth client id — either user-provided or assigned via Dynamic
    /// Client Registration at first authorization.
    pub client_id: String,
    /// Optional OAuth client secret (most public MCP servers use PKCE
    /// without a secret and will leave this `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Active bearer token.
    pub access_token: String,
    /// Refresh token, if the server issued one. Long-running MCP sessions
    /// depend on this — without it the user has to re-authorize when the
    /// access token expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds when `access_token` expires. `0` means the server did
    /// not specify a TTL — we treat that as "never refresh proactively".
    #[serde(default)]
    pub expires_at: i64,
    /// Resolved token endpoint (captured during discovery; used by the
    /// refresh path so we don't rediscover every time).
    pub token_endpoint: String,
    /// Resolved authorization endpoint (kept around so the GUI can
    /// surface "re-authorize" without a second discovery hit).
    pub authorization_endpoint: String,
    /// Scopes the server granted (may differ from what we asked for).
    #[serde(default)]
    pub granted_scopes: Vec<String>,
    /// Seconds since epoch when this record was last written.
    #[serde(default)]
    pub issued_at: i64,
}

impl McpCredentials {
    /// True iff the current access token should be refreshed proactively.
    /// A 60-second safety margin keeps us from racing with the server's
    /// own clock skew tolerance.
    pub fn needs_refresh(&self) -> bool {
        if self.expires_at == 0 {
            return false;
        }
        let now = chrono::Utc::now().timestamp();
        self.expires_at.saturating_sub(now) < 60
    }
}

/// Load credentials for a server id. Returns `Ok(None)` when the file
/// is absent (the normal "not authorized yet" path) — only real I/O or
/// parse failures surface as `Err`. Single syscall; race-free against
/// a concurrent refresh writer (no exists-then-read window).
pub fn load(server_id: &str) -> Result<Option<McpCredentials>> {
    let path = mcp_credential_path(server_id)?;
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow!("read {}: {e}", path.display())),
    };
    let creds: McpCredentials =
        serde_json::from_slice(&bytes).map_err(|e| anyhow!("parse {}: {e}", path.display()))?;
    Ok(Some(creds))
}

/// Atomically persist credentials for a server id. Overwrites any prior
/// file in place.
pub fn save(server_id: &str, creds: &McpCredentials) -> Result<()> {
    let path = mcp_credential_path(server_id)?;
    let bytes =
        serde_json::to_vec_pretty(creds).map_err(|e| anyhow!("serialize credentials: {e}"))?;
    write_secure_file(&path, &bytes).map_err(|e| anyhow!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Delete credentials on the user's behalf — called when they click
/// "Sign out" or when the owning server is removed from config. Missing
/// file is not an error.
pub fn clear(server_id: &str) -> Result<()> {
    let path = mcp_credential_path(server_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("remove {}: {e}", path.display())),
    }
}

/// Ensure the parent directory exists. Called lazily before the first
/// write. Safe to call repeatedly.
pub fn ensure_dir() -> Result<()> {
    let dir = mcp_credentials_dir()?;
    fs::create_dir_all(&dir).map_err(|e| anyhow!("create {}: {e}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_refresh_respects_zero_expiry() {
        let c = McpCredentials {
            client_id: "cid".into(),
            client_secret: None,
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: 0,
            token_endpoint: "https://x/oauth/token".into(),
            authorization_endpoint: "https://x/oauth/authorize".into(),
            granted_scopes: vec![],
            issued_at: 0,
        };
        assert!(!c.needs_refresh());
    }

    #[test]
    fn needs_refresh_detects_near_expiry() {
        let mut c = McpCredentials {
            client_id: "cid".into(),
            client_secret: None,
            access_token: "tok".into(),
            refresh_token: Some("r".into()),
            expires_at: chrono::Utc::now().timestamp() + 30,
            token_endpoint: "https://x/oauth/token".into(),
            authorization_endpoint: "https://x/oauth/authorize".into(),
            granted_scopes: vec![],
            issued_at: 0,
        };
        assert!(c.needs_refresh());
        c.expires_at = chrono::Utc::now().timestamp() + 600;
        assert!(!c.needs_refresh());
    }
}
