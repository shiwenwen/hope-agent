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

pub(super) fn emit_tool_result(on_delta: &(impl Fn(&str) + Send), call_id: &str, name: &str, result: &str, duration_ms: u64, is_error: bool) {
    emit_event(on_delta, &json!({
        "type": "tool_result",
        "call_id": call_id,
        "name": name,
        "result": result,
        "duration_ms": duration_ms,
        "is_error": is_error,
    }));
}

/// Build tool result content, detecting image base64 markers for multimodal responses.
/// For Anthropic: returns a content array with image + text blocks.
/// For OpenAI: returns a plain string (OpenAI tool results don't support images directly).
pub(super) fn build_anthropic_tool_result_content(result: &str) -> serde_json::Value {
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    if let Some(rest) = result.strip_prefix(IMAGE_BASE64_PREFIX) {
        // Format: __IMAGE_BASE64__<mime>__<base64data>\n<text description>
        if let Some(sep_idx) = rest.find("__") {
            let mime = &rest[..sep_idx];
            let after = &rest[sep_idx + 2..];
            let (b64, text) = after.split_once('\n').unwrap_or((after, ""));
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
    }
    json!(result)
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
