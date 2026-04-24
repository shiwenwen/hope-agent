//! MCP client lifecycle — connect, refresh catalog, disconnect.
//!
//! Callers go through [`ensure_connected`] which is idempotent:
//! `Ready` short-circuits, `Connecting` waits for the in-flight attempt,
//! `Idle` / `Failed` kicks off a fresh handshake. [`connect_now`] is the
//! user-triggered equivalent and bypasses the backoff window.
//!
//! Every connection attempt that succeeds immediately fetches the
//! initial catalog (tools + resources + prompts) and pushes it into the
//! manager's `tool_index`.

use std::sync::Arc;

use rmcp::model;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::MutexGuard;
use tokio::time::{timeout, Duration};

use super::errors::{McpError, McpResult};
use super::events::{emit_catalog_refreshed, emit_server_status};
use super::registry::{McpManager, ServerHandle, ServerState, ToolIndexEntry};
use super::transport::{build_transport_for, ConnectedClient};

/// Idempotent "make sure this server is connected and has a catalog".
/// Returns quickly when already `Ready`; otherwise performs a full
/// connect + list_all_tools + list_all_resources + list_all_prompts
/// round under the configured `connect_timeout_secs`.
pub async fn ensure_connected(manager: &McpManager, handle: Arc<ServerHandle>) -> McpResult<()> {
    // Fast path: already good.
    {
        let state = handle.state.lock().await;
        if matches!(*state, ServerState::Ready { .. }) {
            return Ok(());
        }
        if matches!(*state, ServerState::Disabled) {
            let cfg = handle.config.read().await;
            return Err(McpError::NotReady {
                server: cfg.name.clone(),
                reason: "server is disabled in config".into(),
            });
        }
        if let ServerState::Failed { retry_at, reason } = &*state {
            let now = chrono::Utc::now().timestamp();
            if now < *retry_at {
                let cfg = handle.config.read().await;
                return Err(McpError::NotReady {
                    server: cfg.name.clone(),
                    reason: format!(
                        "in backoff after failure ({reason}); retry_at in {}s",
                        *retry_at - now
                    ),
                });
            }
        }
    }
    connect_now(manager, handle).await
}

/// Force a (re)connect regardless of current state. Used by the user's
/// "Reconnect" button, by the watchdog after a timer tick, and by Phase 3
/// CRUD paths that need immediate visibility after a config change.
pub async fn connect_now(manager: &McpManager, handle: Arc<ServerHandle>) -> McpResult<()> {
    let cfg = handle.config.read().await.clone();
    if !cfg.enabled {
        set_state(&handle, ServerState::Disabled).await;
        return Err(McpError::NotReady {
            server: cfg.name,
            reason: "disabled".into(),
        });
    }
    set_state(&handle, ServerState::Connecting).await;
    emit_server_status(&cfg.id, &cfg.name, "connecting", None);

    let connect_timeout = Duration::from_secs(cfg.connect_timeout_secs.max(1));

    let result = timeout(connect_timeout, do_connect(&cfg, &handle)).await;
    match result {
        Ok(Ok(())) => {
            handle
                .consecutive_failures
                .store(0, std::sync::atomic::Ordering::Relaxed);
            // Refresh catalog right after connect so the tool index is
            // populated for the first dispatch.
            if let Err(e) = refresh_catalog(manager, handle.clone()).await {
                // Connect succeeded but listing failed — mark Failed and
                // drop the connection so a retry can try fresh.
                disconnect(&handle).await.ok();
                record_failure(&handle, &cfg.name, &e).await;
                return Err(e);
            }
            crate::app_info!(
                "mcp",
                &format!("{}:connect", cfg.name),
                "Connected to MCP server '{}' via {}",
                cfg.name,
                cfg.transport.kind_label()
            );
            Ok(())
        }
        Ok(Err(e)) => {
            record_failure(&handle, &cfg.name, &e).await;
            Err(e)
        }
        Err(_elapsed) => {
            let err = McpError::Timeout {
                server: cfg.name.clone(),
                tool: "<connect>".into(),
                secs: cfg.connect_timeout_secs,
            };
            record_failure(&handle, &cfg.name, &err).await;
            Err(err)
        }
    }
}

/// Close the connection if any. Safe to call repeatedly.
pub async fn disconnect(handle: &ServerHandle) -> McpResult<()> {
    let mut client = handle.client.lock().await;
    if let Some(running) = client.take() {
        let _ = running.cancel().await;
    }
    set_state(handle, ServerState::Idle).await;
    Ok(())
}

