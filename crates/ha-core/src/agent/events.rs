use serde_json::{json, Value};

use crate::attachments::MediaItem;

use super::types::ChatUsage;

pub(super) fn emit_event(on_delta: &(impl Fn(&str) + Send), event: &serde_json::Value) {
    if let Ok(json_str) = serde_json::to_string(event) {
        on_delta(&json_str);
    }
}

pub(super) fn emit_text_delta(on_delta: &(impl Fn(&str) + Send), text: &str) {
    emit_event(
        on_delta,
        &json!({
            "type": "text_delta",
            "content": text,
        }),
    );
}

pub(super) fn emit_tool_call(
    on_delta: &(impl Fn(&str) + Send),
    call_id: &str,
    name: &str,
    arguments: &str,
) {
    emit_event(
        on_delta,
        &json!({
            "type": "tool_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        }),
    );
}

/// Structured media items prefix — the single unified attachment channel for
/// tool outputs (image_generate, send_attachment, future media tools).
/// Carries filename, MIME, size, kind, `local_path`, and optional caption
/// so all downstream consumers (Tauri FileCard, HTTP download route, IM
/// dispatcher) share one shape.
pub(crate) const MEDIA_ITEMS_PREFIX: &str = "__MEDIA_ITEMS__";

/// Extract structured media items from a tool result string.
/// Returns (clean_result, media_items).
/// If the result starts with `__MEDIA_ITEMS__[...]`, the JSON array is parsed and removed.
pub(super) fn extract_media_items(result: &str) -> (String, Vec<MediaItem>) {
    if let Some(rest) = result.strip_prefix(MEDIA_ITEMS_PREFIX) {
        if let Some((json_line, text)) = rest.split_once('\n') {
            if let Ok(items) = serde_json::from_str::<Vec<MediaItem>>(json_line) {
                return (text.to_string(), items);
            }
        }
    }
    (result.to_string(), Vec::new())
}

pub(super) fn emit_tool_result(
    on_delta: &(impl Fn(&str) + Send),
    call_id: &str,
    name: &str,
    result: &str,
    duration_ms: u64,
    is_error: bool,
    media_items: &[MediaItem],
    tool_metadata: Option<&serde_json::Value>,
) {
    let mut event = json!({
        "type": "tool_result",
        "call_id": call_id,
        "name": name,
        "result": result,
        "duration_ms": duration_ms,
        "is_error": is_error,
    });
    if !media_items.is_empty() {
        event["media_items"] = json!(media_items);
    }
    if let Some(md) = tool_metadata {
        event["tool_metadata"] = md.clone();
    }
    emit_event(on_delta, &event);
}

/// Build tool result content for Anthropic Messages API.
/// Detects `__IMAGE_BASE64__` markers and returns a content array with image + text blocks.
pub(super) fn build_anthropic_tool_result_content(result: &str) -> serde_json::Value {
    let Some(parsed) = crate::tools::image_markers::parse_image_markers(result) else {
        return json!(result);
    };

    let mut content = Vec::new();
    if !parsed.leading_text.is_empty() {
        content.push(json!({"type": "text", "text": parsed.leading_text}));
    }
    for m in &parsed.markers {
        let Ok(b64) = crate::tools::image_markers::encode_marker_image(m) else {
            return json!(result);
        };
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": m.mime,
                "data": b64
            }
        }));
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            &m.text
        };
        content.push(json!({"type": "text", "text": text}));
    }
    json!(content)
}

/// Build tool result content for OpenAI Chat Completions API.
/// Returns a content array with `image_url` (data URI) + `text` blocks when images are detected.
pub(super) fn build_openai_chat_tool_result_content(result: &str) -> serde_json::Value {
    let Some(parsed) = crate::tools::image_markers::parse_image_markers(result) else {
        return json!(result);
    };

    let mut content = Vec::new();
    if !parsed.leading_text.is_empty() {
        content.push(json!({"type": "text", "text": parsed.leading_text}));
    }
    for m in &parsed.markers {
        let Ok(b64) = crate::tools::image_markers::encode_marker_image(m) else {
            return json!(result);
        };
        let data_uri = format!("data:{};base64,{}", m.mime, b64);
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": data_uri }
        }));
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            &m.text
        };
        content.push(json!({"type": "text", "text": text}));
    }
    json!(content)
}

