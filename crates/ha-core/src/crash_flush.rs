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

/// Idempotent no-op kept for API stability; per-task panic cleanup runs
/// through `StreamPersister::Drop` instead of a global flush. Removing
/// it from caller code would force every entrypoint to inline the same
/// reasoning, so we keep the export but make the body trivial.
pub fn install_panic_hook() {
    let _ = PANIC_HOOK_INSTALLED.set(());
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
                    app_info!("session", "stream_persist", "received SIGINT, crash flush");
                }
                _ = sigterm.recv() => {
                    app_info!("session", "stream_persist", "received SIGTERM, crash flush");
                }
            }
            active_persisters::flush_all_blocking();
            std::process::exit(0);
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
                    app_info!("session", "stream_persist", "received Ctrl+C, crash flush");
                }
                _ = ctrl_break.recv() => {
                    app_info!("session", "stream_persist", "received Ctrl+Break, crash flush");
                }
            }
            active_persisters::flush_all_blocking();
            std::process::exit(0);
        });
    }
}
