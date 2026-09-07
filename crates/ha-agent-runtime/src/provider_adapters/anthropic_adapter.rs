//! Anthropic Messages API adapter implementing [`StreamingChatAdapter`].
//!
//! Owns body construction (with `cache_control` ephemeral blocks for prompt
//! caching), HTTP send, SSE event decoding (text / thinking / tool_use blocks
//! + stop_reason), and history persistence in Anthropic's content-block shape.
//!
//! Phase 2 of the LLM call unification — the public tool loop lives in
//! [`super::super::streaming_loop`]. See `docs/architecture/agent/side-query.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::{AnthropicSseEvent, FunctionCallItem};
use super::super::config::{
    build_api_url, clamp_reasoning_effort, is_claude_5_model, map_think_anthropic_style,
};
use super::super::events::{
    emit_text_delta, emit_thinking_delta, expand_anthropic_image_markers_for_api,
    project_anthropic_image_markers_for_token_count,
};
use super::super::streaming_adapter::{
    observe_before_send, observe_response_started, ExecutedTool, PreparedProviderRequest,
    PreparedRequestVariant, ProviderAccountingInput, ProviderDispatchObserver,
    ProviderDispatchUnknown, ProviderEndpointKind, ProviderRequestShape, RoundOutcome,
    RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ChatUsage, ProviderFormat};
use crate::tool_defs::ToolProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnthropicThinkingPolicy {
    LegacyManual,
    AdaptiveEffort,
}

fn is_first_party_anthropic(base_url: &str) -> bool {
    crate::provider::is_direct_anthropic(base_url)
}

use crate::provider::anthropic_headers;

pub(crate) fn apply_thinking_binding_policy(base_url: &str, model: &str, body: &mut Value) {
    if !is_first_party_anthropic(base_url) {
        return;
    }
    if model == "claude-fable-5-1" && body.get("thinking").is_none() {
        body["thinking"] = json!({"type": "adaptive"});
    }
    if let Some(thinking) = body.get_mut("thinking").and_then(Value::as_object_mut) {
        thinking.insert(
            "block_binding".to_string(),
            json!({"prefix_mismatch_behavior": "drop_block"}),
        );
    }
}

fn anthropic_thinking_policy(base_url: &str, model: &str) -> AnthropicThinkingPolicy {
    if is_first_party_anthropic(base_url) && is_claude_5_model(model) {
        AnthropicThinkingPolicy::AdaptiveEffort
    } else {
        AnthropicThinkingPolicy::LegacyManual
    }
}

fn suppress_anthropic_temperature(base_url: &str, model: &str) -> bool {
    if !is_first_party_anthropic(base_url) {
        return false;
    }
    let model = model.to_ascii_lowercase();
    is_claude_5_model(&model)
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
}

fn supports_native_tool_search(base_url: &str, model: &str) -> bool {
    if !base_url.contains("api.anthropic.com") {
        return false;
    }

    // Anthropic's versioned tool-search tool is supported by Claude 4.5+
    // (but not Opus 4.1 and earlier) and Claude 5+. Parse the actual model
    // generation instead of substring-matching: `claude-3-5-sonnet-*`
    // contains "-5" but is a Claude 3 model.
    let parts: Vec<&str> = model.split('-').collect();
    if parts.first().copied() != Some("claude") {
        return false;
    }
    let version_start = if parts
        .get(1)
        .and_then(|part| part.parse::<u32>().ok())
        .is_some()
    {
        1
    } else {
        2
    };
    let Some(major) = parts
        .get(version_start)
        .and_then(|part| part.parse::<u32>().ok())
    else {
        return false;
    };
    let minor = parts
        .get(version_start + 1)
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    major >= 5 || (major == 4 && minor >= 5)
}

fn build_tools_with_cache_parts(
    tool_schemas: &[Value],
    deferred_tool_schemas: &[Value],
    eager_tool_count: usize,
    native_deferred: bool,
) -> Vec<Value> {
    let eager_end = eager_tool_count.min(tool_schemas.len());
    let mut tools = Vec::with_capacity(tool_schemas.len());
    let mut last_eager_position = None;
    for (index, tool) in tool_schemas.iter().enumerate() {
        if native_deferred && tool.get("name").and_then(|v| v.as_str()) == Some("tool_search") {
            continue;
        }
        if index < eager_end {
            last_eager_position = Some(tools.len());
        }
        tools.push(tool.clone());
    }
    // Cache only the stable directly-callable prefix. A deferred definition
    // must never carry cache_control.
    if let Some(last_eager) = last_eager_position.and_then(|index| tools.get_mut(index)) {
        last_eager["cache_control"] = json!({ "type": "ephemeral" });
    }
    if native_deferred {
        let loaded: std::collections::HashSet<String> = tools
            .iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();
        for schema in deferred_tool_schemas {
            let name = schema.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() || loaded.contains(name) {
                continue;
            }
            let mut deferred = schema.clone();
            deferred["defer_loading"] = json!(true);
            tools.push(deferred);
        }
        tools.push(json!({
            "type": "tool_search_tool_bm25_20251119",
            "name": "tool_search"
        }));
    }
    tools
}

fn build_tools_with_cache(req: &RoundRequest<'_>, native_deferred: bool) -> Vec<Value> {
    build_tools_with_cache_parts(
        req.tool_schemas,
        req.deferred_tool_schemas,
        req.eager_tool_count,
        native_deferred,
    )
}

fn build_anthropic_body(
    base_url: &str,
    model: &str,
    req: &RoundRequest<'_>,
) -> (Value, Vec<Value>, bool) {
    let mut system_blocks = vec![json!({
        "type": "text",
        "text": req.system_prompt,
        "cache_control": { "type": "ephemeral" }
    })];
    for suffix in super::super::streaming_adapter::dynamic_instruction_suffixes(req) {
        system_blocks.push(json!({
            "type": "text",
            "text": suffix,
        }));
    }
    let native_deferred = !req.is_final_round
        && !req.deferred_tool_schemas.is_empty()
        && supports_native_tool_search(base_url, model);
    let tools_with_cache = build_tools_with_cache(req, native_deferred);
    let effective_effort = req
        .reasoning_effort
        .and_then(|effort| clamp_reasoning_effort(model, effort));
    let mut messages = expand_anthropic_image_markers_for_api(req.history_for_api);
    if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
        append_anthropic_user_context(&mut messages, content);
    }
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "system": system_blocks,
        "messages": messages,
        "stream": true,
    });
    if !req.is_final_round {
        body["tools"] = json!(tools_with_cache);
    }
    if let Some(effort) = effective_effort.as_deref() {
        match anthropic_thinking_policy(base_url, model) {
            AnthropicThinkingPolicy::LegacyManual => {
                if let Some(think_config) = map_think_anthropic_style(Some(effort), req.max_tokens)
                {
                    body["thinking"] = think_config;
                }
            }
            AnthropicThinkingPolicy::AdaptiveEffort => {
                body["thinking"] = json!({ "type": "adaptive" });
                body["output_config"] = json!({ "effort": effort });
            }
        }
    }
    if let Some(temp) = req
        .temperature
        .filter(|_| !suppress_anthropic_temperature(base_url, model))
    {
        body["temperature"] = json!(temp);
    }
    if base_url.contains("api.anthropic.com") {
        body["cache_control"] = json!({ "type": "ephemeral" });
    }
    apply_thinking_binding_policy(base_url, model, &mut body);
    (body, tools_with_cache, native_deferred)
}