/// Build tool result for OpenAI Responses API (`function_call_output`).
/// The `output` field only accepts a string, so when images are detected,
/// returns `(clean_text, Vec<image_input_items>)` where each image item
/// should be appended to the input array as a separate user message.
pub(super) fn build_responses_tool_result(result: &str) -> (String, Vec<serde_json::Value>) {
    let Some(parsed) = crate::tools::image_markers::parse_image_markers(result) else {
        return (result.to_string(), Vec::new());
    };

    // Build combined text output for the function_call_output field
    let mut text_parts = Vec::new();
    if !parsed.leading_text.is_empty() {
        text_parts.push(parsed.leading_text.to_string());
    }
    for m in &parsed.markers {
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            &m.text
        };
        text_parts.push(text.to_string());
    }
    let combined_text = text_parts.join("\n");

    // Build one user message per image for the input array
    let mut image_items = Vec::new();
    for (i, m) in parsed.markers.iter().enumerate() {
        let Ok(b64) = crate::tools::image_markers::encode_marker_image(m) else {
            return (result.to_string(), Vec::new());
        };
        let data_uri = format!("data:{};base64,{}", m.mime, b64);
        let label = if m.text.is_empty() {
            "Image captured."
        } else {
            &m.text
        };
        let tag = if parsed.markers.len() > 1 {
            format!(
                "[Tool visual output {}/{}] {}",
                i + 1,
                parsed.markers.len(),
                label
            )
        } else {
            format!("[Tool visual output] {}", label)
        };
        image_items.push(json!({
            "role": "user",
            "content": [
                {
                    "type": "input_image",
                    "image_url": data_uri
                },
                {
                    "type": "input_text",
                    "text": tag
                }
            ]
        }));
    }

    (combined_text, image_items)
}

pub(super) fn expand_anthropic_image_markers_for_api(history: &[Value]) -> Vec<Value> {
    history
        .iter()
        .map(|item| {
            let mut msg = item.clone();
            if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                if let Some(Value::Array(blocks)) = msg.get_mut("content") {
                    for block in blocks.iter_mut() {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            if let Some(result) = block.get("content").and_then(|c| c.as_str()) {
                                block["content"] = build_anthropic_tool_result_content(result);
                            }
                        }
                    }
                }
            }
            msg
        })
        .collect()
}

pub(super) fn expand_openai_chat_image_markers_for_api(history: &[Value]) -> Vec<Value> {
    history
        .iter()
        .map(|item| {
            let mut msg = item.clone();
            if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
                if let Some(result) = msg.get("content").and_then(|c| c.as_str()) {
                    msg["content"] = build_openai_chat_tool_result_content(result);
                }
            }
            msg
        })
        .collect()
}

pub(super) fn expand_responses_image_markers_for_api(history: &[Value]) -> Vec<Value> {
    let mut expanded = Vec::with_capacity(history.len());
    for item in history {
        if item.get("type").and_then(|t| t.as_str()) == Some("function_call_output") {
            if let Some(result) = item.get("output").and_then(|o| o.as_str()) {
                let (text_output, image_items) = build_responses_tool_result(result);
                if !image_items.is_empty() {
                    let mut output_item = item.clone();
                    output_item["output"] = json!(text_output);
                    expanded.push(output_item);
                    expanded.extend(image_items);
                    continue;
                }
            }
        }
        expanded.push(item.clone());
    }
    expanded
}

pub(super) fn emit_thinking_delta(on_delta: &(impl Fn(&str) + Send), text: &str) {
    emit_event(
        on_delta,
        &json!({
            "type": "thinking_delta",
            "content": text,
        }),
    );
}

