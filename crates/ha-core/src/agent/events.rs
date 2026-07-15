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

/// Fired after a `PreToolUse` hook rewrote the tool input via `updatedInput`.
/// The frontend looks up the existing `tool_call` block by `call_id` and
/// replaces its `arguments` so the UI shows what actually ran, not the
/// pre-rewrite arguments the `tool_call` event delivered moments earlier.
/// Skipped entirely when no rewrite happened (the common case).
pub(super) fn emit_tool_call_args_rewritten(
    on_delta: &(impl Fn(&str) + Send),
    call_id: &str,
    arguments: &str,
) {
    emit_event(
        on_delta,
        &json!({
            "type": "tool_call_args_rewritten",
            "call_id": call_id,
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
pub(crate) fn extract_media_items(result: &str) -> (String, Vec<MediaItem>) {
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
/// Detects internal image markers and returns a content array with image + text blocks.
pub(super) fn build_anthropic_tool_result_content(result: &str) -> serde_json::Value {
    let Some(parsed) = crate::tools::image_markers::parse_image_markers(result) else {
        return json!(result);
    };

    let mut content = Vec::new();
    if !parsed.leading_text.is_empty() {
        content.push(json!({"type": "text", "text": parsed.leading_text}));
    }
    for m in &parsed.markers {
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            &m.text
        };
        match crate::tools::image_markers::encode_marker_image(m) {
            Ok(b64) => {
                content.push(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": m.mime,
                        "data": b64
                    }
                }));
                content.push(json!({"type": "text", "text": text}));
            }
            // Image bytes are unavailable (e.g. a materialized `__IMAGE_FILE__`
            // was removed/moved on disk). Degrade to the marker's text note
            // rather than returning the raw `result` string, which would leak
            // the internal marker into the prompt and silently drop vision.
            Err(_) => content.push(json!({"type": "text", "text": text})),
        }
    }
    json!(content)
}

/// Build tool result content for OpenAI Chat Completions API.
///
/// When `model_supports_vision` is true, returns a content array with
/// `image_url` (data URI) + `text` blocks. When false, collapses the markers
/// into a plain string with `Image captured.` placeholders — the OpenAI Chat
/// `role: tool` content type rejects `image_url` blocks on non-vision
/// backends (e.g. DeepSeek text-only) with a 400, so we must strip them.
pub(super) fn build_openai_chat_tool_result_content(
    result: &str,
    model_supports_vision: bool,
) -> serde_json::Value {
    let Some(parsed) = crate::tools::image_markers::parse_image_markers(result) else {
        return json!(result);
    };

    if !model_supports_vision {
        let mut text_parts = Vec::new();
        if !parsed.leading_text.is_empty() {
            text_parts.push(parsed.leading_text.to_string());
        }
        for m in &parsed.markers {
            let label = if m.text.is_empty() {
                "Image captured."
            } else {
                &m.text
            };
            text_parts.push(label.to_string());
        }
        return json!(text_parts.join("\n"));
    }

    let mut content = Vec::new();
    if !parsed.leading_text.is_empty() {
        content.push(json!({"type": "text", "text": parsed.leading_text}));
    }
    for m in &parsed.markers {
        let text = if m.text.is_empty() {
            "Image captured."
        } else {
            &m.text
        };
        match crate::tools::image_markers::encode_marker_image(m) {
            Ok(b64) => {
                let data_uri = format!("data:{};base64,{}", m.mime, b64);
                content.push(json!({
                    "type": "image_url",
                    "image_url": { "url": data_uri }
                }));
                content.push(json!({"type": "text", "text": text}));
            }
            // Image bytes unavailable — degrade to text rather than leaking the
            // raw marker string back to the model.
            Err(_) => content.push(json!({"type": "text", "text": text})),
        }
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
            // Image bytes unavailable — skip this image item. `combined_text`
            // already carries the marker's text note (no raw marker), so the
            // model still gets the description without a leak or a dropped turn.
            continue;
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

pub(super) fn expand_openai_chat_image_markers_for_api(
    history: &[Value],
    model_supports_vision: bool,
) -> Vec<Value> {
    history
        .iter()
        .map(|item| {
            let mut msg = item.clone();
            match msg.get("role").and_then(|r| r.as_str()) {
                // Tool results carry images as `[[ha-image:...]]` markers in a
                // plain string; expanded to `image_url` blocks (or folded to
                // text when vision is unsupported).
                Some("tool") => {
                    if let Some(result) = msg.get("content").and_then(|c| c.as_str()) {
                        msg["content"] =
                            build_openai_chat_tool_result_content(result, model_supports_vision);
                    }
                }
                // User-uploaded images live as `image_url` parts inside a
                // content array. Text-only backends (e.g. DeepSeek) reject the
                // `image_url` variant with a 400, so fold them to a text
                // placeholder when the model can't see images. Vision models
                // keep the array untouched.
                Some("user") if !model_supports_vision => {
                    if let Some(Value::Array(parts)) = msg.get("content") {
                        if parts.iter().any(is_openai_image_part) {
                            msg["content"] = fold_openai_user_content_without_images(parts);
                        }
                    }
                }
                _ => {}
            }
            msg
        })
        .collect()
}

/// True if an OpenAI Chat content part is an `image_url` block.
fn is_openai_image_part(part: &Value) -> bool {
    part.get("type").and_then(Value::as_str) == Some("image_url")
}

/// Collapse a user message's multimodal content array into a plain string,
/// dropping `image_url` parts and prepending a short placeholder so the model
/// still knows an image was attached. Used only for text-only backends.
fn fold_openai_user_content_without_images(parts: &[Value]) -> Value {
    let mut out: Vec<String> = Vec::new();
    let mut dropped = 0usize;
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("image_url") => dropped += 1,
            Some("text") => {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    out.push(t.to_string());
                }
            }
            _ => {}
        }
    }
    if dropped > 0 {
        let placeholder = if dropped == 1 {
            "[image omitted: this model does not accept image input]".to_string()
        } else {
            format!("[{dropped} images omitted: this model does not accept image input]")
        };
        out.insert(0, placeholder);
    }
    json!(out.join("\n"))
}

