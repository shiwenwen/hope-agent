use serde_json::json;

use super::types::ChatUsage;

pub(super) fn emit_event(on_delta: &(impl Fn(&str) + Send), event: &serde_json::Value) {
    if let Ok(json_str) = serde_json::to_string(event) {
        on_delta(&json_str);
    }
}

pub(super) fn emit_text_delta(on_delta: &(impl Fn(&str) + Send), text: &str) {
    emit_event(on_delta, &json!({
        "type": "text_delta",
        "content": text,
    }));
}

pub(super) fn emit_tool_call(on_delta: &(impl Fn(&str) + Send), call_id: &str, name: &str, arguments: &str) {
    emit_event(on_delta, &json!({
        "type": "tool_call",
        "call_id": call_id,
        "name": name,
        "arguments": arguments,
    }));
}

/// Media URL prefix used by tools (e.g. image_generate) to embed file paths in results.
const MEDIA_URLS_PREFIX: &str = "__MEDIA_URLS__";

/// Extract media URLs from a tool result string.
/// Returns (clean_result, media_urls).
/// If the result starts with `__MEDIA_URLS__[...]`, the JSON array is parsed and removed.
pub(super) fn extract_media_urls(result: &str) -> (String, Vec<String>) {
    if let Some(rest) = result.strip_prefix(MEDIA_URLS_PREFIX) {
        if let Some((json_line, text)) = rest.split_once('\n') {
            if let Ok(urls) = serde_json::from_str::<Vec<String>>(json_line) {
                return (text.to_string(), urls);
            }
        }
    }
    (result.to_string(), Vec::new())
}

pub(super) fn emit_tool_result(on_delta: &(impl Fn(&str) + Send), call_id: &str, name: &str, result: &str, duration_ms: u64, is_error: bool, media_urls: &[String]) {
    let mut event = json!({
        "type": "tool_result",
        "call_id": call_id,
        "name": name,
        "result": result,
        "duration_ms": duration_ms,
        "is_error": is_error,
    });
    if !media_urls.is_empty() {
        event["media_urls"] = json!(media_urls);
    }
    emit_event(on_delta, &event);
}

/// Parse the `__IMAGE_BASE64__<mime>__<base64data>\n<text>` marker.
/// Returns `Some((mime, base64, text_description))` if present.
fn parse_image_base64_marker(result: &str) -> Option<(&str, &str, &str)> {
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    let rest = result.strip_prefix(IMAGE_BASE64_PREFIX)?;
    let sep_idx = rest.find("__")?;
    let mime = &rest[..sep_idx];
    let after = &rest[sep_idx + 2..];
    let (b64, text) = after.split_once('\n').unwrap_or((after, ""));
    Some((mime, b64, text))
}

/// Build tool result content for Anthropic Messages API.
/// Detects `__IMAGE_BASE64__` marker and returns a content array with image + text blocks.
pub(super) fn build_anthropic_tool_result_content(result: &str) -> serde_json::Value {
    if let Some((mime, b64, text)) = parse_image_base64_marker(result) {
        return json!([
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": mime,
                    "data": b64
                }
            },
            {
                "type": "text",
                "text": if text.trim().is_empty() { "Screenshot captured." } else { text.trim() }
            }
        ]);
    }
    json!(result)
}

/// Build tool result content for OpenAI Chat Completions API.
/// Returns a content array with `image_url` (data URI) + `text` blocks when image is detected.
pub(super) fn build_openai_chat_tool_result_content(result: &str) -> serde_json::Value {
    if let Some((mime, b64, text)) = parse_image_base64_marker(result) {
        let data_uri = format!("data:{};base64,{}", mime, b64);
        return json!([
            {
                "type": "image_url",
                "image_url": { "url": data_uri }
            },
            {
                "type": "text",
                "text": if text.trim().is_empty() { "Screenshot captured." } else { text.trim() }
            }
        ]);
    }
    json!(result)
}

/// Build tool result for OpenAI Responses API (`function_call_output`).
/// The `output` field only accepts a string, so when an image is detected,
/// returns `(clean_text, Some(image_input_item))` where the image item
/// should be appended to the input array as a separate message.
pub(super) fn build_responses_tool_result(result: &str) -> (String, Option<serde_json::Value>) {
    if let Some((mime, b64, text)) = parse_image_base64_marker(result) {
        let clean = if text.trim().is_empty() { "Screenshot captured.".to_string() } else { text.trim().to_string() };
        let data_uri = format!("data:{};base64,{}", mime, b64);
        let image_item = json!({
            "role": "user",
            "content": [
                {
                    "type": "input_image",
                    "image_url": data_uri
                },
                {
                    "type": "input_text",
                    "text": format!("[Tool visual output] {}", clean)
                }
            ]
        });
        (clean, Some(image_item))
    } else {
        (result.to_string(), None)
    }
}

pub(super) fn emit_thinking_delta(on_delta: &(impl Fn(&str) + Send), text: &str) {
    emit_event(on_delta, &json!({
        "type": "thinking_delta",
        "content": text,
    }));
}

pub(super) fn emit_usage(on_delta: &(impl Fn(&str) + Send), usage: &ChatUsage, model: &str) {
    emit_event(on_delta, &json!({
        "type": "usage",
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
        "model": model,
    }));

    // Structured logging for LLM usage
    if let Some(logger) = crate::get_logger() {
        logger.log("info", "agent", "agent::usage",
            &format!("LLM usage: model={}, in={}, out={}", model, usage.input_tokens, usage.output_tokens),
            Some(serde_json::json!({
                "model": model,
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_creation": usage.cache_creation_input_tokens,
                "cache_read": usage.cache_read_input_tokens,
            }).to_string()),
            None, None);
    }
}
