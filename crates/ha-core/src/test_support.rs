//! Shared helpers for `#[cfg(test)]` code across the crate.
//!
//! Compiled only under `cfg(test)` (see `lib.rs`); never reaches release
//! builds. Add helpers here when at least two test modules need the same
//! pattern — single-module helpers should stay private to that module.

use std::future::Future;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Global lock serializing tests that mutate process-wide environment
/// variables. cargo test runs tests in parallel by default, so without this
/// lock two tests writing the same env var would race and read each other's
/// values. `catch_unwind` ensures the previous value is restored even when
/// the inner closure panics.
fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Run `f` with the given env vars set, restoring the previous values
/// (or unsetting if not previously set) afterwards. Holds a process-wide
/// mutex for the duration of the call so concurrent tests don't trample.
pub fn with_env_vars<T>(vars: &[(&str, &Path)], f: impl FnOnce() -> T) -> T {
    let _guard = env_lock().lock().expect("test env lock poisoned");
    let previous: Vec<_> = vars
        .iter()
        .map(|(key, _)| (*key, std::env::var_os(key)))
        .collect();
    for (key, value) in vars {
        std::env::set_var(key, value);
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    for (key, value) in previous {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Async sibling of [`with_env_vars`] — same isolation guarantees, but
/// the closure returns a future that gets awaited inside the env-mutated
/// scope. Use this from `#[tokio::test]` where `with_env_vars` would
/// otherwise need a nested runtime.
///
/// Panic safety: the env vars are restored if the awaited future panics,
/// because the mutex guard is dropped at function return (Rust unwinding
/// still runs Drop). Unlike `with_env_vars` there's no `catch_unwind`
/// here — futures aren't `UnwindSafe` in general — but the env-restore
/// path runs regardless because it sits before the panic propagates out
/// of the function. If a panic mid-future is so destructive that
/// `std::env` state matters more than the panic itself, prefer
/// `with_env_vars` with a sync inner closure.
pub async fn with_env_vars_async<F, Fut, T>(vars: &[(&str, &Path)], f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let _guard = env_lock().lock().expect("test env lock poisoned");
    let previous: Vec<_> = vars
        .iter()
        .map(|(key, _)| (*key, std::env::var_os(key)))
        .collect();
    for (key, value) in vars {
        std::env::set_var(key, value);
    }

    // Defer-restore on every exit path (Drop runs on panic too).
    struct Restore<'a>(Vec<(&'a str, Option<std::ffi::OsString>)>);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            for (key, value) in std::mem::take(&mut self.0) {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
    let _restore = Restore(previous);

    f().await
}