fn build_anthropic_count_body(base_url: &str, model: &str, req: &RoundRequest<'_>) -> Value {
    let (mut body, _, _) = build_anthropic_body(base_url, model, req);
    if let Some(body) = body.as_object_mut() {
        for field in ["stream", "max_tokens", "temperature", "cache_control"] {
            body.remove(field);
        }
    }
    body
}

fn append_anthropic_user_context(messages: &mut Vec<Value>, content: String) {
    let block = json!({ "type": "text", "text": content });
    if let Some(last) = messages
        .last_mut()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    {
        match last.get_mut("content") {
            Some(Value::Array(parts)) => parts.push(block),
            Some(Value::String(text)) => {
                let original = std::mem::take(text);
                last["content"] = json!([
                    { "type": "text", "text": original },
                    block
                ]);
            }
            _ => last["content"] = json!([block]),
        }
    } else {
        messages.push(json!({ "role": "user", "content": [block] }));
    }
}

pub(crate) struct AnthropicStreamingAdapter<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
    pub workspace_id: Option<&'a str>,
}

#[async_trait]
impl<'a> StreamingChatAdapter for AnthropicStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::Anthropic
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::Anthropic
    }

    fn supports_native_tool_search(&self) -> bool {
        supports_native_tool_search(self.base_url, self.model)
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_anthropic(history);
    }

    fn token_count_tool_schemas_for(
        &self,
        tool_schemas: &[Value],
        deferred_tool_schemas: &[Value],
        eager_tool_count: usize,
        is_final_round: bool,
    ) -> Vec<Value> {
        if is_final_round {
            return Vec::new();
        }
        let native_deferred = !deferred_tool_schemas.is_empty()
            && supports_native_tool_search(self.base_url, self.model);
        build_tools_with_cache_parts(
            tool_schemas,
            deferred_tool_schemas,
            eager_tool_count,
            native_deferred,
        )
    }

    fn token_count_history_for(&self, history: &[Value]) -> Vec<Value> {
        project_anthropic_image_markers_for_token_count(history)
    }

    fn prepare_history_for_api(&self, history: &[Value]) -> Vec<Value> {
        expand_anthropic_image_markers_for_api(history)
    }

    fn token_count_input_for(&self, req: &RoundRequest<'_>) -> ProviderAccountingInput {
        let stable_system = json!([{
            "type": "text",
            "text": req.system_prompt,
            "cache_control": { "type": "ephemeral" }
        }]);
        let mut dynamic_system = Vec::new();
        for suffix in super::super::streaming_adapter::dynamic_instruction_suffixes(req) {
            dynamic_system.push(json!({ "type": "text", "text": suffix }));
        }
        let mut dynamic_items = Vec::new();
        if !dynamic_system.is_empty() {
            dynamic_items.push(json!({
                "_provider_lane": "system",
                "content": dynamic_system
            }));
        }
        if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
            dynamic_items.push(json!({
                "role": "user",
                "content": [{ "type": "text", "text": content }]
            }));
        }
        ProviderAccountingInput {
            stable_prompt: serde_json::to_string(&json!({ "system": stable_system }))
                .unwrap_or_default(),
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
        let headers = anthropic_headers(self.base_url, self.api_key, self.workspace_id)?;
        let capability_key = format!(
            "anthropic_messages_count:{}",
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

        let body = build_anthropic_count_body(self.base_url, self.model, req);
        let api_url = build_api_url(self.base_url, "/v1/messages/count_tokens");
        let app_config = crate::config::cached_config();
        let ssrf = &app_config.ssrf;
        crate::security::ssrf::check_url(&api_url, ssrf.default_policy, &ssrf.trusted_hosts)
            .await?;
        let request = client
            .post(&api_url)
            .headers(headers)
            .header("content-type", "application/json")
            .json(&body);
        let response = match super::cancel::send_with_cancel(request, cancel).await {
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
        // Validate the credential binding before accounting or dispatch can
        // prepare any outbound work. Dispatch rechecks the same header contract.
        anthropic_headers(self.base_url, self.api_key, self.workspace_id)?;
        let (body, tools_with_cache, native_deferred) =
            build_anthropic_body(self.base_url, self.model, req);
        let prepared = PreparedProviderRequest::from_json(
            ProviderEndpointKind::AnthropicMessages,
            ProviderRequestShape::AnthropicMessages,
            self.model,
            req.round,
            req.session_id,
            req.reasoning_effort,
            req.vision_bridge_available,
            PreparedRequestVariant::Anthropic,
            &body,
        )?;
        super::super::token_manifest::log_round_manifest(
            "Anthropic",
            self.model,
            "messages",
            req,
            body.get("tools")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            prepared.identity.body_len as usize,
            native_deferred,
        );
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "debug",
                "agent",
                "agent::chat_anthropic::request",
                &format!(
                    "Anthropic API request round {}: {} messages, {} tools, body {}B",
                    req.round,
                    req.history_for_api.len(),
                    tools_with_cache.len(),
                    prepared.identity.body_len
                ),
                Some(
                    json!({
                        "round": req.round,
                        "endpoint_kind": "anthropic_messages",
                        "model": self.model,
                        "message_count": req.history_for_api.len(),
                        "tool_count": tools_with_cache.len(),
                        "body_size_bytes": prepared.identity.body_len,
                        "body_fingerprint": &prepared.identity.body_keyed_fingerprint,
                        "thinking_enabled": body.get("thinking").is_some(),
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
        let api_url = build_api_url(self.base_url, "/v1/messages");
        // ── Send.
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }
        let headers = anthropic_headers(self.base_url, self.api_key, self.workspace_id)?;
        observe_before_send(observer, prepared).await?;
        let request_start = std::time::Instant::now();
        let request = client
            .post(&api_url)
            .headers(headers)
            .header("content-type", "application/json")
            .body(prepared.body().to_vec());
        let resp = match super::cancel::send_with_cancel(request, cancel).await {
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

        // ── Log response status with rate-limit headers for debugging.
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
                "anthropic-model-id": headers.get("anthropic-model-id").and_then(|v| v.to_str().ok()),
                "retry-after": headers.get("retry-after").and_then(|v| v.to_str().ok()),
            });
            logger.log(
                "debug",
                "agent",
                "agent::chat_anthropic::response",
                &format!(
                    "Anthropic API response: status={}, request_id={}, ttfb={}ms",
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
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let error_text = match super::cancel::read_text_with_cancel(resp, cancel).await {
                Ok(Some(text)) => text,
                Ok(None) => return Ok(super::cancel::cancelled_round_outcome()),
                Err(_) => String::new(),
            };
            if let Some(logger) = crate::get_logger() {
                let error_fingerprint = crate::cache_routing::audit_fingerprint(
                    "anthropic-error",
                    error_text.as_bytes(),
                );
                logger.log(
                    "error",
                    "agent",
                    "agent::chat_anthropic::error",
                    &format!("Anthropic API error ({status})"),
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
                "Anthropic",
                status,
                &error_text,
            )
            .with_retry_after_header(retry_after.as_deref())
            .into());
        }

        // ── Parse SSE stream.
        let (
            text,
            tool_calls,
            provider_history_items,
            stop_reason,
            mut usage,
            thinking_text,
            ttft_ms,
        ) = parse_anthropic_sse(resp, request_start, cancel, on_delta).await?;
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }

        // Log tool loop progress (moved here from the old chat_anthropic so
        // the orchestrator stays oblivious to which tools were requested).
        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_anthropic::tool_loop",
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

        usage.normalize_anthropic_round();
        super::super::token_manifest::log_round_usage(
            "Anthropic",
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
            stop_reason,
        })
    }

    fn append_round_to_history(
        &self,
        history: &mut Vec<Value>,
        round: u32,
        outcome: &RoundOutcome,
        executed: &[ExecutedTool],
    ) {
        // Preserve provider block order, opaque signatures and redacted data.
        let mut assistant_content: Vec<Value> = Vec::new();
        if outcome.provider_history_items.is_empty() && !outcome.text.is_empty() {
            assistant_content.push(json!({
                "type": "text",
                "text": outcome.text,
            }));
        }
        // Text shown in the UI can include local compatibility notices; it
        // must not be substituted for these raw provider content blocks.
        for mut block in outcome.provider_history_items.iter().cloned() {
            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                let Some(tool) = executed.iter().find(|tool| {
                    block.get("id").and_then(Value::as_str) == Some(tool.call_id.as_str())
                }) else {
                    continue;
                };
                // Tool hooks can rewrite arguments. Preserve the envelope but
                // record the input actually executed; never leave orphaned
                // tool_use blocks after cancellation or a partial tool batch.
                block["input"] = serde_json::from_str(&tool.arguments).unwrap_or(json!({}));
            }
            assistant_content.push(block);
        }
        for tc in executed {
            if assistant_content.iter().any(|block| {
                block.get("type").and_then(Value::as_str) == Some("tool_use")
                    && block.get("id").and_then(Value::as_str) == Some(tc.call_id.as_str())
            }) {
                continue;
            }
            let args: Value = serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
            assistant_content.push(json!({
                "type": "tool_use",
                "id": tc.call_id,
                "name": tc.name,
                "input": args,
            }));
        }
        crate::context_compact::push_and_stamp(
            history,
            json!({ "role": "assistant", "content": assistant_content }),
            round,
        );

        // Build user content with tool_result blocks (one per executed tool).
        let mut tool_results: Vec<Value> = Vec::new();
        for et in executed {
            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": et.call_id,
                "content": et.clean_result,
            }));
        }
        if !tool_results.is_empty() {
            crate::context_compact::push_and_stamp(
                history,
                json!({ "role": "user", "content": tool_results }),
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
        let mut final_content: Vec<Value> = Vec::new();
        if !final_text.is_empty() {
            final_content.push(json!({
                "type": "text",
                "text": final_text,
            }));
        }
        if !final_content.is_empty() {
            history.push(json!({ "role": "assistant", "content": final_content }));
        }
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        // Anthropic's terminal signal is `stop_reason != "tool_use"`. If the
        // model picked tools in this round but the stop reason isn't
        // "tool_use" (e.g. "max_tokens"), bail before executing them — sending
        // tool_results back without a tool_use stop would desync the chain.
        outcome.tool_calls.is_empty() || outcome.stop_reason.as_deref() != Some("tool_use")
    }
}

