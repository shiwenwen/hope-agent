//! OpenAI Responses API adapter implementing [`StreamingChatAdapter`].
//!
//! Owns body construction (using [`ResponsesRequest`] struct with
//! `instructions` + `input` fields), HTTP send, SSE event decoding (with
//! `response.output_text.delta` / `response.function_call_arguments.delta` /
//! reasoning summary events), and history persistence as Responses native
//! items (`message` text + `function_call` + `function_call_output`).
//! Reasoning items are intentionally dropped from history — Hope Agent runs
//! with `store: false`, where any `rs_*` id replayed in a follow-up request
//! 404s.
//!
//! The SSE parser ([`parse_openai_sse`]) is shared with the Codex adapter
//! since they speak the same protocol — only auth header and endpoint differ.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::{FunctionCallItem, ResponsesRequest, SseEvent};
use super::super::config::build_api_url;
use super::super::events::{
    emit_text_delta, emit_thinking_delta, expand_responses_image_markers_for_api,
    project_responses_image_markers_for_token_count,
};
use super::super::streaming_adapter::{
    observe_before_send, observe_response_started, ExecutedTool, PreparedProviderRequest,
    PreparedRequestVariant, ProviderAccountingInput, ProviderDispatchObserver,
    ProviderDispatchUnknown, ProviderEndpointKind, ProviderRequestShape, RoundOutcome,
    RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ChatUsage, ProviderFormat};
use crate::tool_defs::ToolProvider;

fn supports_native_tool_search(base_url: &str, model: &str) -> bool {
    if !base_url.contains("api.openai.com") {
        return false;
    }
    let Some(version) = model.strip_prefix("gpt-") else {
        return false;
    };
    let mut parts = version.split(['-', '.']);
    let major = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    major > 5 || (major == 5 && minor >= 4)
}

fn supports_explicit_prompt_cache(base_url: &str, model: &str) -> bool {
    if !base_url.contains("api.openai.com") {
        return false;
    }
    let Some(version) = model.strip_prefix("gpt-") else {
        return false;
    };
    let mut parts = version.split(['-', '.']);
    let major = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    major > 5 || (major == 5 && minor >= 6)
}

fn native_tool_search_tools_from(
    tool_schemas: &[Value],
    deferred_tool_schemas: &[Value],
) -> Vec<Value> {
    let loaded_names: std::collections::HashSet<&str> = tool_schemas
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
        .collect();
    let mut tools: Vec<Value> = tool_schemas
        .iter()
        // Replace Hope's client meta-tool with the provider-native tool.
        .filter(|tool| tool.get("name").and_then(|v| v.as_str()) != Some("tool_search"))
        .cloned()
        .collect();
    for schema in deferred_tool_schemas {
        let name = schema.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || loaded_names.contains(name) {
            continue;
        }
        let mut deferred = schema.clone();
        deferred["defer_loading"] = json!(true);
        tools.push(deferred);
    }
    tools.push(json!({ "type": "tool_search" }));
    tools
}

fn native_tool_search_tools(req: &RoundRequest<'_>) -> Vec<Value> {
    native_tool_search_tools_from(req.tool_schemas, req.deferred_tool_schemas)
}

