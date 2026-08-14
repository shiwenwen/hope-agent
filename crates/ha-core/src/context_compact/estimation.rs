// ── Token Estimation ──

use super::types::{ToolResultLocator, ToolResultUnit};
use super::IMAGE_CHAR_ESTIMATE;
use serde_json::Value;
use std::collections::HashMap;

/// Model-neutral compatibility estimate. New model-aware callers should use
/// `token_accounting::TokenAccountingService` directly; this wrapper remains
/// for pure helpers that have no active Provider/model snapshot.
pub fn estimate_tokens(value: &Value) -> u32 {
    let values = std::slice::from_ref(value);
    let request = crate::token_accounting::TokenCountRequest {
        provider: crate::token_accounting::ProviderFamily::Unknown,
        model: "model-neutral",
        request_shape: crate::token_accounting::RequestShape::Json,
        stable_prompt: "",
        dynamic_prompt: "",
        history: values,
        eager_tool_schemas: &[],
        activated_tool_schemas: &[],
    };
    crate::token_accounting::service()
        .count_local(&request)
        .upper_bound
        .min(u64::from(u32::MAX)) as u32
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
    let request = crate::token_accounting::TokenCountRequest {
        provider: crate::token_accounting::ProviderFamily::Unknown,
        model: "model-neutral",
        request_shape: crate::token_accounting::RequestShape::Json,
        stable_prompt: system_prompt,
        dynamic_prompt: "",
        history: messages,
        eager_tool_schemas: &[],
        activated_tool_schemas: &[],
    };
    crate::token_accounting::service()
        .count_local(&request)
        .upper_bound
        .saturating_add(u64::from(max_output_tokens))
        .min(u64::from(u32::MAX)) as u32
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
    let request = crate::token_accounting::TokenCountRequest {
        provider: crate::token_accounting::ProviderFamily::Unknown,
        model: "model-neutral",
        request_shape: crate::token_accounting::RequestShape::Json,
        stable_prompt: system_prompt,
        dynamic_prompt: "",
        history: messages,
        eager_tool_schemas: tool_schemas,
        activated_tool_schemas: &[],
    };
    crate::token_accounting::service()
        .count_local(&request)
        .upper_bound
        .saturating_add(u64::from(max_output_tokens))
        .min(u64::from(u32::MAX)) as u32
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

/// Enumerate provider-level tool result units without collapsing an Anthropic
/// user message that contains several parallel `tool_result` blocks.
pub(crate) fn tool_result_units(msg: &Value) -> Vec<ToolResultUnit> {
    let role = message_role(msg);
    let msg_type = message_type(msg);

    if role == Some("tool") {
        return vec![ToolResultUnit {
            locator: ToolResultLocator::OpenAiChatContent,
            call_id: msg
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            direct_tool_name: msg.get("name").and_then(Value::as_str).map(str::to_string),
            text: msg
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string),
        }];
    }

    if msg_type == Some("function_call_output") {
        return vec![ToolResultUnit {
            locator: ToolResultLocator::OpenAiResponsesOutput,
            call_id: msg
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            direct_tool_name: None,
            text: msg
                .get("output")
                .and_then(Value::as_str)
                .map(str::to_string),
        }];
    }

    if role != Some("user") {
        return Vec::new();
    }

    msg.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(block_index, block)| {
            (block.get("type").and_then(Value::as_str) == Some("tool_result")).then(|| {
                let text = match block.get("content") {
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(Value::Array(content_blocks)) => {
                        let text = content_blocks
                            .iter()
                            .filter_map(|content_block| {
                                (content_block.get("type").and_then(Value::as_str) == Some("text"))
                                    .then(|| content_block.get("text").and_then(Value::as_str))
                                    .flatten()
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        (!text.is_empty()).then_some(text)
                    }
                    _ => None,
                };
                ToolResultUnit {
                    locator: ToolResultLocator::AnthropicBlock(block_index),
                    call_id: block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    direct_tool_name: None,
                    text,
                }
            })
        })
        .collect()
}

/// Extract one result unit's tool name using the call-id map when needed.
pub(super) fn get_tool_name_for_result_unit(
    unit: &ToolResultUnit,
    id_to_name: &HashMap<String, String>,
) -> Option<String> {
    unit.direct_tool_name.clone().or_else(|| {
        unit.call_id
            .as_ref()
            .and_then(|id| id_to_name.get(id))
            .cloned()
    })
}

/// Read one result unit again from a message using its stable locator.
pub(super) fn get_tool_result_unit_text(msg: &Value, locator: ToolResultLocator) -> Option<String> {
    tool_result_units(msg)
        .into_iter()
        .find(|unit| unit.locator == locator)
        .and_then(|unit| unit.text)
}

/// Replace only the textual payload of one result unit. For Anthropic content
/// arrays this preserves non-text blocks (images/media) and replaces text blocks
/// in place instead of collapsing the complete `tool_result.content` value.
pub(crate) fn set_tool_result_unit_text(
    msg: &mut Value,
    locator: ToolResultLocator,
    new_text: &str,
) -> bool {
    match locator {
        ToolResultLocator::OpenAiChatContent => {
            if message_role(msg) == Some("tool") {
                msg["content"] = Value::String(new_text.to_string());
                true
            } else {
                false
            }
        }
        ToolResultLocator::OpenAiResponsesOutput => {
            if message_type(msg) == Some("function_call_output") {
                msg["output"] = Value::String(new_text.to_string());
                true
            } else {
                false
            }
        }
        ToolResultLocator::AnthropicBlock(block_index) => {
            let Some(block) = msg
                .get_mut("content")
                .and_then(Value::as_array_mut)
                .and_then(|blocks| blocks.get_mut(block_index))
            else {
                return false;
            };
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                return false;
            }

            let Some(content) = block.get_mut("content") else {
                return false;
            };
            match content {
                Value::String(text) => {
                    *text = new_text.to_string();
                    true
                }
                Value::Array(content_blocks) => {
                    let text_indices = content_blocks
                        .iter()
                        .enumerate()
                        .filter_map(|(index, content_block)| {
                            (content_block.get("type").and_then(Value::as_str) == Some("text"))
                                .then_some(index)
                        })
                        .collect::<Vec<_>>();
                    let Some(first_text_index) = text_indices.first().copied() else {
                        return false;
                    };
                    content_blocks[first_text_index]["text"] = Value::String(new_text.to_string());
                    for index in text_indices.into_iter().skip(1).rev() {
                        content_blocks.remove(index);
                    }
                    true
                }
                _ => false,
            }
        }
    }
}