/// Parse Anthropic SSE stream. Returns
/// `(collected_text, tool_calls, provider_items, stop_reason, usage, thinking, ttft_ms)`.
///
/// Free function (not a `&self` method) because none of the streaming state
/// lives on `AssistantAgent` — only `cancel`, `on_delta`, and accumulators.
fn take_next_anthropic_sse_event_block(buffer: &mut Vec<u8>) -> Result<Option<String>> {
    let lf = buffer
        .windows(b"\n\n".len())
        .position(|window| window == b"\n\n")
        .map(|idx| (idx, 2));
    let crlf = buffer
        .windows(b"\r\n\r\n".len())
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| (idx, 4));
    let (idx, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) => {
            if left.0 <= right.0 {
                left
            } else {
                right
            }
        }
        (Some(found), None) | (None, Some(found)) => found,
        (None, None) => return Ok(None),
    };
    let mut consumed: Vec<u8> = buffer.drain(..idx + delimiter_len).collect();
    consumed.truncate(idx);
    let block = String::from_utf8(consumed)
        .map_err(|_| anyhow::anyhow!("Anthropic SSE event contained invalid UTF-8"))?;
    Ok(Some(block))
}

fn validate_anthropic_sse_eof_tail(cancelled: bool, buffer: &[u8]) -> Result<()> {
    if cancelled || buffer.is_empty() {
        return Ok(());
    }
    std::str::from_utf8(buffer)
        .map_err(|_| anyhow::anyhow!("Anthropic SSE ended with invalid UTF-8"))?;
    anyhow::bail!("Anthropic SSE ended with an incomplete event")
}

fn validate_anthropic_stream_completion(
    cancelled: bool,
    saw_message_stop: bool,
    has_open_content_block: bool,
) -> Result<()> {
    if cancelled {
        return Ok(());
    }
    if !saw_message_stop {
        anyhow::bail!("Anthropic SSE ended before message_stop")
    }
    if has_open_content_block {
        anyhow::bail!("Anthropic SSE message_stop arrived with an unfinished content block")
    }
    Ok(())
}

fn decode_anthropic_sse_event(data: &str) -> Result<(Value, AnthropicSseEvent)> {
    let raw_event = serde_json::from_str::<Value>(data)
        .map_err(|err| anyhow::anyhow!("Anthropic SSE event could not be decoded: {err}"))?;
    let event = serde_json::from_value::<AnthropicSseEvent>(raw_event.clone())
        .map_err(|err| anyhow::anyhow!("Anthropic SSE event could not be decoded: {err}"))?;
    Ok((raw_event, event))
}

fn append_block_string(block: &mut Value, key: &str, delta: &str) {
    if let Some(Value::String(value)) = block.get_mut(key) {
        value.push_str(delta);
    } else {
        block[key] = json!(delta);
    }
}