/// Build the "tool loop rounds exhausted" notice shown to the user when a chat
/// request hits `max_tool_rounds` without reaching natural termination. The
/// returned string is appended to the assistant message (for persistence) and
/// emitted as a text_delta so the UI sees it immediately.
pub(super) fn build_max_rounds_notice(max_rounds: u32) -> String {
    format!(
        "\n\n---\n⚠️ 已达到工具调用轮次上限（{} 轮），本轮已停止继续调用工具。\n如果任务还没完成，请发送“继续”，我会接着当前进度执行；也可以在设置 → Agent → 能力中调大 `max_tool_rounds`。",
        max_rounds
    )
}

/// Emit the max-rounds notice as a text_delta AND return it so the caller can
/// append it to `collected_text` for persistence.
pub(super) fn emit_max_rounds_notice(on_delta: &(impl Fn(&str) + Send), max_rounds: u32) -> String {
    let notice = build_max_rounds_notice(max_rounds);
    emit_text_delta(on_delta, &notice);
    notice
}

pub(super) fn emit_usage(
    on_delta: &(impl Fn(&str) + Send),
    usage: &ChatUsage,
    model: &str,
    ttft_ms: Option<u64>,
) {
    let mut event = json!({
        "type": "usage",
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
        "last_input_tokens": usage.last_input_tokens,
        "model": model,
    });
    if let Some(ttft) = ttft_ms {
        event["ttft_ms"] = json!(ttft);
    }
    emit_event(on_delta, &event);

    // Structured logging for LLM usage
    if let Some(logger) = crate::get_logger() {
        logger.log(
            "info",
            "agent",
            "agent::usage",
            &format!(
                "LLM usage: model={}, in={}, out={}",
                model, usage.input_tokens, usage.output_tokens
            ),
            Some(
                serde_json::json!({
                    "model": model,
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cache_creation": usage.cache_creation_input_tokens,
                    "cache_read": usage.cache_read_input_tokens,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{build_responses_tool_result, expand_responses_image_markers_for_api};
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    use serde_json::json;

    #[test]
    fn responses_tool_result_strips_marker_trailer_from_base64() {
        let result = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );

        let (text_output, image_items) = build_responses_tool_result(&result);

        assert_eq!(text_output, "Screenshot captured.");
        assert_eq!(image_items.len(), 1);
        assert_eq!(
            image_items[0]["content"][0]["image_url"],
            "data:image/png;base64,aGVsbG8="
        );
    }

    #[test]
    fn responses_tool_result_handles_read_tool_line_numbers() {
        let result = format!(
            "     3\t{}image/jpeg__/9j/AA==__\n     4\tscreenshot (monitor 0)\n",
            IMAGE_BASE64_PREFIX
        );

        let (_, image_items) = build_responses_tool_result(&result);

        assert_eq!(image_items.len(), 1);
        assert_eq!(
            image_items[0]["content"][0]["image_url"],
            "data:image/jpeg;base64,/9j/AA=="
        );
    }

    #[test]
    fn responses_tool_result_leaves_malformed_markers_as_plain_text() {
        let result = format!(
            "{}image/png__aGVsbG8=\nmissing closing delimiter",
            IMAGE_BASE64_PREFIX
        );

        let (text_output, image_items) = build_responses_tool_result(&result);

        assert_eq!(text_output, result);
        assert!(image_items.is_empty());
    }

    #[test]
    fn responses_tool_result_rejects_truncated_marker_preview() {
        let result = format!(
            "{}image/png__aGVs\n\n[...527806 bytes omitted...]\n\nbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );

        let (text_output, image_items) = build_responses_tool_result(&result);

        assert_eq!(text_output, result);
        assert!(image_items.is_empty());
    }

    #[test]
    fn responses_tool_result_rejects_non_image_mime() {
        let result = format!(
            "{}text/plain__aGVsbG8=__\nNot an image.",
            IMAGE_BASE64_PREFIX
        );

        let (text_output, image_items) = build_responses_tool_result(&result);

        assert_eq!(text_output, result);
        assert!(image_items.is_empty());
    }

    #[test]
    fn responses_request_expansion_is_transient() {
        let result = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );
        let history = vec![json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": result,
        })];

        let expanded = expand_responses_image_markers_for_api(&history);

        assert_eq!(history.len(), 1);
        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0]["output"], "Screenshot captured.");
        assert_eq!(expanded[1]["content"][0]["type"], "input_image");
    }
}