fn sse_request_id(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("x-request-id")
        .or_else(|| resp.headers().get("request-id"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

fn sse_event_error_message(event: &SseEvent) -> Option<&str> {
    event
        .message
        .as_deref()
        .or_else(|| event.error.as_ref().and_then(|e| e.message.as_deref()))
        .or_else(|| {
            event
                .response
                .as_ref()
                .and_then(|r| r.error.as_ref())
                .and_then(|e| e.message.as_deref())
        })
        .or(event.code.as_deref())
        .or_else(|| event.error.as_ref().and_then(|e| e.code.as_deref()))
        .or_else(|| {
            event
                .response
                .as_ref()
                .and_then(|r| r.error.as_ref())
                .and_then(|e| e.code.as_deref())
        })
}

fn sse_event_error_code(event: &SseEvent) -> Option<&str> {
    event
        .code
        .as_deref()
        .or_else(|| event.error.as_ref().and_then(|e| e.code.as_deref()))
        .or_else(|| {
            event
                .response
                .as_ref()
                .and_then(|r| r.error.as_ref())
                .and_then(|e| e.code.as_deref())
        })
}

fn sse_event_error_type(event: &SseEvent) -> Option<&str> {
    event
        .error
        .as_ref()
        .and_then(|e| e.error_type.as_deref())
        .or_else(|| {
            event
                .response
                .as_ref()
                .and_then(|r| r.error.as_ref())
                .and_then(|e| e.error_type.as_deref())
        })
}

fn extract_request_id_from_message(message: &str) -> Option<&str> {
    let marker = "request ID ";
    let start = message.find(marker)? + marker.len();
    let tail = &message[start..];
    let end = tail
        .find(|c: char| c.is_whitespace() || c == '.' || c == ',' || c == ')' || c == '"')
        .unwrap_or(tail.len());
    let candidate = &tail[..end];
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

pub(in crate::agent::providers) fn responses_assistant_message(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    Some(json!({
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text }],
        "status": "completed"
    }))
}

pub(in crate::agent::providers) fn push_responses_assistant_message(
    history: &mut Vec<Value>,
    round: Option<u32>,
    text: &str,
) {
    let Some(message) = responses_assistant_message(text) else {
        return;
    };
    if let Some(round) = round {
        crate::context_compact::push_and_stamp(history, message, round);
    } else {
        history.push(message);
    }
}

fn log_sse_error_event(
    request_id: &str,
    event_type: &str,
    event: &SseEvent,
    raw_data: &str,
    source: &str,
) {
    let Some(logger) = crate::get_logger() else {
        return;
    };

    let message = sse_event_error_message(event).unwrap_or("Unknown error");
    let effective_request_id = if request_id != "-" {
        request_id.to_string()
    } else {
        extract_request_id_from_message(message)
            .unwrap_or(request_id)
            .to_string()
    };
    let message_fingerprint = crate::cache_routing::audit_fingerprint(
        "openai-responses-sse-error-message",
        message.as_bytes(),
    );
    let event_fingerprint = crate::cache_routing::audit_fingerprint(
        "openai-responses-sse-error-event",
        raw_data.as_bytes(),
    );
    logger.log(
        "error",
        "agent",
        source,
        &format!(
            "Responses SSE error event: request_id={}, type={}",
            effective_request_id, event_type
        ),
        Some(
            json!({
                "request_id": effective_request_id,
                "header_request_id": request_id,
                "event_type": event_type,
                "message_bytes": message.len(),
                "message_fingerprint": message_fingerprint,
                "error_code": sse_event_error_code(event),
                "error_type": sse_event_error_type(event),
                "top_level_code": event.code.as_deref(),
                "event_bytes": raw_data.len(),
                "event_fingerprint": event_fingerprint,
            })
            .to_string(),
        ),
        None,
        None,
    );
}

fn log_sse_decode_error(request_id: &str, raw_data: &str, err: &serde_json::Error) {
    let Some(logger) = crate::get_logger() else {
        return;
    };

    let event_fingerprint = crate::cache_routing::audit_fingerprint(
        "openai-responses-sse-decode-event",
        raw_data.as_bytes(),
    );
    logger.log(
        "warn",
        "agent",
        "agent::parse_openai_sse::decode_error",
        &format!(
            "Responses SSE decode error: request_id={}, error={}",
            request_id, err
        ),
        Some(
            json!({
                "request_id": request_id,
                "error": err.to_string(),
                "event_bytes": raw_data.len(),
                "event_fingerprint": event_fingerprint,
            })
            .to_string(),
        ),
        None,
        None,
    );
}

fn take_next_sse_event_block(buffer: &mut Vec<u8>) -> Result<Option<String>> {
    let lf = buffer
        .windows(b"\n\n".len())
        .position(|window| window == b"\n\n")
        .map(|idx| (idx, 2));
    let crlf = buffer
        .windows(b"\r\n\r\n".len())
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| (idx, 4));
    let (idx, delim_len) = match (lf, crlf) {
        (Some(a), Some(b)) => {
            if a.0 <= b.0 {
                a
            } else {
                b
            }
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return Ok(None),
    };

    let mut consumed: Vec<u8> = buffer.drain(..idx + delim_len).collect();
    consumed.truncate(idx);
    let event_block = String::from_utf8(consumed)
        .map_err(|_| anyhow::anyhow!("Responses SSE event contained invalid UTF-8"))?;
    Ok(Some(event_block))
}

fn validate_responses_sse_eof_tail(cancelled: bool, buffer: &[u8]) -> Result<()> {
    if cancelled || buffer.is_empty() {
        return Ok(());
    }
    std::str::from_utf8(buffer)
        .map_err(|_| anyhow::anyhow!("Responses SSE ended with invalid UTF-8"))?;
    anyhow::bail!("Responses SSE ended with an incomplete event")
}

fn validate_completed_response_stream(
    cancelled: bool,
    saw_response_completed: bool,
    pending_calls: &std::collections::HashMap<String, FunctionCallItem>,
    tool_calls: &[FunctionCallItem],
) -> Result<()> {
    if cancelled {
        return Ok(());
    }
    if !saw_response_completed {
        anyhow::bail!("Responses SSE ended before response.completed")
    }
    if !pending_calls.is_empty() {
        anyhow::bail!(
            "Responses SSE completed with {} unfinished function call(s)",
            pending_calls.len()
        )
    }
    for call in tool_calls {
        if call.call_id.is_empty() || call.name.is_empty() {
            anyhow::bail!("Responses SSE completed with an invalid function call")
        }
        serde_json::from_str::<Value>(&call.arguments).map_err(|err| {
            anyhow::anyhow!(
                "Responses SSE completed with invalid function arguments for {}: {}",
                call.name,
                err
            )
        })?;
    }
    Ok(())
}

fn push_provider_history_item(items: &mut Vec<Value>, item: Value) {
    let item_type = item.get("type").and_then(Value::as_str);
    if !matches!(
        item_type,
        Some("message" | "tool_search_call" | "tool_search_output")
    ) {
        return;
    }
    if !items.iter().any(|existing| existing == &item) {
        items.push(item);
    }
}

fn output_item_arguments(item: &super::super::api_types::SseOutputItem) -> String {
    item.arguments
        .as_ref()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default()
}

fn merge_completed_function_call(
    item: &super::super::api_types::SseOutputItem,
    pending_calls: &mut std::collections::HashMap<String, FunctionCallItem>,
    tool_calls: &mut Vec<FunctionCallItem>,
) {
    let call_id = item
        .id
        .clone()
        .or_else(|| item.call_id.clone())
        .unwrap_or_default();
    let mut completed = pending_calls
        .remove(&call_id)
        .unwrap_or_else(|| FunctionCallItem {
            call_id: call_id.clone(),
            name: String::new(),
            arguments: String::new(),
        });
    completed.call_id = call_id;
    if let Some(name) = item.name.as_ref() {
        completed.name = name.clone();
    }
    let arguments = output_item_arguments(item);
    if !arguments.is_empty() {
        completed.arguments = arguments;
    }

    if let Some(existing) = tool_calls
        .iter_mut()
        .find(|existing| existing.call_id == completed.call_id)
    {
        *existing = completed;
    } else {
        tool_calls.push(completed);
    }
}

fn handle_openai_sse_event_block(
    request_id: &str,
    event_block: &str,
    request_start: std::time::Instant,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    collected_text: &mut String,
    collected_thinking: &mut String,
    tool_calls: &mut Vec<FunctionCallItem>,
    provider_history_items: &mut Vec<Value>,
    pending_calls: &mut std::collections::HashMap<String, FunctionCallItem>,
    usage: &mut ChatUsage,
    first_token_time: &mut Option<u64>,
    saw_response_completed: &mut bool,
) -> Result<()> {
    let data_lines: Vec<&str> = event_block
        .lines()
        .filter(|l| l.starts_with("data:"))
        .map(|l| l[5..].trim())
        .collect();

    if data_lines.is_empty() {
        return Ok(());
    }

    let data = data_lines.join("\n").trim().to_string();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }

    let raw_event = serde_json::from_str::<Value>(&data).ok();
    match serde_json::from_str::<SseEvent>(&data) {
        Ok(event) => {
            let event_type = event.event_type.as_deref().unwrap_or("");

            match event_type {
                "response.reasoning_summary_text.delta" => {
                    if let Some(delta) = &event.delta {
                        if first_token_time.is_none() {
                            *first_token_time = Some(request_start.elapsed().as_millis() as u64);
                        }
                        emit_thinking_delta(&on_delta, delta);
                        collected_thinking.push_str(delta);
                    }
                }
                "response.reasoning_summary_part.done" => {
                    collected_thinking.push_str("\n\n");
                    emit_thinking_delta(&on_delta, "\n\n");
                }
                "response.output_text.delta" => {
                    if let Some(delta) = &event.delta {
                        if first_token_time.is_none() {
                            *first_token_time = Some(request_start.elapsed().as_millis() as u64);
                        }
                        emit_text_delta(&on_delta, delta);
                        collected_text.push_str(delta);
                    }
                }
                "response.output_item.added" => {
                    if let Some(item) = &event.item {
                        if item.item_type.as_deref() == Some("function_call") {
                            let call_id = item
                                .id
                                .clone()
                                .or_else(|| item.call_id.clone())
                                .unwrap_or_default();
                            let name = item.name.clone().unwrap_or_default();
                            pending_calls.insert(
                                call_id.clone(),
                                FunctionCallItem {
                                    call_id,
                                    name,
                                    arguments: output_item_arguments(item),
                                },
                            );
                        }
                    }
                }
                "response.function_call_arguments.delta" => {
                    if let Some(delta) = &event.delta {
                        if let Some(item) = &event.item {
                            let call_id = item
                                .id
                                .clone()
                                .or_else(|| item.call_id.clone())
                                .unwrap_or_default();
                            if let Some(tc) = pending_calls.get_mut(&call_id) {
                                tc.arguments.push_str(delta);
                            }
                        } else if let Some(tc) = pending_calls.values_mut().last() {
                            tc.arguments.push_str(delta);
                        }
                    }
                }
                "response.function_call_arguments.done" | "response.output_item.done" => {
                    if let Some(item) = &event.item {
                        if item.item_type.as_deref() == Some("function_call") {
                            merge_completed_function_call(item, pending_calls, tool_calls);
                        }
                        if let Some(raw_item) =
                            raw_event.as_ref().and_then(|raw| raw.get("item")).cloned()
                        {
                            push_provider_history_item(provider_history_items, raw_item);
                        }
                        // Responses/Codex run with `store: false`, so any
                        // `rs_*` reasoning item the server emits is throwaway
                        // — we never replay it back. The streaming `thinking`
                        // text is captured via `collected_thinking` above and
                        // surfaces in the UI; the structured item itself is
                        // deliberately dropped here.
                    }
                }
                "error" => {
                    log_sse_error_event(
                        request_id,
                        event_type,
                        &event,
                        &data,
                        "agent::parse_openai_sse::event_error",
                    );
                    let msg = sse_event_error_message(&event).unwrap_or("Unknown error");
                    return Err(crate::failover::ProviderApiError::from_stream_event(
                        "OpenAI Responses/Codex",
                        sse_event_error_code(&event),
                        sse_event_error_type(&event),
                        Some(msg),
                        format!("Codex error: {msg}"),
                    )
                    .into());
                }
                "response.failed" => {
                    log_sse_error_event(
                        request_id,
                        event_type,
                        &event,
                        &data,
                        "agent::parse_openai_sse::response_failed",
                    );
                    let msg = sse_event_error_message(&event).unwrap_or("Codex response failed");
                    return Err(crate::failover::ProviderApiError::from_stream_event(
                        "OpenAI Responses/Codex",
                        sse_event_error_code(&event),
                        sse_event_error_type(&event),
                        Some(msg),
                        msg.to_string(),
                    )
                    .into());
                }
                "response.completed" => {
                    let Some(resp_obj) = event.response.as_ref() else {
                        anyhow::bail!("response.completed event omitted the response object")
                    };
                    if let Some(raw_outputs) = raw_event
                        .as_ref()
                        .and_then(|raw| raw.pointer("/response/output"))
                        .and_then(Value::as_array)
                    {
                        for item in raw_outputs {
                            push_provider_history_item(provider_history_items, item.clone());
                        }
                    }
                    if let Some(u) = &resp_obj.usage {
                        if let Some(it) = u.input_tokens {
                            usage.input_tokens = it;
                            usage.input_coverage = crate::token_accounting::UsageCoverage::Complete;
                        }
                        if let Some(ot) = u.output_tokens {
                            usage.output_tokens = ot;
                            usage.output_coverage =
                                crate::token_accounting::UsageCoverage::Complete;
                        }
                        if let Some(cr) = u.cache_read_input_tokens {
                            usage.cache_read_input_tokens = cr;
                        }
                        if let Some(cc) = u.cache_creation_input_tokens {
                            usage.cache_creation_input_tokens = cc;
                        }
                        if usage.cache_read_input_tokens == 0 {
                            usage.cache_read_input_tokens = u
                                .input_tokens_details
                                .as_ref()
                                .and_then(|d| d.cached_tokens)
                                .or_else(|| {
                                    u.prompt_tokens_details
                                        .as_ref()
                                        .and_then(|d| d.cached_tokens)
                                })
                                .unwrap_or(0);
                        }
                        if usage.cache_creation_input_tokens == 0 {
                            usage.cache_creation_input_tokens = u
                                .input_tokens_details
                                .as_ref()
                                .and_then(|d| d.cache_write_tokens)
                                .or_else(|| {
                                    u.prompt_tokens_details
                                        .as_ref()
                                        .and_then(|d| d.cache_write_tokens)
                                })
                                .unwrap_or(0);
                        }
                    }
                    if let Some(outputs) = &resp_obj.output {
                        for item in outputs {
                            if collected_text.is_empty()
                                && item.item_type.as_deref() == Some("message")
                            {
                                if let Some(parts) = &item.content {
                                    for part in parts {
                                        if part.part_type.as_deref() == Some("output_text") {
                                            if let Some(text) = &part.text {
                                                collected_text.push_str(text);
                                            }
                                        }
                                    }
                                }
                            }
                            if item.item_type.as_deref() == Some("function_call") {
                                merge_completed_function_call(item, pending_calls, tool_calls);
                            }
                        }
                    }
                    *saw_response_completed = true;
                }
                _ => {}
            }
        }
        Err(err) => {
            log_sse_decode_error(request_id, &data, &err);
            return Err(anyhow::anyhow!(
                "Responses SSE event could not be decoded: {}",
                err
            ));
        }
    }

    Ok(())
}

fn build_responses_request(
    base_url: &str,
    model: &str,
    reasoning: Option<super::super::api_types::ReasoningConfig>,
    req: &RoundRequest<'_>,
) -> (ResponsesRequest, Vec<Value>, bool, bool) {
    let mut api_input: Vec<Value> = Vec::new();
    for content in super::super::streaming_adapter::dynamic_instruction_suffixes(req) {
        api_input.push(json!({ "role": "developer", "content": content }));
    }
    api_input.extend(expand_responses_image_markers_for_api(req.history_for_api));
    if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
        api_input.push(json!({ "role": "user", "content": content }));
    }

    let explicit_prompt_cache = supports_explicit_prompt_cache(base_url, model);
    if explicit_prompt_cache {
        api_input.insert(
            0,
            json!({
                "type": "message",
                "role": "developer",
                "content": [{
                    "type": "input_text",
                    "text": req.system_prompt,
                    "prompt_cache_breakpoint": { "mode": "explicit" }
                }]
            }),
        );
    }

    let native_deferred = !req.is_final_round
        && !req.deferred_tool_schemas.is_empty()
        && supports_native_tool_search(base_url, model);
    let tools = if req.is_final_round {
        None
    } else if native_deferred {
        Some(native_tool_search_tools(req))
    } else {
        Some(req.tool_schemas.to_vec())
    };
    let request = ResponsesRequest {
        model: model.to_string(),
        store: false,
        stream: true,
        instructions: (!explicit_prompt_cache).then(|| req.system_prompt.to_string()),
        input: api_input.clone(),
        reasoning,
        include: None,
        tools,
        temperature: req.temperature,
        prompt_cache_key: req.prompt_cache_key.map(str::to_string),
        prompt_cache_options: explicit_prompt_cache
            .then(|| json!({ "mode": "explicit", "ttl": "30m" })),
    };
    (request, api_input, explicit_prompt_cache, native_deferred)
}