/// Compatibility reader for code that still renders a result container as one
/// item. Tier 0/1/2 must use the unit API.
#[cfg(test)]
pub(super) fn get_tool_result_text(msg: &Value) -> Option<String> {
    tool_result_units(msg)
        .into_iter()
        .find_map(|unit| unit.text)
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
pub(crate) fn is_user_message(msg: &Value) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enumerates_each_provider_result_as_one_unit() {
        let chat = json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "name": "read_file",
            "content": "chat result"
        });
        let responses = json!({
            "type": "function_call_output",
            "call_id": "fc_1",
            "output": "responses result"
        });
        let anthropic = json!({
            "role": "user",
            "content": [
                {"type":"tool_result","tool_use_id":"toolu_1","content":"first"},
                {"type":"text","text":"container note"},
                {"type":"tool_result","tool_use_id":"toolu_2","content":"second"}
            ]
        });

        let chat_units = tool_result_units(&chat);
        assert_eq!(chat_units.len(), 1);
        assert_eq!(chat_units[0].call_id.as_deref(), Some("call_1"));
        assert_eq!(chat_units[0].text.as_deref(), Some("chat result"));

        let response_units = tool_result_units(&responses);
        assert_eq!(response_units.len(), 1);
        assert_eq!(response_units[0].call_id.as_deref(), Some("fc_1"));

        let anthropic_units = tool_result_units(&anthropic);
        assert_eq!(anthropic_units.len(), 2);
        assert_eq!(
            anthropic_units[0].locator,
            ToolResultLocator::AnthropicBlock(0)
        );
        assert_eq!(
            anthropic_units[1].locator,
            ToolResultLocator::AnthropicBlock(2)
        );
        assert_eq!(anthropic_units[1].call_id.as_deref(), Some("toolu_2"));
        assert_eq!(anthropic_units[1].text.as_deref(), Some("second"));
    }

    #[test]
    fn anthropic_unit_write_preserves_other_results_and_media_blocks() {
        let mut message = json!({
            "role": "user",
            "content": [
                {
                    "type":"tool_result",
                    "tool_use_id":"toolu_1",
                    "content":[
                        {"type":"text","text":"old first"},
                        {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}},
                        {"type":"text","text":"old second"}
                    ]
                },
                {"type":"tool_result","tool_use_id":"toolu_2","content":"untouched"}
            ]
        });

        assert!(set_tool_result_unit_text(
            &mut message,
            ToolResultLocator::AnthropicBlock(0),
            "replacement"
        ));

        let blocks = message["content"].as_array().unwrap();
        let first_content = blocks[0]["content"].as_array().unwrap();
        assert_eq!(first_content.len(), 2);
        assert_eq!(first_content[0]["text"], "replacement");
        assert_eq!(first_content[1]["type"], "image");
        assert_eq!(blocks[1]["content"], "untouched");
        assert_eq!(blocks[0]["tool_use_id"], "toolu_1");
    }
}
