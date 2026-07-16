//! Shared helpers for `#[cfg(test)]` code across the crate.
//!
//! Compiled only under `cfg(test)` (see `lib.rs`); never reaches release
//! builds. Add helpers here when at least two test modules need the same
//! pattern — single-module helpers should stay private to that module.

use std::future::Future;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

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
    let guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    // Do not resume a captured panic while holding the mutex: unwinding a
    // MutexGuard would poison it and turn one assertion into dozens of
    // unrelated follow-up failures.
    drop(guard);

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
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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

fn config_cache_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Serialize tests that use the process-global background-jobs database.
/// Module-local locks are insufficient because `ASYNC_JOBS_DB` is shared by
/// workflow, goal, loop and job-status tests.
pub fn lock_async_jobs() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Restores the process-wide config snapshot and serializes tests that replace
/// it. Without this guard, parallel tests can observe another test's permission
/// policy or provider configuration.
pub struct ConfigCacheRestore {
    previous: crate::config::AppConfig,
    _guard: MutexGuard<'static, ()>,
}

impl Drop for ConfigCacheRestore {
    fn drop(&mut self) {
        crate::config::replace_cache_for_test(self.previous.clone());
    }
}

pub fn replace_config_cache(config: crate::config::AppConfig) -> ConfigCacheRestore {
    let guard = config_cache_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = (*crate::config::cached_config()).clone();
    crate::config::replace_cache_for_test(config);
    ConfigCacheRestore {
        previous,
        _guard: guard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_lock_remains_usable_after_inner_panic() {
        let panic = std::panic::catch_unwind(|| {
            with_env_vars(&[("HA_TEST_ENV_LOCK_RECOVERY", Path::new("first"))], || {
                panic!("intentional test panic")
            });
        });
        assert!(panic.is_err());

        with_env_vars(
            &[("HA_TEST_ENV_LOCK_RECOVERY", Path::new("second"))],
            || {
                assert_eq!(
                    std::env::var_os("HA_TEST_ENV_LOCK_RECOVERY").as_deref(),
                    Some(std::ffi::OsStr::new("second"))
                );
            },
        );
    }
}
