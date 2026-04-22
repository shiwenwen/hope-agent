use serde_json::json;

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
    emit_event(on_delta, &event);
}

/// Parsed image marker: (mime, base64_data, text_description).
struct ImageMarker<'a> {
    mime: &'a str,
    b64: &'a str,
    text: &'a str,
}

/// Parse all `__IMAGE_BASE64__<mime>__<base64data>__\n<text>` markers from a tool result.
/// Returns (leading_text, Vec<ImageMarker>). Supports single and multi-image results.
fn parse_all_image_markers(result: &str) -> (String, Vec<ImageMarker<'_>>) {
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    let mut markers = Vec::new();

    // Split by the prefix; first segment is leading text (may be empty)
    let parts: Vec<&str> = result.split(IMAGE_BASE64_PREFIX).collect();
    if parts.len() <= 1 {
        // No markers found
        return (result.to_string(), markers);
    }

    let leading_text = parts[0].trim().to_string();

    for part in &parts[1..] {
        // Each part looks like: "<mime>__<base64>__\n<text_description>\n\n..."
        let Some((mime, rest)) = part.split_once("__") else {
            continue;
        };
        let Some((b64, text)) = rest.split_once("__") else {
            continue;
        };
        let text = text.strip_prefix('\n').unwrap_or(text).trim();
        markers.push(ImageMarker {
            mime: mime.trim(),
            b64: b64.trim(),
            text,
        });
    }

    (leading_text, markers)
}

/// Build tool result content for Anthropic Messages API.
/// Detects `__IMAGE_BASE64__` markers and returns a content array with image + text blocks.
pub(super) fn build_anthropic_tool_result_content(result: &str) -> serde_json::Value {
    let (leading, markers) = parse_all_image_markers(result);
    if markers.is_empty() {
        return json!(result);
    }

    let mut content = Vec::new();
    if !leading.is_empty() {
        content.push(json!({"type": "text", "text": leading}));
    }
    for m in &markers {
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": m.mime,
                "data": m.b64
            }
        }));
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            m.text
        };
        content.push(json!({"type": "text", "text": text}));
    }
    json!(content)
}

/// Build tool result content for OpenAI Chat Completions API.
/// Returns a content array with `image_url` (data URI) + `text` blocks when images are detected.
pub(super) fn build_openai_chat_tool_result_content(result: &str) -> serde_json::Value {
    let (leading, markers) = parse_all_image_markers(result);
    if markers.is_empty() {
        return json!(result);
    }

    let mut content = Vec::new();
    if !leading.is_empty() {
        content.push(json!({"type": "text", "text": leading}));
    }
    for m in &markers {
        let data_uri = format!("data:{};base64,{}", m.mime, m.b64);
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": data_uri }
        }));
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            m.text
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
    let (leading, markers) = parse_all_image_markers(result);
    if markers.is_empty() {
        return (result.to_string(), Vec::new());
    }

    // Build combined text output for the function_call_output field
    let mut text_parts = Vec::new();
    if !leading.is_empty() {
        text_parts.push(leading);
    }
    for m in &markers {
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            m.text
        };
        text_parts.push(text.to_string());
    }
    let combined_text = text_parts.join("\n");

    // Build one user message per image for the input array
    let mut image_items = Vec::new();
    for (i, m) in markers.iter().enumerate() {
        let data_uri = format!("data:{};base64,{}", m.mime, m.b64);
        let label = if m.text.is_empty() {
            "Image captured."
        } else {
            m.text
        };
        let tag = if markers.len() > 1 {
            format!("[Tool visual output {}/{}] {}", i + 1, markers.len(), label)
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
        "\n\n---\n⚠️ 已达到工具调用轮次上限（{} 轮），本次回复已被强制中止。\n可在设置 → Agent → 能力中调大 `max_tool_rounds`，或重新发起请求让我换个思路再试。",
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
    use super::build_responses_tool_result;
    use crate::tools::browser::IMAGE_BASE64_PREFIX;

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
}
