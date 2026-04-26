//! Integration test for `init_runtime` + `build_app_state` +
//! `start_minimal_background_tasks`.
//!
//! Cargo runs each integration test file in its own binary, so the
//! process-global OnceLocks (SESSION_DB, CRON_DB, APP_LOGGER, …) written
//! by `init_runtime` don't collide with `mcp_e2e.rs` or other tests. We
//! deliberately keep this file to a **single** `#[test]` so we don't
//! race within the binary either: tempdir-backed DBs need to outlive
//! every assertion, and parallel `#[test]`s would tear down their own
//! dirs before the other reads them.

use std::sync::Arc;

use ha_core::globals::{
    APP_LOGGER, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY, CODEX_TOKEN_CACHE,
    CRON_DB, EVENT_BUS, IDLE_EXTRACT_HANDLES, LOG_DB, MEMORY_BACKEND, PROJECT_DB, REASONING_EFFORT,
    SESSION_DB, SUBAGENT_CANCELS,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn init_runtime_full_lifecycle() {
    // Sandbox: redirect the data dir into a tempdir for this test
    // process. HA_DATA_DIR is honored by `paths::root_dir()` directly,
    // unlike HOME/USERPROFILE which `dirs::home_dir()` ignores on Windows.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());

    // Sanity: paths::root_dir() should now point at the tempdir itself
    // (HA_DATA_DIR is used as-is, no `.hope-agent` suffix).
    let root = ha_core::paths::root_dir().expect("root_dir resolves");
    assert_eq!(
        root,
        tmp.path(),
        "expected paths::root_dir() == HA_DATA_DIR, got {root:?}"
    );
    ha_core::paths::ensure_dirs().expect("ensure_dirs in tempdir");

    // ── First call: every OnceLock must be Some afterwards. ──
    ha_core::init_runtime("test");

    assert!(SESSION_DB.get().is_some(), "SESSION_DB");
    assert!(PROJECT_DB.get().is_some(), "PROJECT_DB");
    assert!(LOG_DB.get().is_some(), "LOG_DB");
    assert!(APP_LOGGER.get().is_some(), "APP_LOGGER");
    assert!(MEMORY_BACKEND.get().is_some(), "MEMORY_BACKEND");
    assert!(CRON_DB.get().is_some(), "CRON_DB");
    assert!(SUBAGENT_CANCELS.get().is_some(), "SUBAGENT_CANCELS");
    assert!(IDLE_EXTRACT_HANDLES.get().is_some(), "IDLE_EXTRACT_HANDLES");
    assert!(CHANNEL_CANCELS.get().is_some(), "CHANNEL_CANCELS");
    assert!(CHANNEL_REGISTRY.get().is_some(), "CHANNEL_REGISTRY");
    assert!(CHANNEL_DB.get().is_some(), "CHANNEL_DB");
    assert!(CODEX_TOKEN_CACHE.get().is_some(), "CODEX_TOKEN_CACHE");
    assert!(REASONING_EFFORT.get().is_some(), "REASONING_EFFORT");
    assert!(CACHED_AGENT.get().is_some(), "CACHED_AGENT");
    assert!(EVENT_BUS.get().is_some(), "EVENT_BUS auto-bootstrap");

    // ── Idempotency: second call must not panic and must not reset. ──
    let session_arc_before = SESSION_DB.get().expect("SESSION_DB").clone();
    ha_core::init_runtime("test");
    let session_arc_after = SESSION_DB.get().expect("SESSION_DB").clone();
    assert!(
        Arc::ptr_eq(&session_arc_before, &session_arc_after),
        "init_runtime second call must not replace SESSION_DB Arc"
    );

    // ── build_app_state: returns a coherent AppState whose Arc fields
    //    are pointer-equal to the OnceLocks. The debug_assert!s inside
    //    build_app_state would fail otherwise; we re-check explicitly so
    //    release builds also catch drift. ──
    let state = ha_core::build_app_state();
    assert!(Arc::ptr_eq(
        &state.session_db,
        SESSION_DB.get().expect("SESSION_DB"),
    ));
    assert!(Arc::ptr_eq(
        &state.project_db,
        PROJECT_DB.get().expect("PROJECT_DB"),
    ));
    assert!(Arc::ptr_eq(&state.cron_db, CRON_DB.get().expect("CRON_DB"),));
    assert!(Arc::ptr_eq(
        &state.subagent_cancels,
        SUBAGENT_CANCELS.get().expect("SUBAGENT_CANCELS"),
    ));
    assert!(Arc::ptr_eq(
        &state.channel_cancels,
        CHANNEL_CANCELS.get().expect("CHANNEL_CANCELS"),
    ));
    assert!(Arc::ptr_eq(
        &state.codex_token,
        CODEX_TOKEN_CACHE.get().expect("CODEX_TOKEN_CACHE"),
    ));
    assert!(Arc::ptr_eq(
        &state.reasoning_effort,
        REASONING_EFFORT.get().expect("REASONING_EFFORT"),
    ));
    assert!(Arc::ptr_eq(
        &state.agent,
        CACHED_AGENT.get().expect("CACHED_AGENT"),
    ));
    assert!(Arc::ptr_eq(&state.log_db, LOG_DB.get().expect("LOG_DB"),));

    // ── Minimal background-task variant (ACP shape): must run to
    //    completion without panicking under a tokio runtime. The full
    //    `start_background_tasks` would also kick off the cron scheduler
    //    and a 1-minute dreaming ticker we don't want to leak past the
    //    test, so we only exercise the minimal path here. ──
    ha_core::start_minimal_background_tasks().await;
}
