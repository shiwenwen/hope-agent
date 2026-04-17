//! EventBus-backed broadcast of chat stream deltas.
//!
//! Complements the per-call `EventSink` (Tauri IPC Channel / HTTP WebSocket)
//! with a session-scoped EventBus event so a frontend that reloads (closes its
//! Channel) can re-subscribe to the same session's ongoing stream.
//!
//! Each delta carries a per-session monotonic `seq` (see [`stream_seq`]) so the
//! frontend can deduplicate between the primary sink and the EventBus path.

use super::stream_seq;
use crate::globals;
use serde_json::json;

/// Event name the frontend listens on for resumable stream deltas.
pub const EVENT_CHAT_STREAM_DELTA: &str = "chat:stream_delta";

/// Event name emitted once at `run_chat` completion.
pub const EVENT_CHAT_STREAM_END: &str = "chat:stream_end";

/// Inject a `_oc_seq` field into a serialized stream event (JSON string) and
/// return `(enveloped_string, seq)`. If the input isn't valid JSON or isn't an
/// object, return `(event.to_string(), seq)` without injection — defensive,
/// lets the frontend still see the event (without dedup guarantee) rather than
/// dropping it.
pub fn inject_seq(session_id: &str, event: &str) -> (String, u64) {
    let seq = stream_seq::next_seq(session_id);
    match serde_json::from_str::<serde_json::Value>(event) {
        Ok(serde_json::Value::Object(mut map)) => {
            map.insert("_oc_seq".into(), json!(seq));
            let out = serde_json::Value::Object(map).to_string();
            (out, seq)
        }
        _ => (event.to_string(), seq),
    }
}

/// Emit `chat:stream_delta` to the EventBus. Caller has already obtained the
/// enveloped event string + seq via [`inject_seq`]; pass them straight through
/// so the primary sink and this broadcast share identical payloads.
pub fn broadcast_delta(session_id: &str, event: &str, seq: u64) {
    if let Some(bus) = globals::get_event_bus() {
        bus.emit(
            EVENT_CHAT_STREAM_DELTA,
            json!({
                "sessionId": session_id,
                "seq": seq,
                "event": event,
            }),
        );
    }
}

/// Emit `chat:stream_end` once when `run_chat` completes (success or failure).
pub fn broadcast_stream_end(session_id: &str) {
    if let Some(bus) = globals::get_event_bus() {
        bus.emit(
            EVENT_CHAT_STREAM_END,
            json!({
                "sessionId": session_id,
            }),
        );
    }
}