pub(crate) fn report_thinking_transformations(
    transformations: Option<&Value>,
    reported: &mut std::collections::HashSet<(String, String)>,
    text: &mut String,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) {
    let Some(transformations) = transformations.and_then(Value::as_array) else {
        return;
    };
    for item in transformations {
        let reason = item.get("reason").and_then(Value::as_str).unwrap_or("");
        if item.get("type").and_then(Value::as_str) != Some("thinking_dropped")
            || !matches!(reason, "prefix_binding_mismatch" | "model_binding_mismatch")
        {
            continue;
        }
        let path = item.get("path").and_then(Value::as_str).unwrap_or("");
        if !reported.insert((path.to_string(), reason.to_string())) {
            continue;
        }
        crate::app_warn!(
            "provider",
            "anthropic_thinking_binding",
            "Anthropic omitted a thinking block from this request: {}",
            reason
        );
        if reported
            .iter()
            .filter(|(_, reported_reason)| reported_reason == reason)
            .count()
            > 1
        {
            continue;
        }
        let notice = if reason == "model_binding_mismatch" {
            "\n\n[提示：当前模型无法使用部分历史思考，本地原始记录已保留。]\n\n"
        } else {
            "\n\n[提示：上下文前缀已变化，本次请求未使用部分历史思考，本地原始记录已保留。]\n\n"
        };
        emit_text_delta(&on_delta, notice);
        text.push_str(notice);
    }
}

