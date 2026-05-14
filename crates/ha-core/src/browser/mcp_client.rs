//! Internal stdio MCP client used exclusively by [`super::mcp_backend`] to
//! drive a private `chrome-devtools-mcp` subprocess.
//!
//! This **does not** participate in the user's [`AppConfig.mcp_servers`]
//! ([`crate::mcp::config`]) — we construct a one-off `McpServerConfig` in
//! memory, hand it to [`crate::mcp::transport::build_stdio_client`], and
//! keep the resulting [`ConnectedClient`] inside the active backend. None
//! of it leaks into the LLM-visible MCP catalog.
//!
//! Lifecycle: the spawned `npx … chrome-devtools-mcp` process is reaped
//! automatically when the [`rmcp::service::RunningService`] is dropped
//! (rmcp closes the stdio pipes; the `TokioChildProcess` it wraps was built
//! with `kill_on_drop`). Wiring through
//! [`super::backend_select::reset_backend`] is therefore the canonical way
//! to terminate the subprocess.

use std::time::Duration;

use anyhow::{anyhow, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::timeout;

use crate::mcp::config::{McpServerConfig, McpTransportSpec, McpTrustLevel};
use crate::mcp::transport::{build_stdio_client, ConnectedClient};

use super::mcp_backend::{CHROME_DEVTOOLS_MCP_FEATURE_ARGS, CHROME_DEVTOOLS_MCP_VERSION_SPEC};

/// Cap the first-run `npx -y chrome-devtools-mcp@latest` pull. On a clean
/// machine npm can take 10–30s to download + install; anything beyond 60s
/// is treated as a hang and the backend falls back to CDP.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(60);

/// Build a one-off [`McpServerConfig`] targeted at `npx … chrome-devtools-mcp
/// --browserUrl <browser_url>`. The name fits the `^[a-z0-9_-]{1,32}$`
/// invariant in case it ever gets logged through MCP error paths.
fn chrome_devtools_mcp_cfg(browser_url: &str) -> McpServerConfig {
    let mut args = vec![
        "-y".to_string(),
        CHROME_DEVTOOLS_MCP_VERSION_SPEC.to_string(),
        "--browserUrl".to_string(),
        browser_url.to_string(),
    ];
    args.extend(CHROME_DEVTOOLS_MCP_FEATURE_ARGS.iter().map(|s| (*s).into()));

    McpServerConfig {
        // Sentinel id — this config never enters `AppConfig.mcp_servers`,
        // so the id only surfaces in transport-level error messages.
        id: "ha-internal-chrome-devtools-mcp".into(),
        name: "internal-chrome-devtools-mcp".into(),
        enabled: true,
        transport: McpTransportSpec::Stdio {
            command: "npx".into(),
            args,
            cwd: None,
        },
        env: Default::default(),
        headers: Default::default(),
        oauth: None,
        allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        connect_timeout_secs: 60,
        call_timeout_secs: 120,
        health_check_interval_secs: 60,
        max_concurrent_calls: 4,
        auto_approve: false,
        trust_level: McpTrustLevel::Trusted,
        eager: false,
        deferred_tools: false,
        project_paths: Vec::new(),
        description: None,
        icon: None,
        created_at: 0,
        updated_at: 0,
        trust_acknowledged_at: None,
    }
}

/// Spawn `npx -y chrome-devtools-mcp@latest --browserUrl <url>` and complete
/// the rmcp handshake. On success the returned [`ConnectedClient`] owns the
/// subprocess; dropping it closes stdio and kills the child.
///
/// A stderr drain task is spawned alongside so a chatty server can't fill
/// its pipe buffer and stall stdout. The drained lines are forwarded to
/// `app_warn!` so first-run npm output / chrome-devtools-mcp warnings show
/// up in the unified log.
pub async fn spawn(browser_url: &str) -> Result<ConnectedClient> {
    let cfg = chrome_devtools_mcp_cfg(browser_url);
    let mut connected = match timeout(SPAWN_TIMEOUT, build_stdio_client(&cfg)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(anyhow!("chrome-devtools-mcp spawn failed: {e}")),
        Err(_) => {
            return Err(anyhow!(
                "chrome-devtools-mcp spawn timed out after {}s (npx may still be pulling the package)",
                SPAWN_TIMEOUT.as_secs()
            ));
        }
    };

    if let Some(stderr) = connected.stderr.take() {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                app_warn!("browser", "chrome_devtools_mcp", "stderr: {}", line);
            }
        });
    }

    Ok(connected)
}
