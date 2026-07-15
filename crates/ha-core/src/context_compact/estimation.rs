// ── Token Estimation ──

use super::{CHARS_PER_TOKEN, IMAGE_CHAR_ESTIMATE};
use serde_json::Value;
use std::collections::HashMap;

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
                            "text" | "output_text" | "tool_result" => block
                                .get("text")
                                .or_else(|| block.get("content"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.len())
                                .unwrap_or(128),
                            "thinking" => block
                                .get("thinking")
                                .and_then(|v| v.as_str())
                                .map(|s| s.len())
                                .unwrap_or(128),
                            "image" | "image_url" | "input_image" => IMAGE_CHAR_ESTIMATE,
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

/// Provider-shape request estimate including callable tool schemas. The old
/// estimator is retained for callers that genuinely have no tools (manual
/// summaries and one-shot automation).
pub fn estimate_request_tokens_with_tools(
    system_prompt: &str,
    messages: &[Value],
    tool_schemas: &[Value],
    max_output_tokens: u32,
) -> u32 {
    estimate_request_tokens(system_prompt, messages, max_output_tokens)
        .saturating_add(tool_schemas.iter().map(estimate_tokens).sum::<u32>())
}

// ── Tool Result Detection (format-agnostic) ──

pub(super) fn message_type(msg: &Value) -> Option<&str> {
    msg.get("type").and_then(|t| t.as_str())
}

pub(super) fn message_role(msg: &Value) -> Option<&str> {
    msg.get("role").and_then(|r| r.as_str())
}

pub(super) fn has_anthropic_tool_use(msg: &Value) -> bool {
    msg.get("content")
        .and_then(|c| c.as_array())
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        })
}

pub(super) fn has_openai_chat_tool_calls(msg: &Value) -> bool {
    msg.get("tool_calls")
        .and_then(|calls| calls.as_array())
        .is_some_and(|calls| !calls.is_empty())
}

pub(super) fn is_tool_call(msg: &Value) -> bool {
    has_anthropic_tool_use(msg)
        || has_openai_chat_tool_calls(msg)
        || message_type(msg) == Some("function_call")
}

pub(super) fn tool_call_ids(msg: &Value) -> Vec<&str> {
    let mut ids = Vec::new();
    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
        ids.extend(content.iter().filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                b.get("id").and_then(|v| v.as_str())
            } else {
                None
            }
        }));
    }
    if let Some(tool_calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        ids.extend(
            tool_calls
                .iter()
                .filter_map(|tc| tc.get("id").and_then(|v| v.as_str())),
        );
    }
    if message_type(msg) == Some("function_call") {
        if let Some(id) = msg.get("call_id").and_then(|v| v.as_str()) {
            ids.push(id);
        }
    }
    ids
}

pub(super) fn tool_result_ids(msg: &Value) -> Vec<&str> {
    let mut ids = Vec::new();
    if let Some(id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
        ids.push(id);
    }
    if message_type(msg) == Some("function_call_output") {
        if let Some(id) = msg.get("call_id").and_then(|v| v.as_str()) {
            ids.push(id);
        }
    }
    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
        ids.extend(content.iter().filter_map(|b| {
            if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                b.get("tool_use_id").and_then(|v| v.as_str())
            } else {
                None
            }
        }));
    }
    ids
}

pub(super) fn first_tool_result_id(msg: &Value) -> Option<&str> {
    tool_result_ids(msg).into_iter().next()
}

/// Build a map from provider-specific tool call IDs to tool names.
pub(super) fn build_tool_id_to_name_map(messages: &[Value]) -> HashMap<String, String> {
    let mut id_to_name = HashMap::new();

    for msg in messages {
        // Anthropic: content array with tool_use blocks.
        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    if let (Some(id), Some(name)) = (
                        block.get("id").and_then(|v| v.as_str()),
                        block.get("name").and_then(|v| v.as_str()),
                    ) {
                        id_to_name.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }

        // OpenAI Chat: tool_calls array.
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
            for tc in tool_calls {
                if let (Some(id), Some(name)) = (
                    tc.get("id").and_then(|v| v.as_str()),
                    tc.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str()),
                ) {
                    id_to_name.insert(id.to_string(), name.to_string());
                }
            }
        }

        // OpenAI Responses: top-level function_call item.
        if msg.get("type").and_then(|t| t.as_str()) == Some("function_call") {
            if let (Some(id), Some(name)) = (
                msg.get("call_id").and_then(|v| v.as_str()),
                msg.get("name").and_then(|v| v.as_str()),
            ) {
                id_to_name.insert(id.to_string(), name.to_string());
            }
        }
    }

    id_to_name
}

/// Extract a tool result's tool name using the call-id map when needed.
pub(super) fn get_tool_name_for_result(
    msg: &Value,
    id_to_name: &HashMap<String, String>,
) -> Option<String> {
    // OpenAI Chat may carry the tool name directly on role=tool messages.
    if let Some(name) = msg.get("name").and_then(|n| n.as_str()) {
        return Some(name.to_string());
    }

    if let Some(id) = first_tool_result_id(msg) {
        return id_to_name.get(id).cloned();
    }

    // Anthropic: role=user content array with tool_result blocks.
    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                if let Some(id) = block.get("tool_use_id").and_then(|v| v.as_str()) {
                    return id_to_name.get(id).cloned();
                }
            }
        }
    }

    None
}

/// Get the text content of a tool result message, format-agnostic.
pub(super) fn get_tool_result_text(msg: &Value) -> Option<String> {
    let role = message_role(msg);
    let msg_type = message_type(msg);

    // OpenAI Chat: role=tool, content is string
    if role == Some("tool") {
        return msg
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
    }

    // OpenAI Responses: type=function_call_output, output is string
    if msg_type == Some("function_call_output") {
        return msg
            .get("output")
            .and_then(|o| o.as_str())
            .map(|s| s.to_string());
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
    let role = message_role(msg).map(str::to_string);
    let msg_type = message_type(msg).map(str::to_string);

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
    let role = message_role(msg);
    let msg_type = message_type(msg);

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

/// Check if a message has role=user (and is NOT a tool_result container).
pub(super) fn is_user_message(msg: &Value) -> bool {
    let role = message_role(msg);
    if role != Some("user") {
        return false;
    }
    // Exclude Anthropic tool_result containers
    !is_tool_result(msg)
}

/// Check if a tool name matches any pattern in the deny list.
#[allow(dead_code)]
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
