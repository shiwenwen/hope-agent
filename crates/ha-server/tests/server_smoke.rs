//! End-to-end smoke test: bring up the full server runtime path and
//! confirm `/api/health` returns 200. `#[ignore]` because it touches
//! global OnceLocks (via `ha_core::init_runtime`) and binds a TCP port —
//! both heavier than typical unit tests, both fine for an opt-in
//! `cargo test -- --ignored` lane.
//!
//! Run with `cargo test -p ha-server -- --ignored`.
//!
//! First axum integration test in this crate — expand cautiously. New
//! tests in this style should consider whether they really need a real
//! server (port bind, OnceLock pollution) versus targeted handler tests.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn server_starts_and_serves_health() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", tmp.path());
    #[cfg(windows)]
    std::env::set_var("USERPROFILE", tmp.path());

    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime();

    let session_db = ha_core::globals::SESSION_DB
        .get()
        .expect("SESSION_DB set")
        .clone();
    let project_db = ha_core::globals::PROJECT_DB
        .get()
        .expect("PROJECT_DB set")
        .clone();
    let event_bus = ha_core::get_event_bus().expect("EVENT_BUS set").clone();

    let ctx = Arc::new(ha_server::AppContext {
        session_db,
        project_db,
        event_bus,
        chat_streams: Arc::new(ha_server::ws::chat_stream::ChatStreamRegistry::new()),
        chat_cancels: Arc::new(RwLock::new(HashMap::<String, Arc<AtomicBool>>::new())),
        api_key: None,
    });

    // Bind to an ephemeral port and read the actual address back so the
    // test client knows where to connect.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let actual = listener.local_addr().expect("local_addr");
    let router = ha_server::build_router(ctx);

    let serve = tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    // Tiny grace period for the listener to settle. axum::serve is
    // synchronous about accept() so the bind is already live, but the
    // task scheduler may not have run yet.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let url = format!("http://{}/api/health", actual);
    let resp = reqwest::get(&url).await.expect("GET /api/health");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "expected 200 from /api/health, got {}",
        resp.status()
    );

    serve.abort();
}
