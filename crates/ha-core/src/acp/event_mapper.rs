//! Maps Hope Agent Agent events to ACP session update notifications.
//!
//! The Agent's `on_delta` callback emits JSON strings with typed events.
//! This module parses those events and converts them to ACP `session/update`
//! notifications via the NDJSON transport.

use serde_json::Value;

use crate::acp::types::{JsonRpcNotification, SessionUpdate, TextContent, ToolCallContent};

/// Parse an Agent event JSON string and produce an ACP session update notification.
/// Returns None for events that don't map to ACP updates.
pub fn map_agent_event(session_id: &str, event_json: &str) -> Option<JsonRpcNotification> {
    let event: Value = serde_json::from_str(event_json).ok()?;
    let event_type = event.get("type")?.as_str()?;

    let update = match event_type {
        "text_delta" => {
            let text = event.get("content")?.as_str()?.to_string();
            SessionUpdate::AgentMessageChunk {
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
                let s = crate::truncate_utf8(&result, 8192);
                format!("{}...(truncated)", s)
            } else {
                result
            };

            SessionUpdate::ToolCallUpdate {
                tool_call_id: call_id,
                status: status.to_string(),
                content: Some(vec![ToolCallContent {
                    content_type: "text".to_string(),
                    content: TextContent::new(truncated),
                }]),
            }
        }
        "usage" => {
            let input_tokens = event
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output_tokens = event
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            SessionUpdate::UsageUpdate {
                used: input_tokens + output_tokens,
                size: 0, // context window size not known here; set by caller
            }
        }
        _ => return None,
    };

    let params = serde_json::json!({
        "sessionId": session_id,
        "sessionUpdate": serde_json::to_value(&update).ok()?,
    });

    Some(JsonRpcNotification::new("session/update", params))
}
