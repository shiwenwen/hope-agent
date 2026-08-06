//! MCP client lifecycle — connect, refresh catalog, disconnect.
//!
//! Callers go through [`ensure_connected`] which is idempotent:
//! `Ready` short-circuits, `Connecting` waits for the in-flight attempt,
//! `Idle` / `Failed` kicks off a fresh handshake. [`connect_now`] is the
//! user-triggered equivalent and bypasses the backoff window.
//!
//! Every connection attempt that succeeds immediately fetches the
//! initial catalog (tools + resources + prompts) and atomically publishes it
//! through the manager's `CatalogSnapshot`.

use std::sync::Arc;

use futures_util::stream::{self, StreamExt};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{MutexGuard, OnceCell};
use tokio::time::{timeout, Duration};

use super::errors::{McpError, McpResult};
use super::events::{emit_catalog_refreshed, emit_server_status};
use super::registry::{McpManager, ServerHandle, ServerState};
use super::transport::{build_transport_for, ConnectedClient};

/// Bound catalog discovery bursts independently from tool-call concurrency.
/// A user can configure many MCP servers; the first `tool_search` must not
/// spawn all stdio children or handshakes at once.
const CATALOG_DISCOVERY_CONCURRENCY: usize = 4;
/// Chat-facing discovery latency must not grow with the number of configured
/// servers. Workers that outlive this aggregate deadline remain detached and
/// continue publishing ready catalogs in the background.
const CATALOG_DISCOVERY_WAIT_SECS: u64 = 30;

/// One process-wide startup barrier. A failed eager handshake still completes
/// the barrier: normal chat must not synchronously retry an unavailable MCP
/// server on every turn. Later recovery belongs to config warm-up, watchdog,
/// or an explicit discovery operation.
static INITIAL_EAGER_CATALOG_WARMUP: OnceCell<()> = OnceCell::const_new();

/// Start the one-shot startup warm-up for `eager=true` servers. The first
/// provider request awaits this same cell if the task is still running.
pub fn spawn_initial_eager_catalog_warmup() {
    tokio::spawn(async {
        ensure_initial_eager_tool_catalogs().await;
    });
}

/// Warm eager servers after a live config reconciliation without putting the
/// operation back on the chat critical path.
pub fn spawn_reconciled_eager_catalog_warmup() {
    tokio::spawn(async {
        warm_catalogs(true, "config_reconcile").await;
    });
}

/// Populate catalogs for lazy servers on the first discovery operation.
/// Failures are isolated per server: ready catalogs still participate in the
/// current `tool_search`, while failed servers retain their normal backoff /
/// NeedsAuth state and diagnostics.
pub async fn ensure_tool_catalogs() {
    warm_catalogs(false, "tool_search").await;
}

/// Wait for the startup contract of `eager=true` servers. The subsystem also
/// spawns this work immediately during initialization, but the first provider
/// request awaits the same idempotent path so schema assembly cannot win the
/// startup race.
pub async fn ensure_initial_eager_tool_catalogs() {
    INITIAL_EAGER_CATALOG_WARMUP
        .get_or_init(|| async {
            warm_catalogs(true, "startup").await;
        })
        .await;
}

async fn warm_catalogs(eager_only: bool, source: &'static str) {
    let Some(manager) = McpManager::global() else {
        return;
    };
    if !manager.is_enabled().await {
        return;
    }

    let handles: Vec<Arc<ServerHandle>> = manager.servers.read().await.values().cloned().collect();
    let server_count = handles.len();
    let worker = tokio::spawn(async move {
        stream::iter(handles)
            .for_each_concurrent(CATALOG_DISCOVERY_CONCURRENCY, |handle| async move {
                let cfg = handle.config.read().await.clone();
                if eager_only && !cfg.eager {
                    return;
                }
                let should_connect = {
                    let state = handle.state.lock().await;
                    match &*state {
                        ServerState::Idle | ServerState::Connecting => true,
                        ServerState::Failed { retry_at, .. } => {
                            chrono::Utc::now().timestamp() >= *retry_at
                        }
                        ServerState::Ready { .. }
                        | ServerState::Disabled
                        | ServerState::NeedsAuth { .. } => false,
                    }
                };
                if !should_connect {
                    return;
                }
                if let Err(error) = ensure_connected(manager, handle).await {
                    ha_core::app_warn!(
                        "mcp",
                        &format!("{}:catalog_discovery", cfg.name),
                        "MCP catalog discovery from {} failed: {}",
                        source,
                        error
                    );
                }
            })
            .await;
    });
    match await_catalog_worker(
        worker,
        Duration::from_secs(CATALOG_DISCOVERY_WAIT_SECS),
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => ha_core::app_warn!(
            "mcp",
            "catalog_discovery_deadline",
            "MCP catalog discovery from {} reached the aggregate {}s deadline across {} servers; ready catalogs remain available and unfinished work continues in background",
            source,
            CATALOG_DISCOVERY_WAIT_SECS,
            server_count
        ),
        Err(error) => ha_core::app_warn!(
            "mcp",
            "catalog_discovery_worker",
            "MCP catalog discovery worker from {} failed: {}",
            source,
            error
        ),
    }
}