/// (Re-)fetch tools/resources/prompts on an already-connected server
/// and rebuild the manager's tool index entries for it. The `Ready`
/// catalog snapshot is replaced in place; other servers' entries in
/// the index are untouched.
/// Hard cap on tools per server. A malicious or buggy MCP server could
/// advertise millions of entries via `list_tools`; without a cap, the
/// reverse index + schema cache + `Ready` state's embedded Vec would
/// allocate unbounded memory + every LLM request would spend time
/// filtering the giant list. 512 is generous for any legitimate
/// catalog (the biggest public servers ship ~50).
const TOOLS_PER_SERVER_CAP: usize = 512;

pub async fn refresh_catalog(manager: &McpManager, handle: Arc<ServerHandle>) -> McpResult<()> {
    let cfg = handle.config.read().await.clone();
    let peer_res = {
        let client = handle.client.lock().await;
        match client.as_ref() {
            Some(s) => Ok(s.peer().clone()),
            None => Err(McpError::NotReady {
                server: cfg.name.clone(),
                reason: "not connected".into(),
            }),
        }
    };
    let peer = peer_res?;

    let mut tools = peer
        .list_all_tools()
        .await
        .map_err(|e| rmcp_service_err(&cfg.name, "list_tools", e))?;
    if tools.len() > TOOLS_PER_SERVER_CAP {
        crate::app_warn!(
            "mcp",
            &format!("{}:catalog", cfg.name),
            "Server advertised {} tools; truncating to the per-server cap of {}",
            tools.len(),
            TOOLS_PER_SERVER_CAP
        );
        tools.truncate(TOOLS_PER_SERVER_CAP);
    }

    // Resources / prompts are optional per spec; an `InvalidRequest` /
    // method-not-found is NOT a real failure — it just means the server
    // doesn't expose that primitive.
    let resources = peer.list_all_resources().await.unwrap_or_default();
    let prompts = peer.list_all_prompts().await.unwrap_or_default();

    let tool_count = tools.len();
    let resource_count = resources.len();
    let prompt_count = prompts.len();

    rebuild_tool_index_for(manager, &cfg, &tools).await;

    set_state(
        &handle,
        ServerState::Ready {
            tools,
            resources,
            prompts,
        },
    )
    .await;

    emit_server_status(&cfg.id, &cfg.name, "ready", None);
    emit_catalog_refreshed(&cfg.id, &cfg.name, tool_count, resource_count, prompt_count);
    crate::app_info!(
        "mcp",
        &format!("{}:catalog", cfg.name),
        "MCP '{}' catalog: {} tools / {} resources / {} prompts",
        cfg.name,
        tool_count,
        resource_count,
        prompt_count
    );
    Ok(())
}

// ── Internals ────────────────────────────────────────────────────

async fn do_connect(cfg: &super::config::McpServerConfig, handle: &ServerHandle) -> McpResult<()> {
    // `build_transport_for` runs the SSRF gate (for HTTP/SSE/WS) and
    // completes the rmcp handshake internally — isolating the concrete
    // reqwest-0.13 client type from the rest of the subsystem, which
    // would otherwise conflict with ha-core's reqwest-0.12 dep.
    let ConnectedClient { running, stderr } = build_transport_for(cfg).await?;

    if let Some(err_stream) = stderr {
        // Spawn the stderr tailer AFTER the handshake — prior to this
        // the child hasn't produced output yet, and doing it post-serve
        // keeps the flow simpler.
        spawn_stderr_tailer(cfg.name.clone(), err_stream);
    }

    let mut client = handle.client.lock().await;
    *client = Some(running);
    Ok(())
}

async fn rebuild_tool_index_for(
    manager: &McpManager,
    cfg: &super::config::McpServerConfig,
    tools: &[model::Tool],
) {
    // 1. Update the id → (server, original_name) reverse lookup used by
    //    `invoke::call_tool` for dispatch. Drops stale entries owned by
    //    this server before inserting fresh ones.
    {
        let mut idx = manager.tool_index.write().await;
        idx.retain(|_, e| e.server_id != cfg.id);
        for tool in tools {
            let orig = tool.name.to_string();
            if !cfg.denied_tools.is_empty() && cfg.denied_tools.contains(&orig) {
                continue;
            }
            if !cfg.allowed_tools.is_empty() && !cfg.allowed_tools.contains(&orig) {
                continue;
            }
            let namespaced = super::catalog::namespaced_tool_name(&cfg.name, &orig);
            idx.insert(
                namespaced,
                ToolIndexEntry {
                    server_id: cfg.id.clone(),
                    server_name: cfg.name.clone(),
                    original_tool_name: orig,
                },
            );
        }
    }

    // 2. Rebuild the sync-readable ToolDefinition cache. The schema
    //    assembly path in `agent/mod.rs::build_tool_schemas` reads this
    //    without awaiting — it must be kept atomic with the reverse
    //    lookup so a dispatch never finds a name that isn't in the
    //    schema list, or vice versa.
    let always_load_names = manager
        .global_settings
        .read()
        .await
        .always_load_servers
        .clone();
    let always_load = always_load_names.iter().any(|n| n == &cfg.name);

    let defs_for_server: Vec<crate::tools::ToolDefinition> = tools
        .iter()
        .filter(|tool| {
            let orig = tool.name.to_string();
            !(cfg.denied_tools.iter().any(|d| d == &orig)
                || (!cfg.allowed_tools.is_empty() && !cfg.allowed_tools.iter().any(|a| a == &orig)))
        })
        .map(|t| super::catalog::rmcp_tool_to_definition(cfg, t, always_load))
        .collect();

    // Merge: keep other servers' defs, replace this server's. Use the
    // `mcp__<cfg.name>__` prefix as the ownership test.
    let prefix = format!("{}{}{}", super::catalog::MCP_TOOL_PREFIX, cfg.name, "__");
    let prior = manager.mcp_tool_definitions();
    let mut next: Vec<crate::tools::ToolDefinition> = prior
        .iter()
        .filter(|d| !d.name.starts_with(&prefix))
        .cloned()
        .collect();
    next.extend(defs_for_server);
    manager.store_cached_tool_defs(next);
}