/// True if the OpenAI Chat history carries any image content the model would
/// need vision for — user-uploaded `image_url` parts or tool image markers.
/// Drives the one-shot "model can't see images" notice.
pub(super) fn openai_chat_history_has_images(history: &[Value]) -> bool {
    history
        .iter()
        .any(|msg| match msg.get("role").and_then(|r| r.as_str()) {
            Some("user") => msg
                .get("content")
                .and_then(Value::as_array)
                .map(|parts| parts.iter().any(is_openai_image_part))
                .unwrap_or(false),
            Some("tool") => msg
                .get("content")
                .and_then(Value::as_str)
                .and_then(crate::tools::image_markers::parse_image_markers)
                .map(|p| !p.markers.is_empty())
                .unwrap_or(false),
            _ => false,
        })
}

pub(super) fn expand_responses_image_markers_for_api(history: &[Value]) -> Vec<Value> {
    let mut expanded = Vec::with_capacity(history.len());
    for item in history {
        if item.get("type").and_then(|t| t.as_str()) == Some("function_call_output") {
            if let Some(result) = item.get("output").and_then(|o| o.as_str()) {
                let (text_output, image_items) = build_responses_tool_result(result);
                // Always substitute `text_output`. When markers were present but
                // all failed to encode (file moved/deleted), `image_items` is
                // empty yet `text_output` is the marker-free note — gating on a
                // non-empty `image_items` here would push the original item and
                // leak the raw `__IMAGE_*__` marker to the model. With no
                // markers, `text_output == result`, so the rewrite is a no-op.
                let mut output_item = item.clone();
                output_item["output"] = json!(text_output);
                expanded.push(output_item);
                expanded.extend(image_items);
                continue;
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

pub(super) fn emit_round_limit_event(on_delta: &(impl Fn(&str) + Send), max_rounds: u32) {
    emit_event(
        on_delta,
        &json!({
            "type": "round_limit_reached",
            "max_rounds": max_rounds,
        }),
    );
}

pub(super) fn emit_usage(
    on_delta: &(impl Fn(&str) + Send),
    usage: &ChatUsage,
    model: &str,
    ttft_ms: Option<u64>,
    log_summary: bool,
) {
    let mut event = json!({
        "type": "usage",
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
        "context_input_tokens": usage.context_input_tokens,
        "fresh_input_tokens": usage.fresh_input_tokens,
        "last_input_tokens": usage.last_input_tokens,
        "last_context_input_tokens": usage.last_context_input_tokens,
        "last_fresh_input_tokens": usage.last_fresh_input_tokens,
        "last_cache_creation_input_tokens": usage.last_cache_creation_input_tokens,
        "last_cache_read_input_tokens": usage.last_cache_read_input_tokens,
        "model": model,
    });
    if let Some(ttft) = ttft_ms {
        event["ttft_ms"] = json!(ttft);
    }
    emit_event(on_delta, &event);

    // Structured logging for LLM usage — only on the final per-turn emit so a
    // long tool loop does not write one `agent::usage` row per round (the
    // interim per-round emits exist only to refresh the live usage gauge).
    if !log_summary {
        return;
    }
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
                    "context_input_tokens": usage.context_input_tokens,
                    "fresh_input_tokens": usage.fresh_input_tokens,
                    "output_tokens": usage.output_tokens,
                    "cache_creation": usage.cache_creation_input_tokens,
                    "cache_read": usage.cache_read_input_tokens,
                    "last_cache_creation": usage.last_cache_creation_input_tokens,
                    "last_cache_read": usage.last_cache_read_input_tokens,
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
    use super::{
        build_openai_chat_tool_result_content, build_responses_tool_result,
        expand_openai_chat_image_markers_for_api, expand_responses_image_markers_for_api,
        openai_chat_history_has_images,
    };
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
    fn responses_expand_rewrites_valid_marker_to_clean_text_plus_image() {
        // Happy path through the now-ungated caller: a valid marker yields a
        // rewritten function_call_output whose `output` is the marker-free note
        // plus a separate input_image item.
        let marker = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );
        let history = vec![json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": marker,
        })];

        let expanded = expand_responses_image_markers_for_api(&history);

        assert!(
            expanded.len() >= 2,
            "expected rewritten output + image item"
        );
        let output = expanded[0]["output"]
            .as_str()
            .expect("function_call_output output is a string");
        assert!(!output.contains(IMAGE_BASE64_PREFIX));
        assert!(output.contains("Screenshot captured."));
        assert!(expanded
            .iter()
            .any(|it| it["content"][0]["type"] == "input_image"));
    }

    #[test]
    fn responses_expand_leaves_marker_free_output_unchanged() {
        // Removing the `!image_items.is_empty()` gate must not alter non-marker
        // function_call_output items (text_output == original result → no-op).
        let history = vec![json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": "plain tool result, no images",
        })];

        let expanded = expand_responses_image_markers_for_api(&history);

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0]["output"], "plain tool result, no images");
    }

    #[test]
    fn openai_chat_tool_result_vision_emits_image_url_blocks() {
        let result = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );
        let content = build_openai_chat_tool_result_content(&result, true);
        let arr = content.as_array().expect("vision path returns array");
        assert!(arr
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("image_url")));
        assert!(arr
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")));
    }

    #[test]
    fn openai_chat_tool_result_no_vision_collapses_to_text_string() {
        let result = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );
        let content = build_openai_chat_tool_result_content(&result, false);
        // Non-vision must produce a plain string — `role: tool` content
        // on DeepSeek / text-only OpenAI-compat backends rejects any object
        // block (including `image_url`) with a 400.
        let text = content.as_str().expect("no-vision returns string");
        assert!(!text.contains("image_url"));
        assert!(!text.contains("data:image/png"));
        assert!(text.contains("Screenshot captured."));
    }

    #[test]
    fn openai_chat_tool_result_passthrough_without_markers() {
        let result = "plain text result with no image markers";
        let with_vision = build_openai_chat_tool_result_content(result, true);
        let without_vision = build_openai_chat_tool_result_content(result, false);
        assert_eq!(with_vision, json!(result));
        assert_eq!(without_vision, json!(result));
    }

    #[test]
    fn openai_chat_history_expansion_respects_vision_flag() {
        let result = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );
        let history = vec![
            json!({ "role": "user", "content": "ignore me" }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": result }),
        ];
        let with_vision = expand_openai_chat_image_markers_for_api(&history, true);
        let without_vision = expand_openai_chat_image_markers_for_api(&history, false);

        assert!(with_vision[1]["content"].is_array());
        // Non-vision path must keep `content` as a string so the server's
        // tool-message deserializer accepts it.
        assert!(without_vision[1]["content"].is_string());
        assert!(without_vision[1]["content"]
            .as_str()
            .unwrap()
            .contains("Screenshot captured."));
    }

    #[test]
    fn openai_chat_user_image_folded_for_text_only_model() {
        // A user-uploaded image is an `image_url` part inside a content array.
        let history = vec![json!({
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,aGk=" } },
                { "type": "text", "text": "看下这个图片" },
            ],
        })];

        // Vision model: untouched (array preserved).
        let with_vision = expand_openai_chat_image_markers_for_api(&history, true);
        assert!(with_vision[0]["content"].is_array());

        // Text-only model: collapsed to a string with a placeholder + the
        // original text, so the backend's deserializer never sees `image_url`.
        let without_vision = expand_openai_chat_image_markers_for_api(&history, false);
        let folded = without_vision[0]["content"].as_str().unwrap();
        assert!(folded.contains("看下这个图片"));
        assert!(folded.contains("image omitted"));
        assert!(!folded.contains("image_url"));
    }

    #[test]
    fn openai_chat_history_has_images_detects_user_and_tool_images() {
        let user_img = json!({
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,aGk=" } },
                { "type": "text", "text": "hi" },
            ],
        });
        let tool_img = json!({
            "role": "tool",
            "tool_call_id": "c1",
            "content": format!("{}image/png__aGVsbG8=__\nShot.", IMAGE_BASE64_PREFIX),
        });
        let plain_user = json!({ "role": "user", "content": "just text" });

        assert!(openai_chat_history_has_images(&[user_img]));
        assert!(openai_chat_history_has_images(&[tool_img]));
        assert!(!openai_chat_history_has_images(&[plain_user]));
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
