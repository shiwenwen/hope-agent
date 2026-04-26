//! Tests for `runtime_lock`.
//!
//! Cargo runs each integration test file as its own binary, so the
//! module-level `OnceLock` state in `runtime_lock` doesn't bleed into
//! other test files. Within this file we keep a single `#[test]` so
//! parallel test functions don't race on `std::env::HOME` or on the
//! shared `TIER` / `LOCK_FILE` statics.
//!
//! True multi-process semantics (cross-process contention, crash
//! release, kill -9 release) are exercised by the manual smoke
//! checklist in the PR description rather than an `#[ignore]`
//! subprocess fixture — `cargo test`'s default test runner consumes
//! stdout in a way that makes spawning the test binary as a probe
//! flaky, and the build-time cost of an example binary or
//! `[[bin]]` shim isn't worth it for an opt-in `--ignored` lane.
//! See `PR description → manual smoke checklist § 1-4`.

use std::sync::Arc;

use ha_core::runtime_lock::{self, Tier};

#[test]
fn runtime_lock_full_lifecycle() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", tmp.path());
    #[cfg(windows)]
    std::env::set_var("USERPROFILE", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");

    // ── First call: fresh data dir, lock free → Primary. ──
    let tier = runtime_lock::acquire_or_secondary("test");
    assert_eq!(tier, Tier::Primary);
    assert!(runtime_lock::is_primary());
    assert_eq!(runtime_lock::tier(), Some(Tier::Primary));

    // ── Idempotency: repeated calls return the same tier without
    //    re-acquiring (would otherwise contend with our own held lock). ──
    for _ in 0..3 {
        assert_eq!(runtime_lock::acquire_or_secondary("test"), Tier::Primary);
    }

    // ── Diagnostic body: PID + recent timestamp + role we passed. ──
    let holder = runtime_lock::current_holder().expect("holder body present");
    assert_eq!(holder.pid, std::process::id());
    assert_eq!(holder.role, "test");
    assert!(holder.started_at_unix > 1_735_689_600); // 2025-01-01

    // ── Parallel callers in one process all see the same tier. File
    //    locks are per-process, so contention is not directly
    //    observable here — see PR manual smoke checklist for the
    //    cross-process scenarios. ──
    let observed = Arc::new(std::sync::Mutex::new(Vec::<Tier>::new()));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let observed = observed.clone();
        handles.push(std::thread::spawn(move || {
            let t = runtime_lock::acquire_or_secondary("parallel-test");
            observed.lock().unwrap().push(t);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let observed = observed.lock().unwrap();
    for &t in observed.iter() {
        assert_eq!(t, Tier::Primary, "every thread must see the same tier");
    }
}