/// `false` means the join handle was dropped at the deadline. Tokio detaches
/// rather than aborts on `JoinHandle::drop`, so the bounded caller can return
/// while the worker retains its per-server concurrency cap and finishes in the
/// background.
async fn await_catalog_worker(
    worker: tokio::task::JoinHandle<()>,
    deadline: Duration,
) -> Result<bool, tokio::task::JoinError> {
    match timeout(deadline, worker).await {
        Ok(joined) => joined.map(|()| true),
        Err(_) => Ok(false),
    }
}

/// Idempotent "make sure this server is connected and has a catalog".
/// Returns quickly when already `Ready`; otherwise performs a full
/// connect + list_all_tools + list_all_resources + list_all_prompts
/// round under the configured `connect_timeout_secs`.
pub async fn ensure_connected(manager: &McpManager, handle: Arc<ServerHandle>) -> McpResult<()> {
    // Fast path: already good.
    if !connect_needed_or_error(&handle).await? {
        return Ok(());
    }

    let _connect_guard = handle.connect_lock.lock().await;
    // Another caller may have completed the handshake while we were waiting
    // for the lock. Re-check before doing any work.
    if !connect_needed_or_error(&handle).await? {
        return Ok(());
    }
    connect_now_inner(manager, handle.clone()).await
}

/// Force a (re)connect regardless of current state. Used by the user's
/// "Reconnect" button, by the watchdog after a timer tick, and by Phase 3
/// CRUD paths that need immediate visibility after a config change.
pub async fn connect_now(manager: &McpManager, handle: Arc<ServerHandle>) -> McpResult<()> {
    let _connect_guard = handle.connect_lock.lock().await;
    connect_now_inner(manager, handle.clone()).await
}

async fn connect_needed_or_error(handle: &ServerHandle) -> McpResult<bool> {
    if handle.is_retired() {
        let cfg = handle.config.read().await;
        return Err(McpError::NotReady {
            server: cfg.name.clone(),
            reason: "server config was replaced or removed".into(),
        });
    }
    let now = chrono::Utc::now().timestamp();
    let decision = {
        let state = handle.state.lock().await;
        match &*state {
            ServerState::Ready { .. } => Ok(false),
            ServerState::Disabled => Err("server is disabled in config".to_string()),
            ServerState::NeedsAuth { .. } => {
                Err("authorization required; use the Authorize action in Settings".to_string())
            }
            ServerState::Failed { retry_at, reason } if now < *retry_at => Err(format!(
                "in backoff after failure ({reason}); retry_at in {}s",
                *retry_at - now
            )),
            ServerState::Idle | ServerState::Connecting | ServerState::Failed { .. } => Ok(true),
        }
    };
    match decision {
        Ok(needed) => Ok(needed),
        Err(reason) => {
            let cfg = handle.config.read().await;
            Err(McpError::NotReady {
                server: cfg.name.clone(),
                reason,
            })
        }
    }
}

async fn connect_now_inner(manager: &McpManager, handle: Arc<ServerHandle>) -> McpResult<()> {
    let cfg = handle.config.read().await.clone();
    if handle.is_retired() {
        return Err(McpError::NotReady {
            server: cfg.name,
            reason: "server config was replaced or removed".into(),
        });
    }
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

    // The startup/discovery timeout covers both the transport handshake and
    // the initial tools/resources/prompts inventory. Bounding only
    // `do_connect` lets a server that accepts the handshake but never answers
    // `list_tools` hold the process-wide eager barrier forever.
    let result = timeout(connect_timeout, async {
        do_connect(&cfg, &handle).await?;
        if handle.is_retired() {
            return Err(McpError::NotReady {
                server: cfg.name.clone(),
                reason: "server config was replaced or removed".into(),
            });
        }
        handle
            .consecutive_failures
            .store(0, std::sync::atomic::Ordering::Relaxed);
        refresh_catalog(manager, handle.clone()).await
    })
    .await;
    match result {
        Ok(Ok(())) => {
            ha_core::app_info!(
                "mcp",
                &format!("{}:connect", cfg.name),
                "Connected to MCP server '{}' via {}",
                cfg.name,
                cfg.transport.kind_label()
            );
            Ok(())
        }
        Ok(Err(e)) => {
            disconnect_after_failed_attempt(&handle).await;
            if handle.is_retired() {
                return Err(e);
            }
            record_failure(&handle, &cfg.name, &e).await;
            Err(e)
        }
        Err(_elapsed) => {
            let err = McpError::Timeout {
                server: cfg.name.clone(),
                tool: "<connect/catalog>".into(),
                secs: cfg.connect_timeout_secs,
            };
            disconnect_after_failed_attempt(&handle).await;
            if handle.is_retired() {
                return Err(err);
            }
            record_failure(&handle, &cfg.name, &err).await;
            Err(err)
        }
    }
}

