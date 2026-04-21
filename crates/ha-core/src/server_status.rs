//! Process-wide runtime status of the embedded HTTP/WS server.
//!
//! WS counters are `Arc<AtomicU32>` so connection handlers stay lock-free
//! on the hot path; the `RwLock` only guards the rarely-mutated metadata
//! (addr / started_at / error).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;

pub struct ServerStatus {
    pub started_at: Option<Instant>,
    pub bound_addr: Option<SocketAddr>,
    pub startup_error: Option<String>,
    pub events_ws_count: Arc<AtomicU32>,
    pub chat_ws_count: Arc<AtomicU32>,
}

impl ServerStatus {
    fn new() -> Self {
        Self {
            started_at: None,
            bound_addr: None,
            startup_error: None,
            events_ws_count: Arc::new(AtomicU32::new(0)),
            chat_ws_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

static SERVER_STATUS: OnceLock<Arc<RwLock<ServerStatus>>> = OnceLock::new();
// Cache the two counter Arcs so WS handlers don't retake the outer RwLock
// on every connection — only the first call hits the full init path.
static EVENTS_WS_COUNTER: OnceLock<Arc<AtomicU32>> = OnceLock::new();
static CHAT_WS_COUNTER: OnceLock<Arc<AtomicU32>> = OnceLock::new();

pub fn get_or_init() -> Arc<RwLock<ServerStatus>> {
    SERVER_STATUS
        .get_or_init(|| Arc::new(RwLock::new(ServerStatus::new())))
        .clone()
}

pub fn mark_started(addr: SocketAddr) {
    let handle = get_or_init();
    if let Ok(mut s) = handle.write() {
        s.started_at = Some(Instant::now());
        s.bound_addr = Some(addr);
        s.startup_error = None;
    };
}

pub fn mark_failed(err: String) {
    let handle = get_or_init();
    if let Ok(mut s) = handle.write() {
        s.startup_error = Some(err);
        s.bound_addr = None;
        s.started_at = None;
    };
}

fn counter_from<F>(cache: &'static OnceLock<Arc<AtomicU32>>, pick: F) -> Arc<AtomicU32>
where
    F: FnOnce(&ServerStatus) -> Arc<AtomicU32>,
{
    cache
        .get_or_init(|| {
            get_or_init()
                .read()
                .ok()
                .map(|s| pick(&s))
                .unwrap_or_else(|| Arc::new(AtomicU32::new(0)))
        })
        .clone()
}

pub fn events_ws_counter() -> Arc<AtomicU32> {
    counter_from(&EVENTS_WS_COUNTER, |s| s.events_ws_count.clone())
}

pub fn chat_ws_counter() -> Arc<AtomicU32> {
    counter_from(&CHAT_WS_COUNTER, |s| s.chat_ws_count.clone())
}

pub struct StatusSnapshot {
    pub bound_addr: Option<String>,
    pub started_at_unix_secs: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub startup_error: Option<String>,
    pub events_ws_count: u32,
    pub chat_ws_count: u32,
}

/// Snapshot the current state. Server-level metrics only; in-flight chat
/// session counts are merged in by [`runtime_status_json`] for endpoints.
pub fn snapshot() -> StatusSnapshot {
    let handle = get_or_init();
    let (bound, started_at, error) = match handle.read().ok() {
        Some(s) => (
            s.bound_addr.map(|a| a.to_string()),
            s.started_at,
            s.startup_error.clone(),
        ),
        None => (None, None, None),
    };

    // Convert monotonic Instant → wall-clock unix secs so UIs can render
    // "since 14:02" instead of a process-relative number.
    let (started_unix, uptime) = match started_at {
        Some(i) => {
            let elapsed = i.elapsed().as_secs();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (Some(now.saturating_sub(elapsed)), Some(elapsed))
        }
        None => (None, None),
    };

    StatusSnapshot {
        bound_addr: bound,
        started_at_unix_secs: started_unix,
        uptime_secs: uptime,
        startup_error: error,
        events_ws_count: events_ws_counter().load(Ordering::Relaxed),
        chat_ws_count: chat_ws_counter().load(Ordering::Relaxed),
    }
}

/// Composed runtime-status JSON shared by `GET /api/server/status` and the
/// `get_server_runtime_status` Tauri command. Both transports serialize the
/// same shape so front-end `Transport` calls route identically.
///
/// `local_desktop_client` is `true` when the caller is the embedded Tauri
/// shell (whose webview talks to the backend via IPC + EventBus, not
/// WebSocket) — it's surfaced so the UI can add this app itself to the
/// "active connections" tally. HTTP callers pass `false`.
///
/// `activeChatStreams` is a back-compat alias for `activeChatCounts.total`
/// (meaning changed from "WS subscribers" to "in-flight chat engines").
pub fn runtime_status_json(local_desktop_client: bool) -> serde_json::Value {
    let snap = snapshot();
    let counts = crate::chat_engine::stream_seq::active_counts();
    serde_json::json!({
        "boundAddr": snap.bound_addr,
        "startedAt": snap.started_at_unix_secs,
        "uptimeSecs": snap.uptime_secs,
        "startupError": snap.startup_error,
        "eventsWsCount": snap.events_ws_count,
        "chatWsCount": snap.chat_ws_count,
        "localDesktopClient": local_desktop_client,
        "activeChatStreams": counts.total,
        "activeChatCounts": counts,
    })
}

/// Increments the given counter on construction, decrements on drop —
/// covers normal exit, early return, client disconnect, and panic unwind.
pub struct WsConnectionGuard(Arc<AtomicU32>);

impl WsConnectionGuard {
    pub fn new(counter: Arc<AtomicU32>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_increments_and_decrements() {
        let counter = Arc::new(AtomicU32::new(0));
        {
            let _g = WsConnectionGuard::new(counter.clone());
            assert_eq!(counter.load(Ordering::Relaxed), 1);
            let _g2 = WsConnectionGuard::new(counter.clone());
            assert_eq!(counter.load(Ordering::Relaxed), 2);
        }
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn mark_started_then_failed_clears_addr() {
        let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        mark_started(addr);
        let snap = snapshot();
        assert!(snap.bound_addr.is_some());
        assert!(snap.startup_error.is_none());

        mark_failed("boom".to_string());
        let snap = snapshot();
        assert!(snap.bound_addr.is_none());
        assert_eq!(snap.startup_error.as_deref(), Some("boom"));

        // Recovery path
        mark_started(addr);
        let snap = snapshot();
        assert!(snap.startup_error.is_none());
    }
}
