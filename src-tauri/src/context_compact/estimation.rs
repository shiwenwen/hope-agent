// ── Token Estimation ──

use serde_json::Value;
use super::{CHARS_PER_TOKEN, IMAGE_CHAR_ESTIMATE};

/// Estimate token count for a JSON value using char/4 heuristic.
pub fn estimate_tokens(value: &Value) -> u32 {
    match value {
        Value::String(s) => (s.len() / CHARS_PER_TOKEN) as u32,
        Value::Array(arr) => arr.iter().map(estimate_tokens).sum(),
        Value::Object(obj) => {
            obj.values().map(estimate_tokens).sum::<u32>()
                + obj
                    .keys()
                    .map(|k| (k.len() / CHARS_PER_TOKEN) as u32)
                    .sum::<u32>()
        }
        Value::Number(_) => 1,
        Value::Bool(_) => 1,
        Value::Null => 1,
    }
}

/// Estimate char count for a message, using IMAGE_CHAR_ESTIMATE for images.
pub fn estimate_message_chars(msg: &Value) -> usize {
    if let Some(content) = msg.get("content") {
        match content {
            Value::String(s) => s.len(),
            Value::Array(arr) => arr
                .iter()
                .map(|block| {
                    if let Some(t) = block.get("type").and_then(|t| t.as_str()) {
                        match t {
                            "text" | "tool_result" => block
                                .get("text")
                                .or_else(|| block.get("content"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.len())
                                .unwrap_or(128),
                            "image" | "image_url" => IMAGE_CHAR_ESTIMATE,
                            _ => 128,
                        }
                    } else {
                        128
                    }
                })
                .sum(),
            _ => 128,
        }
    } else if let Some(output) = msg.get("output") {
        // OpenAI Responses format
        output.as_str().map(|s| s.len()).unwrap_or(128)
    } else {
        128
    }
}

/// Estimate total request tokens: system_prompt + messages + max_output.
pub fn estimate_request_tokens(
    system_prompt: &str,
    messages: &[Value],
    max_output_tokens: u32,
) -> u32 {
    let system_tokens = (system_prompt.len() / CHARS_PER_TOKEN) as u32;
    let message_tokens: u32 = messages.iter().map(|m| estimate_tokens(m)).sum();
    system_tokens + message_tokens + max_output_tokens
}

// ── Tool Result Detection (format-agnostic) ──

/// Extract tool name from a message, format-agnostic.
pub(super) fn extract_tool_name(msg: &Value) -> Option<String> {
    // Anthropic: look in preceding assistant message's tool_use blocks
    // For now, extract from the tool_result's own fields if available
    msg.get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

/// Get the text content of a tool result message, format-agnostic.
pub(super) fn get_tool_result_text(msg: &Value) -> Option<String> {
    let role = msg.get("role").and_then(|r| r.as_str());
    let msg_type = msg.get("type").and_then(|t| t.as_str());

    // OpenAI Chat: role=tool, content is string
    if role == Some("tool") {
        return msg.get("content").and_then(|c| c.as_str()).map(|s| s.to_string());
    }

    // OpenAI Responses: type=function_call_output, output is string
    if msg_type == Some("function_call_output") {
        return msg.get("output").and_then(|o| o.as_str()).map(|s| s.to_string());
    }

    // Anthropic: role=user with content array containing tool_result blocks
    if role == Some("user") {
        if let Some(Value::Array(blocks)) = msg.get("content") {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    if let Some(content) = block.get("content") {
                        match content {
                            Value::String(s) => return Some(s.clone()),
                            Value::Array(inner) => {
                                // Array of content blocks — collect text
                                let text: String = inner
                                    .iter()
                                    .filter_map(|b| {
                                        if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                            b.get("text").and_then(|t| t.as_str())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                if !text.is_empty() {
                                    return Some(text);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    None
}

/// Set the text content of a tool result message, format-agnostic.
pub(super) fn set_tool_result_text(msg: &mut Value, new_text: &str) {
    let role = msg.get("role").and_then(|r| r.as_str()).map(|s| s.to_string());
    let msg_type = msg
        .get("type")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());

    // OpenAI Chat: role=tool
    if role.as_deref() == Some("tool") {
        msg["content"] = Value::String(new_text.to_string());
        return;
    }

    // OpenAI Responses: type=function_call_output
    if msg_type.as_deref() == Some("function_call_output") {
        msg["output"] = Value::String(new_text.to_string());
        return;
    }

    // Anthropic: role=user with tool_result blocks
    if role.as_deref() == Some("user") {
        if let Some(Value::Array(blocks)) = msg.get_mut("content") {
            for block in blocks.iter_mut() {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    block["content"] = Value::String(new_text.to_string());
                    return;
                }
            }
        }
    }
}

/// Check if a message is a tool result (any format).
pub(super) fn is_tool_result(msg: &Value) -> bool {
    let role = msg.get("role").and_then(|r| r.as_str());
    let msg_type = msg.get("type").and_then(|t| t.as_str());

    // OpenAI Chat
    if role == Some("tool") {
        return true;
    }
    // OpenAI Responses
    if msg_type == Some("function_call_output") {
        return true;
    }
    // Anthropic: user message containing tool_result blocks
    if role == Some("user") {
        if let Some(Value::Array(blocks)) = msg.get("content") {
            return blocks
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
        }
    }
    false
}

/// Check if a message has role=assistant.
pub(super) fn is_assistant_message(msg: &Value) -> bool {
    msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
}

/// Check if a message has role=user (and is NOT a tool_result container).
pub(super) fn is_user_message(msg: &Value) -> bool {
    let role = msg.get("role").and_then(|r| r.as_str());
    if role != Some("user") {
        return false;
    }
    // Exclude Anthropic tool_result containers
    !is_tool_result(msg)
}

/// Check if a tool name matches any pattern in the deny list.
pub(super) fn is_tool_denied(tool_name: &str, deny_list: &[String]) -> bool {
    let lower = tool_name.to_lowercase();
    deny_list.iter().any(|pattern| {
        let p = pattern.to_lowercase();
        if p.contains('*') {
            // Simple glob: "memory_*" matches "memory_search"
            let parts: Vec<&str> = p.split('*').collect();
            if parts.len() == 2 {
                lower.starts_with(parts[0]) && lower.ends_with(parts[1])
            } else {
                lower == p
            }
        } else {
            lower == p
        }
    })
}