/// Best-effort cleanup must not turn a bounded failed attempt back into an
/// unbounded wait. `disconnect` takes the running client out before awaiting
/// cancellation, so dropping this one-second cleanup future still prevents a
/// stale connection from being reused.
async fn disconnect_after_failed_attempt(handle: &ServerHandle) {
    let _ = timeout(Duration::from_secs(1), disconnect(handle)).await;
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
    if handle.is_retired() {
        return Err(McpError::NotReady {
            server: cfg.name,
            reason: "server config was replaced or removed".into(),
        });
    }
    let peer = handle.peer().await?;

    let mut tools = peer
        .list_all_tools()
        .await
        .map_err(|e| rmcp_service_err(&cfg.name, "list_tools", e))?;
    if tools.len() > TOOLS_PER_SERVER_CAP {
        ha_core::app_warn!(
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

    if handle.is_retired() {
        return Err(McpError::NotReady {
            server: cfg.name,
            reason: "server config was replaced or removed".into(),
        });
    }

    manager
        .publish_ready_catalog(&handle, tools, resources, prompts)
        .await?;

    emit_server_status(&cfg.id, &cfg.name, "ready", None);
    emit_catalog_refreshed(&cfg.id, &cfg.name, tool_count, resource_count, prompt_count);
    ha_core::app_info!(
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
    // Auth errors take a different branch — the recovery path is "user
    // clicks Authorize in the GUI", not "watchdog retries". Keeping the
    // server in NeedsAuth instead of Failed prevents a tight retry loop
    // that would spam refresh attempts against an already-broken token.
    let cfg_id = handle.config.read().await.id.clone();
    if matches!(err, McpError::Auth { .. }) {
        set_state(
            handle,
            ServerState::NeedsAuth {
                // Real authorize URL is emitted dynamically by
                // `oauth::authorize_server` (embeds one-shot PKCE); we
                // leave this empty to signal "press the button and the
                // backend will produce a fresh URL".
                auth_url: String::new(),
            },
        )
        .await;
        emit_server_status(&cfg_id, server_name, "needsAuth", Some(&err.to_string()));
        ha_core::app_warn!(
            "mcp",
            &format!("{server_name}:auth"),
            "MCP server requires re-authorization: {err}"
        );
        return;
    }
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
    emit_server_status(&cfg_id, server_name, "failed", Some(&err.to_string()));
    ha_core::app_warn!(
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
                    ha_core::app_warn!(
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
                    ha_core::truncate_utf8(&line, STDERR_LINE_TRUNCATE_BYTES),
                    line.len().saturating_sub(STDERR_LINE_TRUNCATE_BYTES)
                )
            } else {
                line
            };
            ha_core::app_warn!("mcp", &source, "{}", trimmed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn sample_handle() -> ServerHandle {
        let config = serde_json::from_value(serde_json::json!({
            "id": "id-auth",
            "name": "auth",
            "enabled": true,
            "transport": { "kind": "stdio", "command": "true" }
        }))
        .expect("valid MCP server fixture");
        ServerHandle::new(config)
    }

    #[tokio::test]
    async fn generic_lazy_connect_does_not_retry_needs_auth() {
        let handle = sample_handle();
        *handle.state.lock().await = ServerState::NeedsAuth {
            auth_url: String::new(),
        };

        let error = connect_needed_or_error(&handle)
            .await
            .expect_err("NeedsAuth must require an explicit owner action");
        assert!(error.to_string().contains("authorization required"));
        assert_eq!(handle.snapshot().await.state, "needsAuth");
    }

    #[tokio::test]
    async fn aggregate_catalog_deadline_detaches_remaining_work() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(AtomicBool::new(false));
        let worker_gate = gate.clone();
        let worker_completed = completed.clone();
        let worker = tokio::spawn(async move {
            worker_gate.notified().await;
            worker_completed.store(true, Ordering::Release);
        });

        let completed_in_time = await_catalog_worker(worker, Duration::from_millis(1))
            .await
            .unwrap();
        assert!(!completed_in_time);

        gate.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !completed.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached catalog worker should keep running after the caller deadline");
    }
}
