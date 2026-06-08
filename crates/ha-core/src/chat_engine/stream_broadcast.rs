//! EventBus broadcast of chat stream deltas. Runs alongside the per-call
//! `EventSink` so a reloaded frontend (dead Channel / WebSocket) can keep
//! receiving events via `listen("chat:stream_delta")`.

use super::stream_seq;
use crate::globals;
use crate::session::{ChatTurnInterruptReason, ChatTurnStatus};
use serde_json::json;

/// Event name the frontend listens on for resumable stream deltas.
pub const EVENT_CHAT_STREAM_DELTA: &str = "chat:stream_delta";

/// Event name emitted once at `run_chat` completion.
pub const EVENT_CHAT_STREAM_END: &str = "chat:stream_end";

/// Event emitted once a user-facing turn id is known.
pub const EVENT_CHAT_TURN_STARTED: &str = "chat:turn_started";

/// Event emitted when a turn's persisted status changes but the stream is
/// not terminal yet, e.g. `running` -> `cancelling` after the user presses Stop.
pub const EVENT_CHAT_TURN_STATUS: &str = "chat:turn_status";

/// Counterpart for IM channel worker sessions — same envelope shape
/// (`{sessionId, event}`), different name so subscribers can filter.
pub const EVENT_CHANNEL_STREAM_DELTA: &str = "channel:stream_delta";

/// Inject `_oc_seq` and `_oc_stream_id` into a serialized stream event and
/// return `(enveloped_string, seq, stream_id)`. If the input isn't valid JSON
/// or isn't an object, return the original event — defensive, lets the
/// frontend still see the event (without dedup guarantee) rather than
/// dropping it.
pub fn inject_seq(
    session_id: &str,
    event: &str,
    turn_id: Option<&str>,
) -> (String, u64, Option<String>) {
    let (seq, stream_id) = stream_seq::next_seq_and_stream(session_id);

    // Fast path: `event` is a compact JSON object produced upstream by
    // `Value::to_string()`. Splice the `_oc_*` keys in just before the closing
    // `}` instead of paying a full serde parse + re-serialize per token. The
    // frontend dedup (useChatStream) accesses `_oc_seq` / `_oc_stream_id` by
    // key name, not position, so append order is transparent.
    let trimmed = event.trim_end();
    let Some(close) = trimmed.rfind('}') else {
        // Not a JSON object — preserve the old defensive verbatim fallback.
        return (event.to_string(), seq, stream_id);
    };
    if !trimmed.trim_start().starts_with('{') {
        return (event.to_string(), seq, stream_id);
    }

    // Empty object `{}` → no leading comma (matches the old path's
    // `{"_oc_seq":N}` output for an empty input object).
    let needs_comma = !event[..close].trim_end().ends_with('{');

    let mut out = String::with_capacity(event.len() + 96);
    out.push_str(&event[..close]); // everything up to (not including) the close `}`
    if needs_comma {
        out.push(',');
    }
    out.push_str("\"_oc_seq\":");
    out.push_str(&seq.to_string()); // u64 — no escaping needed
    if let Some(id) = stream_id.as_deref() {
        out.push_str(",\"_oc_stream_id\":");
        // serde escapes the scalar string — zero object traversal, injection-safe.
        out.push_str(&serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string()));
    }
    if let Some(id) = turn_id {
        out.push_str(",\"_oc_turn_id\":");
        out.push_str(&serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string()));
    }
    out.push_str(&event[close..]); // the close `}` and anything after it
    (out, seq, stream_id)
}

/// Emit `chat:stream_delta` to the EventBus. Caller has already obtained the
/// enveloped event string + seq via [`inject_seq`]; pass them straight through
/// so the primary sink and this broadcast share identical payloads.
pub fn broadcast_delta(session_id: &str, event: &str, seq: u64, stream_id: Option<&str>) {
    if let Some(bus) = globals::get_event_bus() {
        bus.emit(
            EVENT_CHAT_STREAM_DELTA,
            json!({
                "sessionId": session_id,
                "seq": seq,
                "streamId": stream_id,
                "event": event,
            }),
        );
    }
}