fn build_responses_count_body(
    base_url: &str,
    model: &str,
    reasoning: Option<super::super::api_types::ReasoningConfig>,
    req: &RoundRequest<'_>,
) -> Value {
    let (request, _, _, _) = build_responses_request(base_url, model, reasoning, req);
    let mut body = json!({
        "model": request.model,
        "instructions": request.instructions,
        "input": request.input,
        "reasoning": request.reasoning,
        "tools": request.tools,
    });
    if let Some(object) = body.as_object_mut() {
        object.retain(|_, value| !value.is_null());
    }
    body
}

pub(crate) struct OpenAIResponsesStreamingAdapter<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
    /// Resolved Responses `reasoning` config for this turn (built by
    /// [`AssistantAgent::resolve_reasoning_config`] which clamps to model's
    /// supported range). `None` = reasoning disabled.
    pub reasoning: Option<super::super::api_types::ReasoningConfig>,
}

#[async_trait]
impl<'a> StreamingChatAdapter for OpenAIResponsesStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::OpenAIResponses
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::OpenAI
    }

    fn supports_native_tool_search(&self) -> bool {
        supports_native_tool_search(self.base_url, self.model)
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_responses(history);
    }

    fn token_count_tool_schemas_for(
        &self,
        tool_schemas: &[Value],
        deferred_tool_schemas: &[Value],
        _eager_tool_count: usize,
        is_final_round: bool,
    ) -> Vec<Value> {
        if is_final_round {
            return Vec::new();
        }
        if !deferred_tool_schemas.is_empty()
            && supports_native_tool_search(self.base_url, self.model)
        {
            native_tool_search_tools_from(tool_schemas, deferred_tool_schemas)
        } else {
            tool_schemas.to_vec()
        }
    }

    fn token_count_history_for(&self, history: &[Value]) -> Vec<Value> {
        project_responses_image_markers_for_token_count(history)
    }

    fn prepare_history_for_api(&self, history: &[Value]) -> Vec<Value> {
        expand_responses_image_markers_for_api(history)
    }

    fn token_count_input_for(&self, req: &RoundRequest<'_>) -> ProviderAccountingInput {
        let mut dynamic_items: Vec<Value> =
            super::super::streaming_adapter::dynamic_instruction_suffixes(req)
                .into_iter()
                .map(|content| json!({ "role": "developer", "content": content }))
                .collect();
        if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
            dynamic_items.push(json!({ "role": "user", "content": content }));
        }

        let explicit_prompt_cache = supports_explicit_prompt_cache(self.base_url, self.model);
        let stable_prompt = if explicit_prompt_cache {
            serde_json::to_string(&json!({
                "input": [{
                    "type": "message",
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": req.system_prompt,
                        "prompt_cache_breakpoint": { "mode": "explicit" }
                    }]
                }]
            }))
            .unwrap_or_default()
        } else {
            serde_json::to_string(&json!({ "instructions": req.system_prompt })).unwrap_or_default()
        };
        ProviderAccountingInput {
            stable_prompt,
            dynamic_prompt: serde_json::to_string(&dynamic_items).unwrap_or_default(),
            history: self.token_count_history_for(req.history_for_api),
        }
    }

    async fn count_input_tokens(
        &self,
        client: &reqwest::Client,
        req: &RoundRequest<'_>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Option<u64>> {
        let capability_key = format!(
            "openai_responses_input_tokens:{}",
            self.base_url.trim_end_matches('/')
        );
        let accounting = crate::token_accounting::service();
        let profile_key =
            crate::token_accounting::profile_suppression_key(&capability_key, self.api_key);
        if !accounting.provider_count_profile_allowed(&profile_key) {
            return Ok(None);
        }
        let Some(_attempt) = accounting.begin_provider_count(&capability_key) else {
            return Ok(None);
        };

        let body =
            build_responses_count_body(self.base_url, self.model, self.reasoning.clone(), req);
        let api_url = build_api_url(self.base_url, "/v1/responses/input_tokens");
        let app_config = crate::config::cached_config();
        let ssrf = &app_config.ssrf;
        crate::security::ssrf::check_url(&api_url, ssrf.default_policy, &ssrf.trusted_hosts)
            .await?;
        let mut http_request = client
            .post(&api_url)
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            http_request = http_request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = match super::cancel::send_with_cancel(http_request.json(&body), cancel).await
        {
            Ok(response) => response,
            Err(error) => {
                accounting.suppress_provider_count_profile(
                    profile_key,
                    std::time::Duration::from_secs(5),
                );
                return Err(error.into());
            }
        };
        let Some(response) = response else {
            return Ok(None);
        };
        let status = response.status();
        if !status.is_success() {
            if matches!(status.as_u16(), 404 | 405 | 501) {
                accounting.record_provider_count_unsupported(capability_key);
            } else if matches!(status.as_u16(), 401 | 403) {
                accounting.suppress_provider_count_profile(
                    profile_key,
                    std::time::Duration::from_secs(60),
                );
            }
            return Ok(None);
        }
        let value =
            super::super::streaming_adapter::read_token_count_json_limited(response).await?;
        let count = value.get("input_tokens").and_then(Value::as_u64);
        if count.is_some() {
            accounting.record_provider_count_supported(capability_key);
        }
        Ok(count)
    }

    fn prepare_round_request(&self, req: &RoundRequest<'_>) -> Result<PreparedProviderRequest> {
        let (request, api_input, _explicit_prompt_cache, native_deferred) =
            build_responses_request(self.base_url, self.model, self.reasoning.clone(), req);
        let request_tool_count = request.tools.as_ref().map_or(0, Vec::len);
        let prepared = PreparedProviderRequest::from_json(
            ProviderEndpointKind::OpenAIResponses,
            ProviderRequestShape::OpenAIResponsesJson,
            self.model,
            req.round,
            req.session_id,
            req.reasoning_effort,
            req.vision_bridge_available,
            PreparedRequestVariant::OpenAIResponses,
            &request,
        )?;
        super::super::token_manifest::log_round_manifest(
            "OpenAIResponses",
            self.model,
            "responses",
            req,
            request.tools.as_deref().unwrap_or(&[]),
            prepared.identity.body_len as usize,
            native_deferred,
        );
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "debug",
                "agent",
                "agent::chat_openai_responses::request",
                &format!(
                    "OpenAI Responses API request round {}: {} input items, {} tools, body {}B",
                    req.round,
                    api_input.len(),
                    request_tool_count,
                    prepared.identity.body_len
                ),
                Some(
                    json!({
                        "round": req.round,
                        "endpoint_kind": "openai_responses",
                        "model": self.model,
                        "input_count": api_input.len(),
                        "tool_count": request_tool_count,
                        "body_size_bytes": prepared.identity.body_len,
                        "body_fingerprint": &prepared.identity.body_keyed_fingerprint,
                        "reasoning": self.reasoning.as_ref().map(|r| r.effort.as_str()),
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }
        Ok(prepared)
    }

    async fn dispatch_prepared(
        &self,
        client: &reqwest::Client,
        prepared: &PreparedProviderRequest,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
        observer: &dyn ProviderDispatchObserver,
    ) -> Result<RoundOutcome> {
        let api_url = build_api_url(self.base_url, "/v1/responses");
        // ── Send.
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }
        observe_before_send(observer, prepared).await?;
        let mut http_req = client
            .post(&api_url)
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            http_req = http_req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let request_start = std::time::Instant::now();
        let resp =
            match super::cancel::send_with_cancel(http_req.body(prepared.body().to_vec()), cancel)
                .await
            {
                Ok(Some(resp)) => resp,
                Ok(None) => {
                    return Err(ProviderDispatchUnknown(
                        "cancelled after dispatch claim and before response headers".to_string(),
                    )
                    .into())
                }
                Err(e) => return Err(ProviderDispatchUnknown(e.to_string()).into()),
            };
        observe_response_started(observer, prepared, 1, &resp).await?;

        if let Some(logger) = crate::get_logger() {
            let status = resp.status().as_u16();
            let headers = resp.headers();
            let request_id = headers
                .get("x-request-id")
                .or_else(|| headers.get("request-id"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-")
                .to_string();
            let ttfb_ms = request_start.elapsed().as_millis() as u64;
            let response_headers = json!({
                "x-request-id": request_id,
                "x-ratelimit-limit-requests": headers.get("x-ratelimit-limit-requests").and_then(|v| v.to_str().ok()),
                "x-ratelimit-limit-tokens": headers.get("x-ratelimit-limit-tokens").and_then(|v| v.to_str().ok()),
                "x-ratelimit-remaining-requests": headers.get("x-ratelimit-remaining-requests").and_then(|v| v.to_str().ok()),
                "x-ratelimit-remaining-tokens": headers.get("x-ratelimit-remaining-tokens").and_then(|v| v.to_str().ok()),
                "x-ratelimit-reset-requests": headers.get("x-ratelimit-reset-requests").and_then(|v| v.to_str().ok()),
                "x-ratelimit-reset-tokens": headers.get("x-ratelimit-reset-tokens").and_then(|v| v.to_str().ok()),
                "openai-model": headers.get("openai-model").and_then(|v| v.to_str().ok()),
                "openai-organization": headers.get("openai-organization").and_then(|v| v.to_str().ok()),
                "openai-version": headers.get("openai-version").and_then(|v| v.to_str().ok()),
                "retry-after": headers.get("retry-after").and_then(|v| v.to_str().ok()),
            });
            logger.log(
                "debug",
                "agent",
                "agent::chat_openai_responses::response",
                &format!(
                    "OpenAI Responses API response: status={}, request_id={}, ttfb={}ms",
                    status, request_id, ttfb_ms
                ),
                Some(
                    json!({
                        "status": status,
                        "request_id": request_id,
                        "ttfb_ms": ttfb_ms,
                        "round": prepared.identity.round,
                        "response_headers": response_headers,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let error_text = match super::cancel::read_text_with_cancel(resp, cancel).await {
                Ok(Some(text)) => text,
                Ok(None) => return Ok(super::cancel::cancelled_round_outcome()),
                Err(_) => String::new(),
            };
            if let Some(logger) = crate::get_logger() {
                let error_fingerprint = crate::cache_routing::audit_fingerprint(
                    "openai-responses-error",
                    error_text.as_bytes(),
                );
                logger.log(
                    "error",
                    "agent",
                    "agent::chat_openai_responses::error",
                    &format!("OpenAI Responses API error ({status})"),
                    Some(
                        json!({
                            "status": status,
                            "error_bytes": error_text.len(),
                            "error_fingerprint": error_fingerprint,
                            "round": prepared.identity.round
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }
            return Err(crate::failover::ProviderApiError::from_http_response(
                "OpenAI Responses",
                status,
                &error_text,
            )
            .into());
        }

        let (text, tool_calls, provider_history_items, mut usage, thinking_text, ttft_ms) =
            parse_openai_sse(resp, request_start, cancel.as_ref(), on_delta).await?;
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }

        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_openai_responses::tool_loop",
                    &format!(
                        "Tool loop round {}: executing {} tools: {:?}",
                        prepared.identity.round,
                        tool_calls.len(),
                        tool_names
                    ),
                    Some(
                        json!({
                            "round": prepared.identity.round,
                            "tool_count": tool_calls.len(),
                            "tools": tool_names,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }
        }

        usage.normalize_openai_round();
        super::super::token_manifest::log_round_usage(
            "OpenAIResponses",
            self.model,
            prepared.identity.round,
            prepared.session_id.as_deref(),
            &usage,
            ttft_ms,
        );
        Ok(RoundOutcome {
            text,
            thinking: thinking_text,
            tool_calls,
            provider_history_items,
            usage,
            ttft_ms,
            stop_reason: None,
        })
    }

    fn append_round_to_history(
        &self,
        history: &mut Vec<Value>,
        round: u32,
        outcome: &RoundOutcome,
        executed: &[ExecutedTool],
    ) {
        // Keep assistant narration in the next round's model-visible context,
        // matching the Responses item stream shape: message, function_call,
        // then function_call_output.
        // A completed Responses payload contains message/tool-search items in
        // provider order. Prefer those exact items; synthesize a message only
        // for interrupted streams where the completed item was unavailable.
        if !outcome
            .provider_history_items
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        {
            push_responses_assistant_message(history, Some(round), &outcome.text);
        }

        for item in &outcome.provider_history_items {
            crate::context_compact::push_and_stamp(history, item.clone(), round);
        }

        // Per executed tool: function_call item + function_call_output item.
        for et in executed {
            crate::context_compact::push_and_stamp(
                history,
                json!({
                    "type": "function_call",
                    "id": et.call_id,
                    "call_id": et.call_id,
                    "name": et.name,
                    "arguments": et.arguments,
                }),
                round,
            );
            crate::context_compact::push_and_stamp(
                history,
                json!({
                    "type": "function_call_output",
                    "call_id": et.call_id,
                    "output": et.clean_result,
                }),
                round,
            );
        }
    }

    fn append_final_assistant(
        &self,
        history: &mut Vec<Value>,
        final_text: &str,
        _last_thinking: &str,
    ) {
        // Responses API final assistant is a `message` item with `output_text`
        // content. With `store: false` we never replay reasoning items, so
        // thinking is intentionally dropped here — it streams to the UI live
        // but does not persist into history.
        push_responses_assistant_message(history, None, final_text);
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        outcome.tool_calls.is_empty()
    }
}

/// Parse OpenAI SSE stream (Responses API + Codex share this).
/// Returns `(collected_text, tool_calls, provider_items, usage, thinking, ttft_ms)`.
pub(in crate::agent) async fn parse_openai_sse(
    resp: reqwest::Response,
    request_start: std::time::Instant,
    cancel: &AtomicBool,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    Vec<FunctionCallItem>,
    Vec<Value>,
    ChatUsage,
    String,
    Option<u64>,
)> {
    let request_id = sse_request_id(&resp);
    let mut collected_text = String::new();
    let mut collected_thinking = String::new();
    let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
    let mut provider_history_items: Vec<Value> = Vec::new();
    let mut pending_calls: std::collections::HashMap<String, FunctionCallItem> =
        std::collections::HashMap::new();
    let mut usage = ChatUsage::default();
    let mut first_token_time: Option<u64> = None;

    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut saw_response_completed = false;

    'response_stream: while let Some(chunk) =
        super::cancel::next_chunk_or_cancel_flag(&mut stream, cancel).await
    {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "error",
                        "agent",
                        "agent::parse_openai_sse::stream_error",
                        &format!(
                            "Responses SSE stream read error: request_id={}, error={}",
                            request_id, err
                        ),
                        Some(
                            json!({
                                "request_id": request_id,
                                "error": err.to_string(),
                            })
                            .to_string(),
                        ),
                        None,
                        None,
                    );
                }
                return Err(err.into());
            }
        };
        buffer.extend_from_slice(&chunk);

        while let Some(event_block) = take_next_sse_event_block(&mut buffer)? {
            handle_openai_sse_event_block(
                &request_id,
                &event_block,
                request_start,
                on_delta,
                &mut collected_text,
                &mut collected_thinking,
                &mut tool_calls,
                &mut provider_history_items,
                &mut pending_calls,
                &mut usage,
                &mut first_token_time,
                &mut saw_response_completed,
            )?;
            if saw_response_completed {
                break 'response_stream;
            }
        }
    }

    let cancelled = cancel.load(std::sync::atomic::Ordering::SeqCst);
    validate_responses_sse_eof_tail(cancelled, &buffer)?;

    if cancelled {
        pending_calls.clear();
        tool_calls.clear();
    }
    validate_completed_response_stream(
        cancelled,
        saw_response_completed,
        &pending_calls,
        &tool_calls,
    )?;

    if let Some(logger) = crate::get_logger() {
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        logger.log(
            "debug",
            "agent",
            "agent::parse_openai_sse::done",
            &format!(
                "OpenAI Responses SSE done: {}chars text, {} tool_calls",
                collected_text.len(),
                tool_calls.len()
            ),
            Some(
                json!({
                    "request_id": request_id,
                    "text_length": collected_text.len(),
                    "tool_calls": tool_names,
                    "tool_call_count": tool_calls.len(),
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_creation": usage.cache_creation_input_tokens,
                        "cache_read": usage.cache_read_input_tokens,
                    },
                    "response_completed": saw_response_completed,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok((
        collected_text,
        tool_calls,
        provider_history_items,
        usage,
        collected_thinking,
        first_token_time,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        build_responses_count_body, build_responses_request, extract_request_id_from_message,
        handle_openai_sse_event_block, sse_event_error_code, sse_event_error_message,
        sse_event_error_type, supports_explicit_prompt_cache, supports_native_tool_search,
        take_next_sse_event_block, validate_completed_response_stream,
        validate_responses_sse_eof_tail, FunctionCallItem, OpenAIResponsesStreamingAdapter,
        SseEvent,
    };
    use crate::agent::streaming_adapter::{
        ExecutedTool, RoundOutcome, RoundRequest, StreamingChatAdapter,
    };
    use crate::agent::types::ChatUsage;
    use std::collections::HashMap;

    #[test]
    fn openai_capability_guards_are_endpoint_and_model_aware() {
        assert!(supports_native_tool_search(
            "https://api.openai.com",
            "gpt-5.4"
        ));
        assert!(!supports_native_tool_search(
            "https://api.openai.com",
            "gpt-5.3"
        ));
        assert!(!supports_native_tool_search(
            "https://compatible.example",
            "gpt-5.6"
        ));
        assert!(supports_explicit_prompt_cache(
            "https://api.openai.com",
            "gpt-5.6"
        ));
        assert!(!supports_explicit_prompt_cache(
            "https://api.openai.com",
            "gpt-5.5"
        ));
    }

    #[test]
    fn openai_responses_request_golden_preserves_explicit_cache_prefix() {
        let tools = vec![
            serde_json::json!({ "type": "function", "name": "tool_search" }),
            serde_json::json!({ "type": "function", "name": "read" }),
        ];
        let deferred = vec![serde_json::json!({ "type": "function", "name": "browser" })];
        let history = vec![serde_json::json!({ "role": "user", "content": "question" })];
        let mut req = RoundRequest {
            session_id: Some("session"),
            system_prompt: "stable",
            run_instruction_suffix: Some("run"),
            run_data_suffix: Some("run data"),
            awareness_suffix: Some("awareness"),
            active_memory_suffix: Some("memory"),
            legacy_memory_suffix: Some("legacy memory"),
            coding_profile_suffix: Some("coding"),
            procedure_memory_suffix: Some("procedure"),
            related_notes_suffix: Some("notes"),
            attached_knowledge_suffix: Some("attached"),
            capability_catalog_suffix: Some("capabilities"),
            user_profile_suffix: Some("profile"),
            environment_context_suffix: Some("environment"),
            lsp_diagnostics_suffix: None,
            task_reminder_suffix: Some("task"),
            tool_schemas: &tools,
            deferred_tool_schemas: &deferred,
            eager_tool_count: 2,
            deferred_tool_count: 1,
            activated_tool_count: 0,
            prompt_cache_key: Some("stable-key"),
            history_for_api: &history,
            vision_bridge_available: false,
            reasoning_effort: None,
            temperature: None,
            max_tokens: 100,
            is_final_round: false,
            round: 0,
        };
        let (request, input, explicit, native_deferred) =
            build_responses_request("https://api.openai.com", "gpt-5.6", None, &req);
        assert!(explicit);
        assert!(native_deferred);
        let body = serde_json::to_value(&request).unwrap();
        let count_body =
            build_responses_count_body("https://api.openai.com", "gpt-5.6", None, &req);
        for field in ["model", "instructions", "input", "reasoning", "tools"] {
            assert_eq!(count_body.get(field), body.get(field));
        }
        assert!(count_body.get("stream").is_none());
        assert!(count_body.get("prompt_cache_key").is_none());
        assert!(body.get("instructions").is_none());
        assert_eq!(body["prompt_cache_key"], "stable-key");
        assert_eq!(body["prompt_cache_options"]["mode"], "explicit");
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["content"][0]["text"], "stable");
        let contents = input[1..]
            .iter()
            .map(|item| item["content"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(&contents[..3], &["run", "coding", "question"]);
        assert_eq!(input[3]["role"], "user");
        assert!(contents[3].contains("source=\"active_memory\""));
        assert!(contents[3].contains("source=\"legacy_memory\""));
        assert!(contents[3].contains("source=\"task_and_hook_context\""));
        let request_tools = body["tools"].as_array().unwrap();
        assert_eq!(request_tools[0]["name"], "read");
        assert_eq!(request_tools[1]["name"], "browser");
        assert_eq!(request_tools[1]["defer_loading"], true);
        assert_eq!(request_tools[2]["type"], "tool_search");
        let adapter = OpenAIResponsesStreamingAdapter {
            api_key: "sk-responses-test-must-stay-in-header",
            base_url: "https://api.openai.com",
            model: "gpt-5.6",
            reasoning: None,
        };
        let accounting = adapter.token_count_input_for(&req);
        let counted_stable: serde_json::Value =
            serde_json::from_str(&accounting.stable_prompt).unwrap();
        let counted_dynamic: Vec<serde_json::Value> =
            serde_json::from_str(&accounting.dynamic_prompt).unwrap();
        assert_eq!(counted_stable["input"][0], input[0]);
        assert_eq!(
            counted_dynamic,
            vec![input[1].clone(), input[2].clone(), input[4].clone()]
        );
        assert_eq!(accounting.history, history);
        assert_eq!(
            adapter.token_count_tool_schemas(&req),
            request_tools.clone()
        );
        let prepared = adapter.prepare_round_request(&req).unwrap();
        assert_eq!(
            prepared.body_bytes_for_test(),
            serde_json::to_vec(&request).unwrap()
        );
        assert!(!String::from_utf8_lossy(prepared.body_bytes_for_test())
            .contains("sk-responses-test-must-stay-in-header"));
        assert!(!prepared
            .identity
            .body_keyed_fingerprint
            .contains("sk-responses"));
        let prepared_json: serde_json::Value =
            serde_json::from_slice(prepared.body_bytes_for_test()).unwrap();
        for transport_field in ["authorization", "api_key", "access_token", "account_id"] {
            assert!(prepared_json.get(transport_field).is_none());
        }
        req.is_final_round = true;
        assert!(adapter.token_count_tool_schemas(&req).is_empty());
    }

    #[test]
    fn nested_error_fields_are_extracted_from_event_error() {
        let event: SseEvent = serde_json::from_value(serde_json::json!({
            "type": "error",
            "error": {
                "message": "session invalid",
                "code": "invalid_session",
                "type": "invalid_request_error"
            }
        }))
        .expect("parse nested error event");

        assert_eq!(sse_event_error_message(&event), Some("session invalid"));
        assert_eq!(sse_event_error_code(&event), Some("invalid_session"));
        assert_eq!(sse_event_error_type(&event), Some("invalid_request_error"));
    }

    #[test]
    fn response_failed_uses_nested_response_error_fields() {
        let event: SseEvent = serde_json::from_value(serde_json::json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "message": "tool schema rejected",
                    "code": "invalid_tool_schema",
                    "type": "invalid_request_error"
                }
            }
        }))
        .expect("parse response.failed event");

        assert_eq!(
            sse_event_error_message(&event),
            Some("tool schema rejected")
        );
        assert_eq!(sse_event_error_code(&event), Some("invalid_tool_schema"));
        assert_eq!(sse_event_error_type(&event), Some("invalid_request_error"));
    }

    #[test]
    fn request_id_is_extracted_from_error_message() {
        let message = "An error occurred while processing your request. Please include the request ID 8d46da73-d9c2-44d5-af24-707fb7680aad in your message.";
        assert_eq!(
            extract_request_id_from_message(message),
            Some("8d46da73-d9c2-44d5-af24-707fb7680aad")
        );
    }

    #[test]
    fn take_next_sse_event_block_supports_lf_delimiter() {
        let mut buffer =
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\nrest".to_vec();
        let block = take_next_sse_event_block(&mut buffer)
            .expect("valid UTF-8")
            .expect("event block");
        assert_eq!(
            block,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}"
        );
        assert_eq!(buffer, b"rest");
    }

    #[test]
    fn take_next_sse_event_block_supports_crlf_delimiter() {
        let mut buffer =
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\r\n\r\nrest"
                .to_vec();
        let block = take_next_sse_event_block(&mut buffer)
            .expect("valid UTF-8")
            .expect("event block");
        assert_eq!(
            block,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}"
        );
        assert_eq!(buffer, b"rest");
    }

    #[test]
    fn responses_sse_framing_preserves_unicode_split_inside_scalar() {
        let payload = serde_json::json!({
            "type": "response.output_text.delta",
            "delta": "中文🙂"
        });
        let wire = format!("data: {payload}\n\n").into_bytes();
        let scalar_start = wire
            .windows("🙂".len())
            .position(|window| window == "🙂".as_bytes())
            .expect("emoji scalar in fixture");
        let split = scalar_start + 1;
        let mut buffer = wire[..split].to_vec();
        assert!(take_next_sse_event_block(&mut buffer).unwrap().is_none());
        buffer.extend_from_slice(&wire[split..]);
        let block = take_next_sse_event_block(&mut buffer)
            .unwrap()
            .expect("complete SSE frame");
        let data = block
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .expect("data line")
            .trim();
        let decoded: serde_json::Value =
            serde_json::from_str(data).expect("valid Unicode JSON event");
        assert_eq!(decoded["delta"], "中文🙂");
        assert!(buffer.is_empty());
    }

    #[test]
    fn responses_sse_eof_and_invalid_utf8_fail_closed() {
        assert!(validate_responses_sse_eof_tail(false, b"data: partial").is_err());
        assert!(validate_responses_sse_eof_tail(false, &[0xff]).is_err());
        assert!(validate_responses_sse_eof_tail(false, b"").is_ok());
        let mut invalid_frame = b"data: ".to_vec();
        invalid_frame.push(0xff);
        invalid_frame.extend_from_slice(b"\n\n");
        assert!(take_next_sse_event_block(&mut invalid_frame).is_err());
    }

    #[test]
    fn responses_stream_requires_response_completed_proof() {
        assert!(validate_completed_response_stream(false, false, &HashMap::new(), &[]).is_err());
        assert!(validate_completed_response_stream(false, true, &HashMap::new(), &[]).is_ok());
        assert!(validate_completed_response_stream(true, false, &HashMap::new(), &[]).is_ok());
    }

    #[test]
    fn append_round_to_history_keeps_assistant_text_before_tool_items() {
        let adapter = OpenAIResponsesStreamingAdapter {
            api_key: "",
            base_url: "",
            model: "gpt-test",
            reasoning: None,
        };
        let outcome = RoundOutcome {
            text: "I found the scripts and will inspect them now.".to_string(),
            thinking: String::new(),
            tool_calls: vec![FunctionCallItem {
                call_id: "call_1".to_string(),
                name: "read".to_string(),
                arguments: r#"{"path":"scripts/a.py"}"#.to_string(),
            }],
            provider_history_items: Vec::new(),
            usage: ChatUsage::default(),
            ttft_ms: None,
            stop_reason: None,
        };
        let executed = vec![ExecutedTool {
            model_call_ordinal: 0,
            call_id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: r#"{"path":"scripts/a.py"}"#.to_string(),
            clean_result: "file contents".to_string(),
            result_admission: None,
        }];

        let mut history = Vec::new();
        adapter.append_round_to_history(&mut history, 3, &outcome, &executed);

        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["type"], "message");
        assert_eq!(history[0]["role"], "assistant");
        assert_eq!(history[0]["content"][0]["type"], "output_text");
        assert_eq!(
            history[0]["content"][0]["text"],
            "I found the scripts and will inspect them now."
        );
        assert_eq!(history[0]["_oc_round"], "r3");
        assert_eq!(history[1]["type"], "function_call");
        assert_eq!(history[2]["type"], "function_call_output");
    }

    #[test]
    fn append_final_assistant_skips_empty_terminal_text() {
        let adapter = OpenAIResponsesStreamingAdapter {
            api_key: "",
            base_url: "",
            model: "gpt-test",
            reasoning: None,
        };
        let mut history = Vec::new();

        adapter.append_final_assistant(&mut history, "", "");

        assert!(history.is_empty());
    }

    #[test]
    fn response_completed_rejects_incomplete_pending_tool_calls() {
        let mut pending = HashMap::new();
        pending.insert(
            "call_1".into(),
            FunctionCallItem {
                call_id: "call_1".into(),
                name: "exec".into(),
                arguments: "{\"command\":\"dat".into(),
            },
        );
        assert!(validate_completed_response_stream(false, true, &pending, &[]).is_err());
    }

    // SSE event blocks must put the entire JSON payload on a single `data:`
    // line — `handle_openai_sse_event_block` filters by `starts_with("data:")`,
    // so multi-line `r#"data: {...}"#` literals get truncated to just `{`.
    fn sse_event_block(payload: serde_json::Value) -> String {
        format!("data: {}", payload)
    }

    fn observe_response_terminal_event(event_block: &str) -> anyhow::Result<bool> {
        let mut text = String::new();
        let mut thinking = String::new();
        let mut tool_calls = Vec::new();
        let mut provider_history_items = Vec::new();
        let mut pending = HashMap::new();
        let mut usage = ChatUsage::default();
        let mut first_token_time = None;
        let mut saw_response_completed = false;
        handle_openai_sse_event_block(
            "-",
            event_block,
            std::time::Instant::now(),
            &|_s: &str| {},
            &mut text,
            &mut thinking,
            &mut tool_calls,
            &mut provider_history_items,
            &mut pending,
            &mut usage,
            &mut first_token_time,
            &mut saw_response_completed,
        )?;
        Ok(saw_response_completed)
    }

    #[test]
    fn responses_terminal_proof_is_response_completed_only() {
        let legacy_done = sse_event_block(serde_json::json!({
            "type": "response.done",
            "response": { "output": [] }
        }));
        assert!(!observe_response_terminal_event(&legacy_done).unwrap());

        let completed = sse_event_block(serde_json::json!({
            "type": "response.completed",
            "response": { "output": [] }
        }));
        assert!(observe_response_terminal_event(&completed).unwrap());
        assert!(observe_response_terminal_event("data: {").is_err());
    }

    // Reasoning-item replay was deleted as part of the `store: false`
    // hardening: Hope Agent never persists `rs_*` ids back into the
    // conversation history because the server has no record of them.
    // The invariant "no reasoning items survive into normalized history"
    // is owned by `normalize_history_for_responses` and its test.
    #[test]
    fn response_completed_yields_output_text() {
        let event = sse_event_block(serde_json::json!({
            "type": "response.completed",
            "response": {
                "output": [
                    {
                        "type": "tool_search_call",
                        "execution": "server",
                        "call_id": null,
                        "status": "completed",
                        "arguments": { "paths": ["browser"] }
                    },
                    {
                        "type": "tool_search_output",
                        "execution": "server",
                        "call_id": null,
                        "status": "completed",
                        "tools": [{ "type": "function", "name": "browser" }]
                    },
                    {
                        "type": "reasoning",
                        "id": "rs_ok",
                        "summary": [],
                        "encrypted_content": "enc",
                        "status": "completed"
                    },
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "done"}]
                    }
                ]
            }
        }));

        let mut text = String::new();
        let mut thinking = String::new();
        let mut tool_calls = Vec::new();
        let mut provider_history_items = Vec::new();
        let mut pending = HashMap::new();
        let mut usage = ChatUsage::default();
        let mut first_token_time = None;
        let mut saw_response_completed = false;
        let on_delta = |_s: &str| {};

        handle_openai_sse_event_block(
            "-",
            &event,
            std::time::Instant::now(),
            &on_delta,
            &mut text,
            &mut thinking,
            &mut tool_calls,
            &mut provider_history_items,
            &mut pending,
            &mut usage,
            &mut first_token_time,
            &mut saw_response_completed,
        )
        .expect("handle event");

        assert!(saw_response_completed);
        assert_eq!(text, "done");
        assert_eq!(provider_history_items.len(), 3);
        assert_eq!(provider_history_items[0]["type"], "tool_search_call");
        assert_eq!(provider_history_items[1]["type"], "tool_search_output");
        assert_eq!(provider_history_items[2]["type"], "message");

        let adapter = OpenAIResponsesStreamingAdapter {
            api_key: "",
            base_url: "https://api.openai.com",
            model: "gpt-5.4",
            reasoning: None,
        };
        let outcome = RoundOutcome {
            text,
            thinking: String::new(),
            tool_calls: Vec::new(),
            provider_history_items,
            usage: ChatUsage::default(),
            ttft_ms: None,
            stop_reason: None,
        };
        let mut history = Vec::new();
        adapter.append_round_to_history(&mut history, 0, &outcome, &[]);
        assert_eq!(history.len(), 3, "assistant message must not be duplicated");
        assert_eq!(history[0]["type"], "tool_search_call");
        assert_eq!(history[1]["type"], "tool_search_output");
        assert_eq!(history[2]["type"], "message");
    }
}