fn rmcp_service_err(server: &str, where_: &str, err: rmcp::service::ServiceError) -> McpError {
    McpError::Protocol {
        server: server.to_string(),
        code: None,
        message: format!("{where_}: {err}"),
    }
}

async fn set_state(handle: &ServerHandle, new_state: ServerState) {
    let mut state: MutexGuard<'_, ServerState> = handle.state.lock().await;
    *state = new_state;
}

async fn record_failure(handle: &ServerHandle, server_name: &str, err: &McpError) {
    handle
        .consecutive_failures
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = chrono::Utc::now().timestamp();
    // Tiny placeholder backoff — the real exponential-backoff policy
    // lives in `watchdog.rs`; this just puts us in the right state so
    // the watchdog can pick up the scheduling.
    let retry_at = now + 5;
    set_state(
        handle,
        ServerState::Failed {
            reason: err.to_string(),
            retry_at,
        },
    )
    .await;
    let cfg_id = handle.config.read().await.id.clone();
    emit_server_status(&cfg_id, server_name, "failed", Some(&err.to_string()));
    crate::app_warn!(
        "mcp",
        &format!("{server_name}:connect"),
        "MCP connect/refresh failed: {err}"
    );
}

/// Max bytes from a single stderr line kept in the log; stack traces
/// from crashing servers can be multi-MB and would saturate the log DB.
const STDERR_LINE_TRUNCATE_BYTES: usize = 4096;

/// Token bucket: at most this many lines get written per window before
/// the tailer drops further lines and emits one summary "N lines
/// suppressed" warning. Prevents a runaway server from DoS-ing the
/// logger.
const STDERR_RATE_LIMIT_LINES: u32 = 100;
const STDERR_RATE_LIMIT_WINDOW_SECS: u64 = 10;

/// Forward each line of the child's stderr to the app log with a stable
/// source prefix `<server_name>:stderr`. Warn-level because MCP servers
/// commonly mix their own info logs in there and users want to see
/// them without tailing a separate file.
///
/// Rate-limit + per-line truncation defend the shared `AppLogger`
/// SQLite store against a chatty or crashing server's firehose.
fn spawn_stderr_tailer(server_name: String, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let mut window_start = std::time::Instant::now();
        let mut lines_in_window: u32 = 0;
        let mut suppressed_in_window: u32 = 0;
        let source = format!("{server_name}:stderr");
        while let Ok(Some(line)) = lines.next_line().await {
            let now = std::time::Instant::now();
            if now.duration_since(window_start).as_secs() >= STDERR_RATE_LIMIT_WINDOW_SECS {
                if suppressed_in_window > 0 {
                    crate::app_warn!(
                        "mcp",
                        &source,
                        "[suppressed {suppressed_in_window} lines over {STDERR_RATE_LIMIT_WINDOW_SECS}s]"
                    );
                }
                window_start = now;
                lines_in_window = 0;
                suppressed_in_window = 0;
            }
            if lines_in_window >= STDERR_RATE_LIMIT_LINES {
                suppressed_in_window += 1;
                continue;
            }
            lines_in_window += 1;
            let trimmed = if line.len() > STDERR_LINE_TRUNCATE_BYTES {
                format!(
                    "{}… [truncated {} bytes]",
                    crate::truncate_utf8(&line, STDERR_LINE_TRUNCATE_BYTES),
                    line.len().saturating_sub(STDERR_LINE_TRUNCATE_BYTES)
                )
            } else {
                line
            };
            crate::app_warn!("mcp", &source, "{}", trimmed);
        }
    });
}
