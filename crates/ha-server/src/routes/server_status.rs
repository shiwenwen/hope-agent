//! `/api/server/status` — unauthenticated (mirrors `/api/health`) so the
//! Transport layer can probe without an API key. No secrets in the payload.

use axum::Json;
use serde_json::{json, Value};

pub async fn server_status() -> Json<Value> {
    let snap = ha_core::server_status::snapshot();
    let counts = ha_core::chat_engine::stream_seq::active_counts();

    Json(json!({
        "boundAddr": snap.bound_addr,
        "startedAt": snap.started_at_unix_secs,
        "uptimeSecs": snap.uptime_secs,
        "startupError": snap.startup_error,
        "eventsWsCount": snap.events_ws_count,
        "chatWsCount": snap.chat_ws_count,
        // Legacy field kept for payload compatibility. Meaning changed:
        // now reflects in-flight chat engines across desktop / HTTP / channel,
        // not WebSocket subscribers.
        "activeChatStreams": counts.total,
        "activeChatCounts": counts,
    }))
}