pub fn broadcast_turn_started(session_id: &str, turn_id: &str, stream_id: Option<&str>) {
    if let Some(bus) = globals::get_event_bus() {
        bus.emit(
            EVENT_CHAT_TURN_STARTED,
            json!({
                "sessionId": session_id,
                "turnId": turn_id,
                "streamId": stream_id,
            }),
        );
    }
}

pub fn broadcast_turn_status(
    session_id: &str,
    turn_id: &str,
    status: ChatTurnStatus,
    interrupt_reason: Option<ChatTurnInterruptReason>,
) {
    if let Some(bus) = globals::get_event_bus() {
        bus.emit(
            EVENT_CHAT_TURN_STATUS,
            json!({
                "sessionId": session_id,
                "turnId": turn_id,
                "status": status.as_str(),
                "interruptReason": interrupt_reason.map(|r| r.as_str()),
            }),
        );
    }
}

/// Emit `chat:stream_end` once when `run_chat` completes (success or failure).
pub fn broadcast_stream_end(
    session_id: &str,
    stream_id: Option<&str>,
    turn_id: Option<&str>,
    status: Option<ChatTurnStatus>,
    interrupt_reason: Option<ChatTurnInterruptReason>,
    error: Option<&str>,
) {
    if let Some(bus) = globals::get_event_bus() {
        bus.emit(
            EVENT_CHAT_STREAM_END,
            json!({
                "sessionId": session_id,
                "streamId": stream_id,
                "turnId": turn_id,
                "status": status.map(|s| s.as_str()),
                "interruptReason": interrupt_reason.map(|r| r.as_str()),
                "error": error,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference impl = the old parse → insert → serialize path. Used to prove
    /// the new string-splice `inject_seq` is semantically byte-equivalent.
    fn reference_envelope(
        event: &str,
        seq: u64,
        stream_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Option<String> {
        match serde_json::from_str::<serde_json::Value>(event) {
            Ok(serde_json::Value::Object(mut map)) => {
                map.insert("_oc_seq".into(), json!(seq));
                if let Some(id) = stream_id {
                    map.insert("_oc_stream_id".into(), json!(id));
                }
                if let Some(id) = turn_id {
                    map.insert("_oc_turn_id".into(), json!(id));
                }
                Some(serde_json::Value::Object(map).to_string())
            }
            _ => None,
        }
    }

    #[test]
    fn inject_seq_splice_matches_reference_semantics() {
        // Unregistered session → next_seq_and_stream returns (0, None),
        // deterministic for the assertion.
        let sid = "test-inject-seq-unregistered-session-xyz";
        let cases = [
            r#"{"type":"text_delta","text":"hi"}"#,
            r#"{"type":"text_delta","text":"quote \" and \\ backslash"}"#,
            r#"{"text":"a close } brace inside a string"}"#,
            r#"{"type":"tool_call","call_id":"c1","content":[{"a":1},{"b":2}]}"#,
            r#"{"a":{"b":{"c":1}}}"#,
            r#"{}"#,
        ];
        for ev in cases {
            for turn in [None, Some("turn-123")] {
                let (out, seq, sid_out) = inject_seq(sid, ev, turn);
                assert_eq!(seq, 0);
                assert_eq!(sid_out, None);
                let want = reference_envelope(ev, 0, None, turn).expect("object");
                let got_v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| {
                    panic!("spliced output not valid JSON for {ev}: {e}\nout={out}")
                });
                let want_v: serde_json::Value = serde_json::from_str(&want).unwrap();
                assert_eq!(got_v, want_v, "mismatch for {ev} turn={turn:?}\n got={out}");
            }
        }
    }

    #[test]
    fn inject_seq_passes_through_non_object() {
        assert_eq!(inject_seq("s", "not json at all", None).0, "not json at all");
        assert_eq!(inject_seq("s", "[1,2,3]", None).0, "[1,2,3]");
    }
}
