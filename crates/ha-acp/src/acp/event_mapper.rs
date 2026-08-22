//! Maps Hope Agent Agent events to ACP session update notifications.
//!
//! The Agent's `on_delta` callback emits JSON strings with typed events.
//! This module parses those events and converts them to ACP `session/update`
//! notifications via the NDJSON transport.

use serde_json::Value;

use crate::acp::types::{
    session_update_params, AcpProtocolVersion, JsonRpcNotification, SessionUpdate, TextContent,
    ToolCallContent,
};

/// Parse an Agent event JSON string and produce an ACP session update notification.
/// Returns None for events that don't map to ACP updates.
pub fn map_agent_event(
    protocol_version: &AcpProtocolVersion,
    session_id: &str,
    message_id: &str,
    event_json: &str,
) -> Option<JsonRpcNotification> {
    let event: Value = serde_json::from_str(event_json).ok()?;
    let event_type = event.get("type")?.as_str()?;

    let update = match event_type {
        "text_delta" => {
            let text = event.get("content")?.as_str()?.to_string();
            SessionUpdate::AgentMessageChunk {
                message_id: Some(message_id.to_string()),
                content: TextContent::new(text),
            }
        }
        "thinking_delta" => {
            let text = event.get("content")?.as_str()?.to_string();
            SessionUpdate::AgentThoughtChunk {
                content: TextContent::new(text),
            }
        }
        "tool_call" => {
            let call_id = event.get("call_id")?.as_str()?.to_string();
            let name = event.get("name")?.as_str()?.to_string();
            let args_str = event
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let raw_input = serde_json::from_str::<Value>(args_str).ok();
            let kind = crate::acp::types::infer_tool_kind(&name);

            SessionUpdate::ToolCall {
                tool_call_id: call_id,
                title: name,
                status: "in_progress".to_string(),
                kind: Some(kind.to_string()),
                raw_input,
            }
        }
        "tool_result" => {
            let call_id = event.get("call_id")?.as_str()?.to_string();
            let result = event
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_error = event
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let status = if is_error { "failed" } else { "completed" };

            // Truncate tool result for ACP notifications (max 8KB)
            let truncated = if result.len() > 8192 {
                let s = ha_core::truncate_utf8(&result, 8192);
                format!("{}...(truncated)", s)
            } else {
                result
            };

            SessionUpdate::ToolCallUpdate {
                tool_call_id: call_id,
                status: status.to_string(),
                content: Some(vec![ToolCallContent {
                    content_type: "content".to_string(),
                    content: TextContent::new(truncated),
                }]),
            }
        }
        "usage" => {
            let input_tokens = event.get("input_tokens").and_then(|v| v.as_u64());
            let output_tokens = event.get("output_tokens").and_then(|v| v.as_u64());
            if input_tokens.is_none() && output_tokens.is_none() {
                return None;
            }

            SessionUpdate::UsageUpdate {
                used: input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0),
                size: 0, // context window size not known here; set by caller
                cost: None,
            }
        }
        _ => return None,
    };

    let params = session_update_params(
        protocol_version,
        session_id,
        serde_json::to_value(&update).ok()?,
    );

    Some(JsonRpcNotification::new("session/update", params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::types::ACP_PROTOCOL_VERSION_V1;

    fn v1() -> AcpProtocolVersion {
        AcpProtocolVersion::V1(ACP_PROTOCOL_VERSION_V1)
    }

    #[test]
    fn v1_stream_event_uses_update_envelope() {
        let notification = map_agent_event(
            &v1(),
            "session-1",
            "message-1",
            r#"{"type":"text_delta","content":"hello"}"#,
        )
        .expect("notification");

        assert_eq!(
            notification.params,
            serde_json::json!({
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": "message-1",
                    "content": {"type": "text", "text": "hello"}
                }
            })
        );
    }

    #[test]
    fn v1_tool_result_uses_content_wrapper() {
        let notification = map_agent_event(
            &v1(),
            "session-1",
            "message-1",
            r#"{"type":"tool_result","call_id":"call-1","result":"done","is_error":false}"#,
        )
        .expect("notification");

        assert_eq!(
            notification.params["update"]["content"],
            serde_json::json!([{
                "type": "content",
                "content": {"type": "text", "text": "done"}
            }])
        );
    }
}
