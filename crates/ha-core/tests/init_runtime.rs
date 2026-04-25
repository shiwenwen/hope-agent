//! Integration test for `init_runtime` + `build_app_state`.
//!
//! `init_runtime` writes process-global OnceLocks (SESSION_DB, CRON_DB,
//! APP_LOGGER, …). Cargo runs each integration test file in its own
//! binary, so these writes don't collide with `mcp_e2e.rs` or other
//! tests. Within this file we only have a single `#[test]` so there's no
//! intra-binary race either.

use std::sync::Arc;

use ha_core::globals::{
    APP_LOGGER, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY, CODEX_TOKEN_CACHE,
    CRON_DB, EVENT_BUS, IDLE_EXTRACT_HANDLES, LOG_DB, MEMORY_BACKEND, PROJECT_DB, REASONING_EFFORT,
    SESSION_DB, SUBAGENT_CANCELS,
};

#[test]
fn init_runtime_full_lifecycle() {
    // Sandbox: redirect ~/.hope-agent into a tempdir for this test process.
    let tmp = tempfile::tempdir().expect("tempdir");
    // dirs::home_dir() reads $HOME on Unix, %USERPROFILE% on Windows.
    // Both: this test only runs in the test binary, the env change dies
    // with the process.
    std::env::set_var("HOME", tmp.path());
    #[cfg(windows)]
    std::env::set_var("USERPROFILE", tmp.path());

    // Sanity: paths::root_dir() should now point inside the tempdir.
    let root = ha_core::paths::root_dir().expect("root_dir resolves");
    assert!(
        root.starts_with(tmp.path()),
        "expected paths::root_dir() inside tempdir, got {root:?}"
    );
    ha_core::paths::ensure_dirs().expect("ensure_dirs in tempdir");

    // ── First call: every OnceLock must be Some afterwards. ──
    ha_core::init_runtime();

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
    ha_core::init_runtime();
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
}
