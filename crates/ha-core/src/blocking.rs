//! Blocking-work isolation for async contexts.
//!
//! Every SQLite database in the app (sessions / cron / channel / logs) is
//! synchronous rusqlite behind a `Mutex<Connection>`, and config persistence
//! does synchronous file IO under a global write lock. Calling any of that
//! directly from an async fn pins a tokio worker for the full lock-wait + IO
//! duration. Workers number only `num_cpus`; if the underlying file IO stalls
//! (antivirus scan, cloud-synced home directory, slow disk), workers are
//! consumed one by one until the whole runtime starves — the "process alive
//! but every command loading forever" freeze (issue #433 comment, Bug 2).
//!
//! [`run_blocking`] routes such work onto tokio's blocking pool (hundreds of
//! expendable threads) so a stalled database or config write degrades that
//! one feature instead of freezing the entire app. It also logs any blocking
//! op that exceeds [`SLOW_OP_WARN`] — turning the next field occurrence of a
//! stalled IO path into a grep-able `logs.db` entry instead of a heisenbug.

use std::time::{Duration, Instant};

/// Blocking ops slower than this are logged via `app_warn!` with the closure's
/// definition site, so a wedged lock/IO path names its holder in the logs.
const SLOW_OP_WARN: Duration = Duration::from_secs(5);

/// Run a synchronous (potentially blocking) operation on tokio's blocking
/// pool and await its result without pinning a runtime worker.
///
/// Must be called from within a tokio runtime (all Tauri commands and
/// ha-server handlers qualify). Panics inside `f` are resumed on the caller,
/// matching the behavior of running `f` inline.
///
/// The slow-op label is `type_name::<F>()`, which embeds `f`'s definition site
/// (module path + enclosing function). Wrappers that add an indirection layer
/// (e.g. [`crate::session::SessionDB::run`], `config::mutate_config_async`)
/// would otherwise log their own wrapper closure for every call — they call
/// [`run_blocking_labeled`] with the *caller's* closure type name instead.
pub async fn run_blocking<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_blocking_labeled(std::any::type_name::<F>(), f).await
}

/// Like [`run_blocking`] but with an explicit slow-op label. Use from thin
/// wrappers whose own closure type would be a useless, always-identical label
/// (they pass `type_name` of the *caller-supplied* closure instead).
pub async fn run_blocking_labeled<T, F>(label: &'static str, f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let started = Instant::now();
    let result = tokio::task::spawn_blocking(f).await;
    let elapsed = started.elapsed();
    if elapsed >= SLOW_OP_WARN {
        crate::app_warn!(
            "blocking",
            "run_blocking",
            "blocking op took {:.1}s (queue + execution): {}",
            elapsed.as_secs_f64(),
            label
        );
    }
    match result {
        Ok(value) => value,
        Err(join_err) => match join_err.try_into_panic() {
            // A panic inside `f` — propagate it unchanged so it surfaces on the
            // caller exactly as an inline call would.
            Ok(panic) => std::panic::resume_unwind(panic),
            // Not a panic: a blocking task cancelled before it started, only
            // reachable while the runtime is shutting down. We have no `T` to
            // return; surface a clear message rather than `into_panic`'s opaque
            // secondary panic ("called into_panic on a non-panic JoinError").
            Err(_) => panic!("run_blocking task cancelled (runtime shutting down)"),
        },
    }
}
