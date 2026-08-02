//! Rewrite stream event JSON before it leaves the server for HTTP clients.
//!
//! Tools emit `tool_result` events with `media_items[]` containing BOTH a
//! logical `url` (`/api/attachments/{sid}/{filename}`) and a server-side
//! absolute `localPath`. The Tauri sink ships both fields untouched — the
//! frontend prefers `localPath` via `convertFileSrc`. HTTP sinks strip
//! `localPath` so server filesystem paths never reach web clients. Browser
//! media requests authenticate with the server's HttpOnly session cookie.
//!
//! This module is kept cheap: if the event isn't a `tool_result` with
//! `media_items` it returns the original string untouched — no allocation.

use serde_json::Value;

/// Rewrite a stream event JSON string for HTTP delivery. Returns the input
/// untouched when no media items are present.
pub fn rewrite_event_for_http(event_json: &str) -> String {
    // Fast path: skip non-matching events without JSON parse.
    if !event_json.contains("\"media_items\"") {
        return event_json.to_string();
    }
    let mut value: Value = match serde_json::from_str(event_json) {
        Ok(v) => v,
        Err(_) => return event_json.to_string(),
    };
    if !apply_http_rewrite(&mut value) {
        return event_json.to_string();
    }
    serde_json::to_string(&value).unwrap_or_else(|_| event_json.to_string())
}

/// Mutate a parsed event `Value` in-place for HTTP delivery. Returns `true`
/// when any `media_items` entry was rewritten.
fn apply_http_rewrite(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let Some(items) = obj.get_mut("media_items").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let mut mutated = false;
    for item in items.iter_mut() {
        if let Some(item_obj) = item.as_object_mut() {
            // Strip server-side absolute path — web clients must never see it.
            if item_obj.remove("localPath").is_some() {
                mutated = true;
            }
        }
    }
    mutated
}

/// For `chat:stream_delta` / `channel:stream_delta` envelopes forwarded on
/// `/ws/events`: the inner stream event lives at `payload.event` as a
/// stringified JSON. Rewrite that nested string in-place.
pub fn rewrite_envelope_event_for_http(envelope: &mut Value) -> bool {
    let Some(payload) = envelope.get_mut("payload").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    let Some(event_field) = payload.get_mut("event") else {
        return false;
    };
    let Some(inner) = event_field.as_str() else {
        return false;
    };
    // Fast path: skip if no media_items marker.
    if !inner.contains("\"media_items\"") {
        return false;
    }
    let Ok(mut inner_val) = serde_json::from_str::<Value>(inner) else {
        return false;
    };
    if !apply_http_rewrite(&mut inner_val) {
        return false;
    }
    if let Ok(rewritten) = serde_json::to_string(&inner_val) {
        *event_field = Value::String(rewritten);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_matching_event_passes_through() {
        let input = r#"{"type":"text_delta","content":"hello"}"#;
        assert_eq!(rewrite_event_for_http(input), input);
    }

    #[test]
    fn strips_local_path_without_putting_credentials_in_urls() {
        let input = r#"{"type":"tool_result","name":"send_attachment","media_items":[{"url":"/api/attachments/s1/foo.pdf","localPath":"/Users/u/.hope-agent/attachments/s1/foo.pdf","name":"foo.pdf","mimeType":"application/pdf","sizeBytes":42,"kind":"file"}]}"#;
        let out = rewrite_event_for_http(input);
        assert!(!out.contains("localPath"));
        assert!(out.contains("/api/attachments/s1/foo.pdf"));
        assert!(!out.contains("token="));
    }

    #[test]
    fn no_api_key_only_strips_local_path() {
        let input = r#"{"type":"tool_result","media_items":[{"url":"/api/attachments/s1/foo.pdf","localPath":"/abs/path","name":"f","mimeType":"x","sizeBytes":1,"kind":"file"}]}"#;
        let out = rewrite_event_for_http(input);
        assert!(!out.contains("localPath"));
        assert!(out.contains("/api/attachments/s1/foo.pdf"));
        assert!(!out.contains("token="));
    }

    #[test]
    fn envelope_rewrites_nested_event() {
        let inner = r#"{"type":"tool_result","media_items":[{"url":"/api/attachments/s/f","localPath":"/abs","name":"n","mimeType":"x","sizeBytes":1,"kind":"file"}]}"#;
        let envelope_json = serde_json::json!({
            "name": "chat:stream_delta",
            "payload": { "sessionId": "s", "seq": 1, "event": inner },
        });
        let mut env = envelope_json;
        let changed = rewrite_envelope_event_for_http(&mut env);
        assert!(changed);
        let event_str = env["payload"]["event"].as_str().unwrap();
        assert!(!event_str.contains("localPath"));
        assert!(!event_str.contains("token="));
    }
}
