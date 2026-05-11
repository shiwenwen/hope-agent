//! Shared helpers for `#[cfg(test)]` code across the crate.
//!
//! Compiled only under `cfg(test)` (see `lib.rs`); never reaches release
//! builds. Add helpers here when at least two test modules need the same
//! pattern — single-module helpers should stay private to that module.

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