pub(crate) async fn parse_anthropic_sse(
    resp: reqwest::Response,
    request_start: std::time::Instant,
    cancel: &Arc<AtomicBool>,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    Vec<FunctionCallItem>,
    Vec<Value>,
    Option<String>,
    ChatUsage,
    String,
    Option<u64>,
)> {
    let mut collected_text = String::new();
    let mut collected_thinking = String::new();
    let mut tool_calls: Vec<FunctionCallItem> = Vec::new();
    let mut streamed_content_blocks: Vec<Value> = Vec::new();
    let mut current_text_block: Option<usize> = None;
    let mut current_thinking_block: Option<usize> = None;
    let mut current_tool_block: Option<usize> = None;
    let mut open_block_index: Option<usize> = None;
    let mut reported_transformations = std::collections::HashSet::new();
    // Single in-flight tool-use block — Anthropic streams them sequentially,
    // not in parallel, so a single slot is sufficient.
    let mut current_tool: Option<(usize, FunctionCallItem)> = None;
    let mut current_native_block: Option<(usize, usize, String)> = None;
    let mut in_thinking_block = false;
    let mut usage = ChatUsage::default();
    let mut stop_reason: Option<String> = None;
    let mut first_token_time: Option<u64> = None;

    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut saw_message_stop = false;

    'anthropic_stream: while let Some(chunk) =
        super::cancel::next_chunk_or_cancel(&mut stream, cancel).await
    {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) if cancel.load(Ordering::SeqCst) => break,
            Err(err) => return Err(err.into()),
        };
        buffer.extend_from_slice(&chunk);

        while let Some(event_block) = take_next_anthropic_sse_event_block(&mut buffer)? {
            // SSE event format: "event: <type>\ndata: <json>"
            let mut event_name = String::new();
            let mut data_lines = Vec::new();
            for line in event_block.lines() {
                if let Some(ev) = line.strip_prefix("event:") {
                    event_name = ev.trim().to_string();
                } else if let Some(d) = line.strip_prefix("data:") {
                    data_lines.push(d.trim().to_string());
                }
            }
            if data_lines.is_empty() {
                continue;
            }
            let data = data_lines.join("\n");
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if saw_message_stop {
                anyhow::bail!("Anthropic SSE emitted data after message_stop")
            }

            let (raw_event, event) = decode_anthropic_sse_event(&data)?;
            let data_event_name = event.event_type.as_deref().unwrap_or("");
            if !event_name.is_empty()
                && !data_event_name.is_empty()
                && event_name != data_event_name
            {
                anyhow::bail!("Anthropic SSE event name did not match its data type")
            }
            if event_name.is_empty() {
                event_name = data_event_name.to_string();
            }
            match event_name.as_str() {
                "content_block_start" => {
                    if open_block_index.replace(event.index.unwrap_or(0)).is_some() {
                        anyhow::bail!(
                            "Anthropic SSE started a block before closing the previous block"
                        )
                    }
                    if let Some(block) = &event.content_block {
                        match block.block_type.as_deref() {
                            Some("tool_use") => {
                                let idx = event.index.unwrap_or(0);
                                let raw_block = raw_event["content_block"].clone();
                                current_tool_block = Some(streamed_content_blocks.len());
                                streamed_content_blocks.push(raw_block);
                                current_tool = Some((
                                    idx,
                                    FunctionCallItem {
                                        // Synthesize a stable id if the block
                                        // omits one, so the tool loop,
                                        // persistence, and PreToolUse /
                                        // PostToolUse hooks all correlate on a
                                        // non-empty tool_use_id rather than "".
                                        call_id: block
                                            .id
                                            .clone()
                                            .filter(|s| !s.is_empty())
                                            .unwrap_or_else(|| format!("toolu_idx_{idx}")),
                                        name: block.name.clone().unwrap_or_default(),
                                        arguments: String::new(),
                                    },
                                ));
                            }
                            Some("thinking") => {
                                in_thinking_block = true;
                                current_thinking_block = Some(streamed_content_blocks.len());
                                streamed_content_blocks.push(raw_event["content_block"].clone());
                            }
                            Some("redacted_thinking") => {
                                streamed_content_blocks.push(raw_event["content_block"].clone());
                            }
                            Some("text") => {
                                let block = raw_event
                                    .get("content_block")
                                    .cloned()
                                    .unwrap_or_else(|| json!({ "type": "text", "text": "" }));
                                current_text_block = Some(streamed_content_blocks.len());
                                streamed_content_blocks.push(block);
                            }
                            Some(
                                "server_tool_use" | "tool_search_tool_result" | "tool_reference",
                            ) => {
                                if let Some(raw_block) = raw_event.get("content_block").cloned() {
                                    let item_pos = streamed_content_blocks.len();
                                    streamed_content_blocks.push(raw_block);
                                    if block.block_type.as_deref() == Some("server_tool_use") {
                                        current_native_block = Some((
                                            event.index.unwrap_or(0),
                                            item_pos,
                                            String::new(),
                                        ));
                                    }
                                }
                            }
                            _ => {
                                if let Some(block) = raw_event.get("content_block") {
                                    streamed_content_blocks.push(block.clone());
                                }
                            }
                        }
                    }
                }
                "content_block_delta" => {
                    if open_block_index != Some(event.index.unwrap_or(0)) {
                        anyhow::bail!("Anthropic SSE delta did not match an open content block")
                    }
                    if let Some(delta) = &event.delta {
                        match delta.delta_type.as_deref() {
                            Some("thinking_delta") => {
                                if let Some(text) = raw_event
                                    .pointer("/delta/thinking")
                                    .and_then(Value::as_str)
                                    .or(delta.text.as_deref())
                                {
                                    if first_token_time.is_none() {
                                        first_token_time =
                                            Some(request_start.elapsed().as_millis() as u64);
                                    }
                                    emit_thinking_delta(&on_delta, text);
                                    collected_thinking.push_str(text);
                                    if let Some(pos) = current_thinking_block {
                                        append_block_string(
                                            &mut streamed_content_blocks[pos],
                                            "thinking",
                                            text,
                                        );
                                    }
                                }
                            }
                            Some("signature_delta") => {
                                if let (Some(pos), Some(signature)) = (
                                    current_thinking_block,
                                    raw_event
                                        .pointer("/delta/signature")
                                        .and_then(Value::as_str),
                                ) {
                                    append_block_string(
                                        &mut streamed_content_blocks[pos],
                                        "signature",
                                        signature,
                                    );
                                }
                            }
                            Some("text_delta") => {
                                if let Some(text) = &delta.text {
                                    if first_token_time.is_none() {
                                        first_token_time =
                                            Some(request_start.elapsed().as_millis() as u64);
                                    }
                                    emit_text_delta(&on_delta, text);
                                    collected_text.push_str(text);
                                    if let Some(item_pos) = current_text_block {
                                        if let Some(Value::String(block_text)) =
                                            streamed_content_blocks[item_pos].get_mut("text")
                                        {
                                            block_text.push_str(text);
                                        }
                                    }
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(partial) = &delta.partial_json {
                                    if let Some((_, ref mut tc)) = current_tool {
                                        tc.arguments.push_str(partial);
                                    }
                                    if let Some((_, _, ref mut input)) = current_native_block {
                                        input.push_str(partial);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "content_block_stop" => {
                    if open_block_index.take() != Some(event.index.unwrap_or(0)) {
                        anyhow::bail!("Anthropic SSE stop did not match an open content block")
                    }
                    if in_thinking_block {
                        in_thinking_block = false;
                    }
                    current_thinking_block = None;
                    current_text_block = None;
                    if let Some((_, mut tc)) = current_tool.take() {
                        if let Some(pos) = current_tool_block.take() {
                            if tc.arguments.is_empty() {
                                tc.arguments = streamed_content_blocks[pos]
                                    .get("input")
                                    .unwrap_or(&json!({}))
                                    .to_string();
                            } else {
                                streamed_content_blocks[pos]["input"] =
                                    serde_json::from_str(&tc.arguments)?;
                            }
                            streamed_content_blocks[pos]["id"] = json!(tc.call_id);
                        }
                        tool_calls.push(tc);
                    }
                    if let Some((index, item_pos, input)) = current_native_block.take() {
                        if index == event.index.unwrap_or(index) && !input.is_empty() {
                            if let Ok(input) = serde_json::from_str::<Value>(&input) {
                                streamed_content_blocks[item_pos]["input"] = input;
                            }
                        }
                    }
                }
                "message_start" => {
                    report_thinking_transformations(
                        raw_event.pointer("/message/input_transformations"),
                        &mut reported_transformations,
                        &mut collected_text,
                        on_delta,
                    );
                    if let Some(msg) = &event.message {
                        if let Some(u) = &msg.usage {
                            if let Some(it) = u.input_tokens {
                                usage.input_tokens = it;
                                usage.input_coverage =
                                    crate::token_accounting::UsageCoverage::Complete;
                            }
                            if let Some(ct) = u.cache_creation_input_tokens {
                                usage.cache_creation_input_tokens = ct;
                            }
                            if let Some(cr) = u.cache_read_input_tokens {
                                usage.cache_read_input_tokens = cr;
                            }
                        }
                    }
                }
                "message_delta" => {
                    report_thinking_transformations(
                        raw_event
                            .get("input_transformations")
                            .or_else(|| raw_event.pointer("/delta/input_transformations")),
                        &mut reported_transformations,
                        &mut collected_text,
                        on_delta,
                    );
                    if let Some(delta) = &event.delta {
                        if let Some(reason) = &delta.stop_reason {
                            stop_reason = Some(reason.clone());
                        }
                    }
                    if let Some(u) = &event.usage {
                        if let Some(ot) = u.output_tokens {
                            usage.output_tokens = ot;
                            usage.output_coverage =
                                crate::token_accounting::UsageCoverage::Complete;
                        }
                    }
                }
                "message_stop" => {
                    saw_message_stop = true;
                }
                "error" => {
                    let provider_error = event.error.as_ref();
                    let msg = provider_error
                        .and_then(|error| error.message.as_deref())
                        .unwrap_or("Unknown Anthropic error");
                    return Err(crate::failover::ProviderApiError::from_stream_event(
                        "Anthropic",
                        None,
                        provider_error.and_then(|error| error.error_type.as_deref()),
                        Some(msg),
                        format!("Anthropic error: {msg}"),
                    )
                    .into());
                }
                _ => {}
            }
            if saw_message_stop {
                break 'anthropic_stream;
            }
        }
    }

    let cancelled = cancel.load(Ordering::SeqCst);
    if cancelled {
        stop_reason = Some("cancelled".to_string());
        let _ = current_tool.take();
        tool_calls.clear();
    }
    validate_anthropic_sse_eof_tail(cancelled, &buffer)?;
    validate_anthropic_stream_completion(
        cancelled,
        saw_message_stop,
        open_block_index.is_some()
            || current_tool.is_some()
            || current_native_block.is_some()
            || current_text_block.is_some()
            || in_thinking_block,
    )?;

    if let Some(logger) = crate::get_logger() {
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        logger.log(
            "debug",
            "agent",
            "agent::parse_anthropic_sse::done",
            &format!(
                "Anthropic SSE done: {}chars text, {} tool_calls, stop={:?}",
                collected_text.len(),
                tool_calls.len(),
                stop_reason
            ),
            Some(
                json!({
                    "text_length": collected_text.len(),
                    "tool_calls": tool_names,
                    "tool_call_count": tool_calls.len(),
                    "stop_reason": stop_reason,
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_creation": usage.cache_creation_input_tokens,
                        "cache_read": usage.cache_read_input_tokens,
                    }
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
        streamed_content_blocks,
        stop_reason,
        usage,
        collected_thinking,
        first_token_time,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        build_anthropic_body, build_anthropic_count_body, build_tools_with_cache,
        decode_anthropic_sse_event, supports_native_tool_search,
        take_next_anthropic_sse_event_block, validate_anthropic_sse_eof_tail,
        validate_anthropic_stream_completion, AnthropicStreamingAdapter,
    };
    use crate::agent::streaming_adapter::{RoundOutcome, RoundRequest, StreamingChatAdapter};

    #[tokio::test]
    async fn invalid_workspace_fails_before_request_preparation_or_token_count_network() {
        let req = super::super::test_support::round_request(&[]);
        let client = reqwest::Client::new();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        for workspace_id in ["", "wrkspc_fixture"] {
            let adapter = AnthropicStreamingAdapter {
                api_key: "synthetic-key",
                // Even URL/SSRF validation must not run before the local
                // binding check. This fixture can never reach a real endpoint.
                base_url: "not-a-url",
                model: "claude-fable-5-1",
                workspace_id: Some(workspace_id),
            };
            let error = adapter
                .prepare_round_request(&req)
                .err()
                .expect("invalid binding must fail before request preparation");
            assert!(error
                .downcast_ref::<crate::failover::ProviderRequestContractError>()
                .is_some());
            let error = adapter
                .count_input_tokens(&client, &req, &cancel)
                .await
                .unwrap_err();
            assert!(error
                .downcast_ref::<crate::failover::ProviderRequestContractError>()
                .is_some());
        }
    }

    #[tokio::test]
    async fn thinking_signatures_redactions_and_tool_order_survive_every_round() {
        use serde_json::json;
        for with_summary in [false, true] {
            for with_tool in [false, true] {
                let mut events = vec![
                    json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
                ];
                if with_summary {
                    events.push(json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"visible summary"}}));
                }
                events.extend([
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-"}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signature"}}),
                    json!({"type":"content_block_stop","index":0}),
                    json!({"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"opaque-data"}}),
                    json!({"type":"content_block_stop","index":1}),
                    json!({"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}),
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"answer"}}),
                    json!({"type":"content_block_stop","index":2}),
                ]);
                if with_tool {
                    events.extend([
                        json!({"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}),
                        json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"fixture\"}"}}),
                        json!({"type":"content_block_stop","index":3}),
                    ]);
                }
                events.extend([
                    json!({"type":"message_delta","delta":{"stop_reason":if with_tool {"tool_use"} else {"end_turn"}},"usage":{"output_tokens":20}}),
                    json!({"type":"message_stop"}),
                ]);
                let response = super::super::test_support::sse_response(&events).await;
                let (text, tool_calls, raw, stop_reason, usage, thinking, ttft_ms) =
                    super::parse_anthropic_sse(
                        response,
                        std::time::Instant::now(),
                        &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        &|_| {},
                    )
                    .await
                    .unwrap();
                assert_eq!(text, "answer");
                assert_eq!(thinking, if with_summary { "visible summary" } else { "" });
                let mut expected = vec![
                    json!({"type":"thinking","thinking":thinking,"signature":"opaque-signature"}),
                    json!({"type":"redacted_thinking","data":"opaque-data"}),
                    json!({"type":"text","text":"answer"}),
                ];
                if with_tool {
                    expected.push(json!({"type":"tool_use","id":"toolu_1","name":"read","input":{"path":"fixture"}}));
                }
                assert_eq!(raw, expected);
                let outcome = RoundOutcome {
                    text,
                    tool_calls,
                    provider_history_items: raw,
                    stop_reason,
                    usage,
                    thinking,
                    ttft_ms,
                };
                let adapter = AnthropicStreamingAdapter {
                    api_key: "",
                    base_url: "https://api.anthropic.com",
                    model: "claude-fable-5-1",
                    workspace_id: None,
                };
                let mut history = Vec::new();
                let executed = if with_tool {
                    vec![crate::agent::streaming_adapter::ExecutedTool {
                        model_call_ordinal: 0,
                        call_id: "toolu_1".into(),
                        name: "read".into(),
                        arguments: r#"{"path":"fixture"}"#.into(),
                        clean_result: "synthetic result".into(),
                        result_admission: None,
                    }]
                } else {
                    vec![]
                };
                adapter.append_round_to_history(&mut history, 1, &outcome, &executed);
                assert_eq!(history[0]["content"], json!(expected));
                let projection = crate::context_compact::prepare_messages_for_api(&history);
                assert_eq!(projection[0]["content"], json!(expected));
                assert_eq!(projection[0].get("_oc_round"), None);
            }
        }
    }

    #[test]
    fn anthropic_history_keeps_thinking_but_only_records_effectively_executed_tools() {
        use serde_json::json;
        let raw = vec![
            json!({"type":"thinking","thinking":"","signature":"opaque"}),
            json!({"type":"tool_use","id":"done","name":"read","input":{"path":"original"}}),
            json!({"type":"tool_use","id":"cancelled","name":"read","input":{}}),
        ];
        let outcome = RoundOutcome {
            text: String::new(),
            thinking: String::new(),
            tool_calls: vec![],
            provider_history_items: raw.clone(),
            stop_reason: Some("tool_use".into()),
            usage: Default::default(),
            ttft_ms: None,
        };
        let executed = vec![crate::agent::streaming_adapter::ExecutedTool {
            model_call_ordinal: 0,
            call_id: "done".into(),
            name: "read".into(),
            arguments: r#"{"path":"effective"}"#.into(),
            clean_result: "result".into(),
            result_admission: None,
        }];
        let adapter = AnthropicStreamingAdapter {
            api_key: "",
            base_url: "https://api.anthropic.com",
            model: "claude-fable-5-1",
            workspace_id: None,
        };
        let mut history = Vec::new();
        adapter.append_round_to_history(&mut history, 0, &outcome, &executed);
        assert_eq!(history[0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(history[0]["content"][0], raw[0]);
        assert_eq!(history[0]["content"][1]["input"]["path"], "effective");
        assert_eq!(history[1]["content"][0]["tool_use_id"], "done");
        assert_eq!(outcome.provider_history_items, raw);
    }

    #[test]
    fn fable_prefix_changes_and_compacted_suffix_use_explicit_binding_policy() {
        use serde_json::json;
        let history = vec![
            json!({"role":"user","content":"compaction summary"}),
            json!({"role":"assistant","content":[
                {"type":"thinking","thinking":"","signature":"opaque"},
                {"type":"text","text":"retained suffix"}
            ]}),
            json!({"role":"user","content":"continue"}),
        ];
        let mut req = super::super::test_support::round_request(&history);
        req.run_instruction_suffix = Some("changed execution instruction");
        req.active_memory_suffix = Some("changed recall data");
        let (body, _, _) =
            build_anthropic_body("https://api.anthropic.com", "claude-fable-5-1", &req);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(
            body["thinking"]["block_binding"]["prefix_mismatch_behavior"],
            "drop_block"
        );
        assert_eq!(body["messages"][1], history[1]);
        assert!(body.get("tool_choice").is_none());
        let count =
            build_anthropic_count_body("https://api.anthropic.com", "claude-fable-5-1", &req);
        assert_eq!(count["thinking"], body["thinking"]);
        assert_eq!(count["messages"], body["messages"]);
        let (relay, _, _) = build_anthropic_body("https://relay.example", "claude-fable-5-1", &req);
        assert!(relay.get("thinking").is_none());

        let transformations = json!([
            {"type":"thinking_dropped","path":"messages.1.content.0","reason":"prefix_binding_mismatch"},
            {"type":"thinking_dropped","path":"messages.2.content.0","reason":"model_binding_mismatch"},
        ]);
        let mut seen = std::collections::HashSet::new();
        let mut visible = String::new();
        super::report_thinking_transformations(
            Some(&transformations),
            &mut seen,
            &mut visible,
            &|_| {},
        );
        let once = visible.clone();
        super::report_thinking_transformations(
            Some(&transformations),
            &mut seen,
            &mut visible,
            &|_| {},
        );
        assert_eq!(visible, once);
        assert!(visible.contains("前缀已变化"));
        assert!(visible.contains("当前模型无法使用"));
        assert!(!visible.contains("opaque"));
        assert_eq!(history[1]["content"][0]["signature"], "opaque");
    }

    #[tokio::test]
    async fn unclosed_redacted_block_fails_instead_of_persisting_partial_thinking() {
        let response = super::super::test_support::sse_response(&[
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"partial"}}),
            serde_json::json!({"type":"message_stop"}),
        ]).await;
        assert!(super::parse_anthropic_sse(
            response,
            std::time::Instant::now(),
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            &|_| {}
        )
        .await
        .is_err());
    }

    #[test]
    fn anthropic_stream_requires_message_stop_and_closed_blocks() {
        assert!(validate_anthropic_stream_completion(false, false, false).is_err());
        assert!(validate_anthropic_stream_completion(false, true, true).is_err());
        assert!(validate_anthropic_stream_completion(false, true, false).is_ok());
        assert!(validate_anthropic_stream_completion(true, false, true).is_ok());
    }

    #[test]
    fn anthropic_sse_framing_accepts_crlf_message_stop() {
        let mut buffer = concat!(
            "event: message_stop\r\n",
            "data: {\"type\":\"message_stop\"}\r\n\r\n",
            "rest"
        )
        .as_bytes()
        .to_vec();
        let block = take_next_anthropic_sse_event_block(&mut buffer)
            .unwrap()
            .unwrap();
        assert!(block.contains("event: message_stop"));
        assert!(block.contains("\"type\":\"message_stop\""));
        assert_eq!(buffer, b"rest");
        assert!(decode_anthropic_sse_event(r#"{"type":"message_stop"}"#).is_ok());
        assert!(decode_anthropic_sse_event("{").is_err());
    }

    #[test]
    fn anthropic_sse_framing_preserves_unicode_split_inside_scalar() {
        let payload = serde_json::json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "中文🙂" }
        });
        let wire = format!("event: content_block_delta\ndata: {payload}\n\n").into_bytes();
        let scalar_start = wire
            .windows("中".len())
            .position(|window| window == "中".as_bytes())
            .expect("Chinese scalar in fixture");
        let split = scalar_start + 1;
        let mut buffer = wire[..split].to_vec();
        assert!(take_next_anthropic_sse_event_block(&mut buffer)
            .unwrap()
            .is_none());
        buffer.extend_from_slice(&wire[split..]);
        let block = take_next_anthropic_sse_event_block(&mut buffer)
            .unwrap()
            .expect("complete SSE frame");
        let data = block
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .expect("data line")
            .trim();
        let (raw, _) = decode_anthropic_sse_event(data).expect("valid Unicode JSON event");
        assert_eq!(raw["delta"]["text"], "中文🙂");
        assert!(buffer.is_empty());
    }

    #[test]
    fn anthropic_sse_eof_and_invalid_utf8_fail_closed() {
        assert!(validate_anthropic_sse_eof_tail(false, b"data: partial").is_err());
        assert!(validate_anthropic_sse_eof_tail(false, &[0xff]).is_err());
        assert!(validate_anthropic_sse_eof_tail(false, b"").is_ok());
        let mut invalid_frame = b"data: ".to_vec();
        invalid_frame.push(0xff);
        invalid_frame.extend_from_slice(b"\n\n");
        assert!(take_next_anthropic_sse_event_block(&mut invalid_frame).is_err());
    }

    #[test]
    fn native_deferred_tools_never_receive_cache_control() {
        let loaded = vec![
            serde_json::json!({
                "name": "tool_search",
                "input_schema": { "type": "object" }
            }),
            serde_json::json!({
                "name": "read",
                "input_schema": { "type": "object" }
            }),
            serde_json::json!({
                "name": "browser__snapshot",
                "input_schema": { "type": "object" }
            }),
        ];
        let deferred = vec![serde_json::json!({
            "name": "browser",
            "input_schema": { "type": "object", "properties": { "action": { "type": "string" } } }
        })];
        let history = Vec::new();
        let req = RoundRequest {
            session_id: Some("session"),
            system_prompt: "stable",
            run_instruction_suffix: None,
            run_data_suffix: None,
            awareness_suffix: None,
            active_memory_suffix: None,
            legacy_memory_suffix: None,
            coding_profile_suffix: None,
            procedure_memory_suffix: None,
            related_notes_suffix: None,
            attached_knowledge_suffix: None,
            capability_catalog_suffix: None,
            user_profile_suffix: None,
            environment_context_suffix: None,
            lsp_diagnostics_suffix: None,
            task_reminder_suffix: None,
            tool_schemas: &loaded,
            deferred_tool_schemas: &deferred,
            eager_tool_count: 2,
            deferred_tool_count: 1,
            activated_tool_count: 1,
            prompt_cache_key: None,
            history_for_api: &history,
            vision_bridge_available: false,
            reasoning_effort: None,
            temperature: None,
            max_tokens: 100,
            is_final_round: false,
            round: 0,
        };
        let tools = build_tools_with_cache(&req, true);
        assert!(tools[0].get("cache_control").is_some());
        assert_eq!(tools[0]["name"], "read");
        assert_eq!(tools[1]["name"], "browser__snapshot");
        assert!(tools[1].get("cache_control").is_none());
        assert_eq!(tools[2]["name"], "browser");
        assert_eq!(tools[2]["defer_loading"], true);
        assert!(tools[2].get("cache_control").is_none());
        assert_eq!(tools[3]["type"], "tool_search_tool_bm25_20251119");
        assert!(supports_native_tool_search(
            "https://api.anthropic.com",
            "claude-sonnet-4-5"
        ));
        assert!(!supports_native_tool_search(
            "https://compatible.example",
            "claude-sonnet-4-5"
        ));
        assert!(!supports_native_tool_search(
            "https://api.anthropic.com",
            "claude-3-5-sonnet-20241022"
        ));
        assert!(!supports_native_tool_search(
            "https://api.anthropic.com",
            "claude-opus-4-1"
        ));
        assert!(supports_native_tool_search(
            "https://api.anthropic.com",
            "claude-mythos-5"
        ));
    }

    #[test]
    fn anthropic_model_request_policy_uses_adaptive_effort_and_safe_sampling() {
        let tools = Vec::new();
        let deferred = Vec::new();
        let history = vec![serde_json::json!({ "role": "user", "content": "question" })];
        let req = RoundRequest {
            session_id: Some("session"),
            system_prompt: "stable",
            run_instruction_suffix: None,
            run_data_suffix: None,
            awareness_suffix: None,
            active_memory_suffix: None,
            legacy_memory_suffix: None,
            coding_profile_suffix: None,
            procedure_memory_suffix: None,
            related_notes_suffix: None,
            attached_knowledge_suffix: None,
            capability_catalog_suffix: None,
            user_profile_suffix: None,
            environment_context_suffix: None,
            lsp_diagnostics_suffix: None,
            task_reminder_suffix: None,
            tool_schemas: &tools,
            deferred_tool_schemas: &deferred,
            eager_tool_count: 0,
            deferred_tool_count: 0,
            activated_tool_count: 0,
            prompt_cache_key: None,
            history_for_api: &history,
            vision_bridge_available: false,
            reasoning_effort: Some("xhigh"),
            temperature: Some(0.2),
            max_tokens: 20_000,
            is_final_round: false,
            round: 0,
        };

        for model in [
            "claude-fable-5",
            "claude-mythos-5",
            "claude-sonnet-5",
            "claude-opus-5",
        ] {
            let (body, _, _) = build_anthropic_body("https://api.anthropic.com", model, &req);
            assert_eq!(
                body["thinking"],
                serde_json::json!({ "type": "adaptive", "block_binding": {"prefix_mismatch_behavior": "drop_block"} })
            );
            assert_eq!(body["output_config"]["effort"], "max");
            assert!(body.get("temperature").is_none());
            assert!(body["thinking"].get("budget_tokens").is_none());
        }

        let (opus_48, _, _) =
            build_anthropic_body("https://api.anthropic.com", "claude-opus-4-8", &req);
        assert_eq!(opus_48["thinking"]["type"], "enabled");
        assert_eq!(opus_48["thinking"]["budget_tokens"], 16_384);
        assert!(opus_48.get("temperature").is_none());

        for model in ["claude-haiku-4-5", "claude-sonnet-4-6"] {
            let (body, _, _) = build_anthropic_body("https://api.anthropic.com", model, &req);
            assert_eq!(body["thinking"]["type"], "enabled");
            assert_eq!(body["temperature"], 0.2);
        }

        let (compatible, _, _) =
            build_anthropic_body("https://compatible.example", "claude-sonnet-5", &req);
        assert_eq!(compatible["thinking"]["type"], "enabled");
        assert_eq!(compatible["temperature"], 0.2);
        assert!(compatible.get("output_config").is_none());
    }

    #[test]
    fn anthropic_request_golden_keeps_stable_prefix_before_dynamic_memory() {
        let loaded = vec![
            serde_json::json!({ "name": "tool_search", "input_schema": { "type": "object" } }),
            serde_json::json!({ "name": "read", "input_schema": { "type": "object" } }),
        ];
        let deferred = vec![serde_json::json!({
            "name": "browser",
            "input_schema": { "type": "object" }
        })];
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
            tool_schemas: &loaded,
            deferred_tool_schemas: &deferred,
            eager_tool_count: 2,
            deferred_tool_count: 1,
            activated_tool_count: 0,
            prompt_cache_key: Some("ignored-by-anthropic"),
            history_for_api: &history,
            vision_bridge_available: false,
            reasoning_effort: None,
            temperature: None,
            max_tokens: 100,
            is_final_round: false,
            round: 0,
        };
        let (body, tools, native_deferred) =
            build_anthropic_body("https://api.anthropic.com", "claude-sonnet-4-5", &req);
        let count_body =
            build_anthropic_count_body("https://api.anthropic.com", "claude-sonnet-4-5", &req);
        for field in ["model", "system", "messages", "tools", "thinking"] {
            assert_eq!(count_body.get(field), body.get(field));
        }
        assert!(count_body.get("stream").is_none());
        assert!(count_body.get("max_tokens").is_none());
        assert!(native_deferred);
        assert_eq!(
            body["system"],
            serde_json::json!([
                { "type": "text", "text": "stable", "cache_control": { "type": "ephemeral" } },
                { "type": "text", "text": "run" },
                { "type": "text", "text": "coding" },
            ])
        );
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "question");
        assert!(body["messages"][0]["content"][1]["text"]
            .as_str()
            .is_some_and(|text| text.contains("source=\"related_notes\"")));
        assert!(body["messages"][0]["content"][1]["text"]
            .as_str()
            .is_some_and(|text| text.contains("source=\"legacy_memory\"")));
        assert_eq!(tools[0]["name"], "read");
        assert_eq!(tools[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(tools[1]["name"], "browser");
        assert_eq!(tools[1]["defer_loading"], true);
        assert!(tools[1].get("cache_control").is_none());
        assert_eq!(tools[2]["type"], "tool_search_tool_bm25_20251119");
        let adapter = AnthropicStreamingAdapter {
            api_key: "sk-ant-test-must-stay-in-header",
            base_url: "https://api.anthropic.com",
            model: "claude-sonnet-4-5",
            workspace_id: None,
        };
        let accounting = adapter.token_count_input_for(&req);
        let counted_stable: serde_json::Value =
            serde_json::from_str(&accounting.stable_prompt).unwrap();
        let counted_dynamic: Vec<serde_json::Value> =
            serde_json::from_str(&accounting.dynamic_prompt).unwrap();
        assert_eq!(counted_stable["system"][0], body["system"][0]);
        assert_eq!(
            counted_dynamic[0]["content"].as_array().unwrap(),
            &body["system"].as_array().unwrap()[1..]
        );
        assert_eq!(
            counted_dynamic[1]["content"][0],
            body["messages"][0]["content"][1]
        );
        assert_eq!(accounting.history, history);
        assert_eq!(body["max_tokens"], req.max_tokens);
        assert_eq!(adapter.token_count_tool_schemas(&req), tools);
        let prepared = adapter.prepare_round_request(&req).unwrap();
        let prepared_body = prepared.body();
        assert_eq!(prepared_body.as_ref(), serde_json::to_vec(&body).unwrap());
        assert!(!String::from_utf8_lossy(prepared_body.as_ref())
            .contains("sk-ant-test-must-stay-in-header"));
        assert!(!prepared.identity.body_keyed_fingerprint.contains("sk-ant"));
        let prepared_json: serde_json::Value =
            serde_json::from_slice(prepared_body.as_ref()).unwrap();
        for transport_field in ["authorization", "api_key", "access_token", "account_id"] {
            assert!(prepared_json.get(transport_field).is_none());
        }
        req.is_final_round = true;
        assert!(adapter.token_count_tool_schemas(&req).is_empty());
    }

    #[test]
    fn native_reference_blocks_are_round_tripped_in_history() {
        let adapter = AnthropicStreamingAdapter {
            api_key: "",
            base_url: "https://api.anthropic.com",
            model: "claude-sonnet-4-5",
            workspace_id: None,
        };
        let outcome = RoundOutcome {
            text: "before after".to_string(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            provider_history_items: vec![
                serde_json::json!({ "type": "text", "text": "before" }),
                serde_json::json!({
                    "type": "tool_search_tool_result",
                    "tool_use_id": "srvtoolu_1",
                    "content": {
                        "type": "tool_search_tool_search_result",
                        "tool_references": [{ "type": "tool_reference", "tool_name": "browser" }]
                    }
                }),
                serde_json::json!({ "type": "text", "text": "after" }),
            ],
            usage: Default::default(),
            ttft_ms: None,
            stop_reason: Some("tool_use".to_string()),
        };
        let mut history = Vec::new();
        adapter.append_round_to_history(&mut history, 0, &outcome, &[]);
        assert_eq!(history[0]["content"][0]["text"], "before");
        assert_eq!(history[0]["content"][1]["type"], "tool_search_tool_result");
        assert_eq!(history[0]["content"][2]["text"], "after");
        assert_eq!(history[0]["content"].as_array().unwrap().len(), 3);
    }
}
