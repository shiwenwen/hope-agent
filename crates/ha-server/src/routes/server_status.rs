//! `/api/server/status` — unauthenticated (mirrors `/api/health`) so the
//! Transport layer can probe without an API key. No secrets in the payload.

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::AppContext;

pub async fn server_status(State(ctx): State<Arc<AppContext>>) -> Json<Value> {
    let snap = ha_core::server_status::snapshot();
    let active_chat_streams = ctx.chat_streams.active_session_count().await;

    Json(json!({
        "boundAddr": snap.bound_addr,
        "startedAt": snap.started_at_unix_secs,
        "uptimeSecs": snap.uptime_secs,
        "startupError": snap.startup_error,
        "eventsWsCount": snap.events_ws_count,
        "chatWsCount": snap.chat_ws_count,
        "activeChatStreams": active_chat_streams,
    }))
}
