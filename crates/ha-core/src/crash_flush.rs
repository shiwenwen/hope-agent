//! Crash-time hooks for flushing in-flight `StreamPersister` placeholders
//! when the process is exiting cleanly. Signal handlers run on actual
//! shutdown (SIGINT/SIGTERM/Ctrl+C/Ctrl+Break) and call
//! `flush_all_blocking` to mark every active placeholder `orphaned` before
//! `std::process::exit`.
//!
//! Panic recovery is intentionally NOT global. Tokio tasks, Tauri commands,
//! and `catch_unwind` boundaries routinely turn local panics into recovered
//! errors while the process keeps running; flushing every active persister
//! on a panic anywhere in the process would corrupt unrelated active
//! sessions. Per-task panic safety lives in `StreamPersister::Drop`: the
//! unwinding task drops its `Arc`, `Drop` finalizes that one placeholder
//! to `orphaned`, and other concurrent sessions are untouched.
//!
//! `install_signal_handlers` requires an ambient tokio runtime; call it
//! from the Tauri `setup` async block, the HTTP server `main`, or the ACP
//! entrypoint after their runtimes are up.

use std::sync::OnceLock;

use crate::chat_engine::active_persisters;

static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();
static SIGNAL_HANDLERS_INSTALLED: OnceLock<()> = OnceLock::new();

/// Idempotent no-op kept for API stability. A process-wide panic hook
/// that SIGKILLs registered exec subprocesses was considered but
/// rejected: tokio task panics are commonly recovered via `JoinHandle`
/// boundaries without the process exiting, and a global kill on any
/// thread's panic would tear down unrelated long-running user commands.
/// Per-task cleanup runs through `tools::exec::ProcessGroupGuard::Drop`
/// (kills the offending exec's own process group) and
/// `StreamPersister::Drop` (finalizes that one placeholder row).
pub fn install_panic_hook() {
    let _ = PANIC_HOOK_INSTALLED.set(());
}

/// Finalize every active GUI/HTTP turn with `TerminationReason::Shutdown`
/// before the process exits. Called from the signal handler after the
/// shutdown sentinel is written. Synchronous so we don't depend on a
/// runtime that may already be tearing down.
fn finalize_active_turns_for_shutdown() {
    let Some(db) = crate::get_session_db() else {
        return;
    };
    let active = crate::chat_engine::active_turn::all_current();
    if active.is_empty() {
        return;
    }
    for snapshot in active {
        // A running unified journal is already durable-before-broadcast and is
        // recovered atomically on the next launch with the clean-shutdown
        // marker. The legacy messages-based finalizer must not terminalize its
        // chat_turn first or advance context independently of the run CAS.
        if db
            .latest_stream_run(&snapshot.session_id)
            .ok()
            .flatten()
            .is_some_and(|run| {
                run.status == "running" && run.turn_id.as_deref() == Some(snapshot.turn_id.as_str())
            })
        {
            continue;
        }
        // Mirror app_init's startup-sweep behavior: resolve the
        // session's actual provider shape so a Shutdown finalize on an
        // OpenAI Chat / Responses / Codex session doesn't get rebuilt
        // as Anthropic `tool_use` / `tool_result` (which the original
        // provider would 4xx or silently drop on resume).
        let provider_kind =
            crate::chat_engine::finalize::rebuild::resolve_provider_kind_for_session(
                &db,
                &snapshot.session_id,
            );
        let partial = crate::chat_engine::finalize::rebuild::collect_partial_from_messages(
            &db,
            &snapshot.session_id,
            provider_kind,
        );
        let partial = crate::chat_engine::finalize::PartialMeta {
            turn_id: Some(snapshot.turn_id.clone()),
            ..partial
        };
        let _ = crate::chat_engine::finalize::finalize_turn_context_blocking(
            &db,
            &snapshot.session_id,
            crate::chat_engine::finalize::TerminationReason::Shutdown,
            partial,
            snapshot.source,
        );
    }
}

/// Install signal handlers (SIGINT/SIGTERM on Unix, ctrl_c/ctrl_break on
/// Windows) that flush active persisters and exit cleanly. Idempotent.
/// MUST be called from within a tokio runtime — uses `tokio::spawn`.
pub fn install_signal_handlers() {
    if SIGNAL_HANDLERS_INSTALLED.set(()).is_err() {
        return;
    }

    #[cfg(unix)]
    {
        tokio::spawn(async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(e) => {
                    app_warn!(
                        "session",
                        "stream_persist",
                        "install SIGINT handler failed: {}",
                        e
                    );
                    return;
                }
            };
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    app_warn!(
                        "session",
                        "stream_persist",
                        "install SIGTERM handler failed: {}",
                        e
                    );
                    return;
                }
            };
            tokio::select! {
                _ = sigint.recv() => {
                    app_info!("session", "stream_persist", "received SIGINT, clean shutdown");
                }
                _ = sigterm.recv() => {
                    app_info!("session", "stream_persist", "received SIGTERM, clean shutdown");
                }
            }
            fire_shutdown_session_end().await;
            run_clean_shutdown();
        });
    }

    #[cfg(windows)]
    {
        tokio::spawn(async {
            let mut ctrl_c = match tokio::signal::windows::ctrl_c() {
                Ok(s) => s,
                Err(e) => {
                    app_warn!(
                        "session",
                        "stream_persist",
                        "install ctrl_c handler failed: {}",
                        e
                    );
                    return;
                }
            };
            let mut ctrl_break = match tokio::signal::windows::ctrl_break() {
                Ok(s) => s,
                Err(e) => {
                    app_warn!(
                        "session",
                        "stream_persist",
                        "install ctrl_break handler failed: {}",
                        e
                    );
                    return;
                }
            };
            tokio::select! {
                _ = ctrl_c.recv() => {
                    app_info!("session", "stream_persist", "received Ctrl+C, clean shutdown");
                }
                _ = ctrl_break.recv() => {
                    app_info!("session", "stream_persist", "received Ctrl+Break, clean shutdown");
                }
            }
            fire_shutdown_session_end().await;
            run_clean_shutdown();
        });
    }
}

/// Fire the `SessionEnd` shutdown hook (app-global, source `other`) on the
/// real shutdown path, bounded so a slow command hook can't wedge process
/// termination. No-op (returns immediately) when no `SessionEnd` hook is
/// configured, so the common case adds no shutdown latency. This is the single
/// place SessionEnd-on-shutdown fires for signal-driven exits (server / ACP /
/// terminal Ctrl-C); GUI window-quit fires it separately from `RunEvent::Exit`.
async fn fire_shutdown_session_end() {
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::hooks::dispatch_session_end("", "other"),
    )
    .await;
}

/// Shared cleanup sequence for SIGINT/SIGTERM/Ctrl+C/Ctrl+Break:
/// sentinel → flush placeholders → finalize active turns → exit.
///
/// **Order matters**: `flush_all_blocking` must run *before* finalize
/// so any in-memory `StreamPersister` buffer that hasn't reached its
/// 500ms / 1KB throttle is persisted as a placeholder row first. The
/// finalize pass then reverse-rebuilds from `messages` and writes the
/// `Shutdown` marker / event row / chat_turn closure including those
/// last-moment bytes. Reversing this order writes finalize from the
/// pre-flush DB state and then dangles the flushed rows as orphans
/// the next launch's restore would miss.
fn run_clean_shutdown() -> ! {
    crate::chat_engine::finalize::sentinel::write_clean_marker();
    active_persisters::flush_all_blocking();
    finalize_active_turns_for_shutdown();
    std::process::exit(0);
}
