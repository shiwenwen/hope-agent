//! ACP Control Plane — Tauri event emission.

use super::types::{AcpControlEvent, AcpStreamEvent};

/// Tauri global event name.
pub const ACP_CONTROL_EVENT: &str = "acp_control_event";

/// Emit an ACP control event to the frontend.
pub fn emit_acp_event(
    run_id: &str,
    parent_session_id: &str,
    backend_id: &str,
    label: Option<&str>,
    event_type: &str,
    data: serde_json::Value,
) {
    let event = AcpControlEvent {
        event_type: event_type.to_string(),
        run_id: run_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        backend_id: backend_id.to_string(),
        label: label.map(|s| s.to_string()),
        data,
    };

    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = handle.emit(ACP_CONTROL_EVENT, &event);
    }
}

/// Map an `AcpStreamEvent` to a Tauri emission.
pub fn emit_stream_event(
    run_id: &str,
    parent_session_id: &str,
    backend_id: &str,
    label: Option<&str>,
    event: &AcpStreamEvent,
) {
    let (event_type, data) = match event {
        AcpStreamEvent::TextDelta { content } => (
            "text_delta",
            serde_json::json!({ "content": content }),
        ),
        AcpStreamEvent::ThinkingDelta { content } => (
            "thinking_delta",
            serde_json::json!({ "content": content }),
        ),
        AcpStreamEvent::ToolCall {
            tool_call_id,
            name,
            status,
            arguments,
        } => (
            "tool_call",
            serde_json::json!({
                "toolCallId": tool_call_id,
                "name": name,
                "status": status,
                "arguments": arguments,
            }),
        ),
        AcpStreamEvent::ToolResult {
            tool_call_id,
            status,
            result_preview,
        } => (
            "tool_result",
            serde_json::json!({
                "toolCallId": tool_call_id,
                "status": status,
                "resultPreview": result_preview,
            }),
        ),
        AcpStreamEvent::Usage {
            input_tokens,
            output_tokens,
        } => (
            "usage",
            serde_json::json!({
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
            }),
        ),
        AcpStreamEvent::Error { message } => (
            "error",
            serde_json::json!({ "message": message }),
        ),
        AcpStreamEvent::Done { stop_reason } => (
            "done",
            serde_json::json!({ "stopReason": stop_reason }),
        ),
    };

    emit_acp_event(run_id, parent_session_id, backend_id, label, event_type, data);
}
