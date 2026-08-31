//! OpenAI Chat Completions API adapter implementing [`StreamingChatAdapter`].
//!
//! Owns body construction (multiple `system` messages for OpenAI's automatic
//! prefix caching), HTTP send, SSE event decoding (delta-based with
//! `tool_calls[]` index accumulation + `<think>` tag filtering), and history
//! persistence in Chat Completions' `tool_calls` + `role=tool` shape.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::FunctionCallItem;
use super::super::config::{apply_thinking_to_chat_body, build_api_url};
use super::super::events::{
    emit_text_delta, emit_thinking_delta, expand_openai_chat_image_markers_for_api,
    openai_chat_history_has_images, project_openai_chat_image_markers_for_token_count,
};
use super::super::streaming_adapter::{
    observe_before_send, observe_response_started, ExecutedTool, PreparedProviderRequest,
    PreparedRequestVariant, ProviderAccountingInput, ProviderDispatchObserver,
    ProviderDispatchUnknown, ProviderEndpointKind, ProviderReprepareReason, ProviderRequestShape,
    ReprepareRequired, RoundOutcome, RoundRequest, StreamingChatAdapter, VisionInputRejected,
};
use super::super::types::{AssistantAgent, ChatUsage, ProviderFormat, ThinkTagFilter};
use crate::provider::ThinkingStyle;
use crate::tool_defs::ToolProvider;

/// OpenAI-compatible backends differ on whether they accept
/// `prompt_cache_key`. Probe optimistically once, then remember an explicit
/// unsupported-parameter response for the rest of the process. This is only a
/// wire capability cache; it never changes the model-visible prompt.
static PROMPT_CACHE_KEY_UNSUPPORTED_BACKENDS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

fn prompt_cache_backend_key(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_ascii_lowercase()
}

fn prompt_cache_key_is_supported(base_url: &str) -> bool {
    let key = prompt_cache_backend_key(base_url);
    PROMPT_CACHE_KEY_UNSUPPORTED_BACKENDS
        .read()
        .map(|backends| !backends.contains(&key))
        .unwrap_or(true)
}

fn mark_prompt_cache_key_unsupported(base_url: &str) {
    if let Ok(mut backends) = PROMPT_CACHE_KEY_UNSUPPORTED_BACKENDS.write() {
        backends.insert(prompt_cache_backend_key(base_url));
    }
}

pub(crate) struct OpenAIChatStreamingAdapter<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
    pub thinking_style: &'a ThinkingStyle,
    pub provider_config: Option<&'a crate::provider::ProviderConfig>,
    /// Set once the backend rejects `image_url` content (400) at runtime, so
    /// later tool-loop rounds in the same turn skip images directly instead of
    /// re-paying a wasted 400 + retry each round.
    pub vision_runtime_disabled: Arc<AtomicBool>,
    /// Guards the user-facing "model can't see images" notice to once per turn.
    pub vision_notice_emitted: Arc<AtomicBool>,
    /// `prepare_history_for_api` may fold catalog-disabled image input before
    /// `chat_round` can inspect it. Preserve that content-free fact so the
    /// existing one-shot user notice is not lost at the freeze boundary.
    pub prepared_history_had_images: AtomicBool,
}

impl OpenAIChatStreamingAdapter<'_> {
    fn prepare_chat_variant(
        &self,
        req: &RoundRequest<'_>,
        thinking_disabled: bool,
        model_supports_vision: bool,
    ) -> Result<PreparedProviderRequest> {
        let thinking_style = if thinking_disabled {
            &ThinkingStyle::None
        } else {
            self.thinking_style
        };
        let (body, api_messages, tools_array) = build_chat_body(
            self.base_url,
            self.model,
            thinking_style,
            model_supports_vision,
            req,
        );
        let prompt_cache_key_included = body.get("prompt_cache_key").is_some();
        let proactive_vision_notice = !model_supports_vision
            && (self.prepared_history_had_images.load(Ordering::Relaxed)
                || openai_chat_history_has_images(req.history_for_api));
        let prepared = PreparedProviderRequest::from_json(
            ProviderEndpointKind::OpenAIChatCompletions,
            ProviderRequestShape::OpenAIChatCompletions,
            self.model,
            req.round,
            req.session_id,
            req.reasoning_effort,
            req.vision_bridge_available,
            PreparedRequestVariant::OpenAIChat {
                thinking_disabled,
                model_supports_vision,
                prompt_cache_key_included,
                proactive_vision_notice,
            },
            &body,
        )?;
        log_openai_chat_request(
            self.model,
            req,
            &api_messages,
            &tools_array,
            &body,
            prepared.identity.body_len as usize,
        );
        Ok(prepared)
    }
}

/// Emit the one-shot "images ignored, continuing without vision" notice.
/// Frontend renders it as a gray inline banner; IM ships it as a standalone
/// system message. Deduped via `vision_notice_emitted`.
fn emit_vision_auto_disabled(
    on_delta: &(impl Fn(&str) + Send + ?Sized),
    provider_config: Option<&crate::provider::ProviderConfig>,
    model: &str,
) {
    let provider_id = provider_config.map(|p| p.id.clone());
    let provider_name = provider_config
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Unknown Provider".to_string());
    on_delta(
        &json!({
            "type": "vision_auto_disabled",
            "provider_id": provider_id,
            "provider_name": provider_name,
            "model_id": model,
            "action": "configure_vision_bridge",
        })
        .to_string(),
    );
}

#[derive(Debug, Clone)]
struct ThinkingAutoDisable {
    payload: serde_json::Value,
}

fn build_chat_body(
    base_url: &str,
    model: &str,
    thinking_style: &ThinkingStyle,
    model_supports_vision: bool,
    req: &RoundRequest<'_>,
) -> (
    serde_json::Value,
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
) {
    let mut api_messages: Vec<Value> =
        vec![json!({ "role": "system", "content": req.system_prompt })];
    for suffix in super::super::streaming_adapter::dynamic_instruction_suffixes(req) {
        // The stable system message remains the exact prefix. A separate
        // dynamic system message is the compatibility form for chat endpoints
        // that do not consistently support the developer role.
        api_messages.push(json!({ "role": "system", "content": suffix }));
    }
    let expanded_history =
        expand_openai_chat_image_markers_for_api(req.history_for_api, model_supports_vision);
    api_messages.extend(expanded_history);
    if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
        api_messages.push(json!({ "role": "user", "content": content }));
    }

    let tools_array: Vec<Value> = req
        .tool_schemas
        .iter()
        .map(|t| json!({ "type": "function", "function": t }))
        .collect();

    let mut body = json!({
        "model": model,
        "messages": api_messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !req.is_final_round {
        body["tools"] = json!(tools_array);
    }
    apply_thinking_to_chat_body(
        &mut body,
        thinking_style,
        req.reasoning_effort,
        req.max_tokens,
    );
    apply_official_chat_effort(
        &mut body,
        base_url,
        model,
        thinking_style,
        req.reasoning_effort,
    );
    if let Some(temp) = req.temperature {
        body["temperature"] = json!(temp);
    }
    if prompt_cache_key_is_supported(base_url) {
        if let Some(key) = req.prompt_cache_key {
            body["prompt_cache_key"] = json!(key);
        }
    }

    (body, api_messages, tools_array)
}

/// Refine the conservative compatibility mapping only for verified first-party
/// endpoint/model pairs. Never change stored preferences or infer gateway support.
fn apply_official_chat_effort(
    body: &mut Value,
    base_url: &str,
    model: &str,
    thinking_style: &ThinkingStyle,
    effort: Option<&str>,
) {
    if !matches!(thinking_style, ThinkingStyle::Openai) {
        return;
    }
    let Some(effort) = effort else { return };
    if !matches!(
        effort,
        "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
    ) {
        return;
    }
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return;
    };
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return;
    }
    let mapped = match (url.host_str(), model, effort) {
        (Some("generativelanguage.googleapis.com"), "gemini-3.7-flash", "minimal") => "low",
        (Some("generativelanguage.googleapis.com"), "gemini-3.7-flash", "xhigh" | "max") => "high",
        (Some("api.x.ai"), "grok-4.6", "minimal") => "low",
        (Some("api.x.ai"), "grok-4.6", "xhigh" | "max") => "xhigh",
        (
            Some("api.openai.com"),
            "gpt-5.6" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna",
            "minimal",
        ) => "low",
        (
            Some("api.openai.com"),
            "gpt-5.6" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna",
            "xhigh" | "max",
        ) => effort,
        _ => return,
    };
    body["reasoning_effort"] = json!(mapped);
}

fn log_openai_chat_request(
    model: &str,
    req: &RoundRequest<'_>,
    api_messages: &[Value],
    tools_array: &[Value],
    body: &Value,
    body_size: usize,
) {
    super::super::token_manifest::log_round_manifest(
        "OpenAIChat",
        model,
        "chat_completions",
        req,
        body.get("tools")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        body_size,
        false,
    );
    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "agent",
            "agent::chat_openai_chat::request",
            &format!(
                "OpenAI Chat API request round {}: {} messages, {} tools, body {}B",
                req.round,
                api_messages.len(),
                tools_array.len(),
                body_size
            ),
            Some(
                json!({
                    "round": req.round,
                    "endpoint_kind": "openai_chat_completions",
                    "model": model,
                    "message_count": api_messages.len(),
                    "tool_count": tools_array.len(),
                    "body_size_bytes": body_size,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

fn log_openai_chat_response(
    resp: &reqwest::Response,
    request_start: std::time::Instant,
    round: u32,
) {
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
            "agent::chat_openai_chat::response",
            &format!(
                "OpenAI Chat API response: status={}, request_id={}, ttfb={}ms",
                status, request_id, ttfb_ms
            ),
            Some(
                json!({
                    "status": status,
                    "request_id": request_id,
                    "ttfb_ms": ttfb_ms,
                    "round": round,
                    "response_headers": response_headers,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

fn log_openai_chat_error(status: u16, error_text: &str, round: u32) {
    if let Some(logger) = crate::get_logger() {
        let error_fingerprint =
            crate::cache_routing::audit_fingerprint("openai-chat-error", error_text.as_bytes());
        logger.log(
            "error",
            "agent",
            "agent::chat_openai_chat::error",
            &format!("OpenAI Chat API error ({status})"),
            Some(
                json!({
                    "status": status,
                    "error_bytes": error_text.len(),
                    "error_fingerprint": error_fingerprint,
                    "round": round
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

fn is_unsupported_thinking_error(style: &ThinkingStyle, status: u16, error_text: &str) -> bool {
    if status != 400 || *style == ThinkingStyle::None {
        return false;
    }
    let lower = error_text.to_lowercase();
    let param = match style {
        ThinkingStyle::Openai => "reasoning_effort",
        ThinkingStyle::Anthropic | ThinkingStyle::Zai => "\"thinking\"",
        ThinkingStyle::Qwen => "enable_thinking",
        ThinkingStyle::None => return false,
    };
    let signal = [
        "unrecognized",
        "unsupported",
        "unknown",
        "invalid",
        "not support",
        "not supported",
    ];
    lower.contains(param) && signal.iter().any(|needle| lower.contains(needle))
}

fn persist_model_thinking_disabled(
    provider_config: &crate::provider::ProviderConfig,
    model_id: &str,
) -> Result<(), String> {
    let provider_id = provider_config.id.clone();
    let model_id = model_id.to_string();
    crate::config::mutate_config(("providers.update", "thinking-autofix"), |store| {
        let provider = store
            .providers
            .iter_mut()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider not found: {}", provider_id))?;
        let model = provider
            .models
            .iter_mut()
            .find(|m| m.id == model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;
        model.thinking_style = Some(ThinkingStyle::None);
        Ok(())
    })
    .map_err(|e| e.to_string())
}

fn maybe_auto_disable_thinking(
    provider_config: Option<&crate::provider::ProviderConfig>,
    model: &str,
    style: &ThinkingStyle,
    status: u16,
    error_text: &str,
) -> Option<ThinkingAutoDisable> {
    if !is_unsupported_thinking_error(style, status, error_text) {
        return None;
    }

    let (provider_id, provider_name) = if let Some(provider) = provider_config {
        let _ = persist_model_thinking_disabled(provider, model);
        (Some(provider.id.clone()), provider.name.clone())
    } else {
        (None, "Unknown Provider".to_string())
    };

    if let Some(logger) = crate::get_logger() {
        let error_fingerprint = crate::cache_routing::audit_fingerprint(
            "openai-chat-thinking-autofix",
            error_text.as_bytes(),
        );
        logger.log(
            "warn",
            "agent",
            "agent::chat_openai_chat::thinking_autofix",
            &format!(
                "Auto-disabled thinking for {} / {} after unsupported parameter error",
                provider_name, model
            ),
            Some(
                json!({
                    "provider_id": provider_id,
                    "provider_name": provider_name,
                    "model": model,
                    "status": status,
                    "error_bytes": error_text.len(),
                    "error_fingerprint": error_fingerprint,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Some(ThinkingAutoDisable {
        payload: json!({
            "type": "thinking_auto_disabled",
            "provider_id": provider_id,
            "provider_name": provider_name,
            "model_id": model,
        }),
    })
}

fn is_unsupported_image_url_error(status: u16, error_text: &str) -> bool {
    if status != 400 {
        return false;
    }
    let lower = error_text.to_lowercase();
    // OpenAI-compat backends that don't accept `image_url` tool content
    // surface this through the body deserializer; DeepSeek phrases it as
    // `unknown variant \`image_url\`, expected \`text\``. Other backends
    // may differ — match on the field name plus any rejection word.
    lower.contains("image_url")
        && (lower.contains("unknown variant")
            || lower.contains("invalid type")
            || lower.contains("invalid_type")
            || lower.contains("unsupported")
            || lower.contains("not supported"))
}

fn is_unsupported_prompt_cache_key_error(status: u16, error_text: &str) -> bool {
    if status != 400 {
        return false;
    }
    let lower = error_text.to_ascii_lowercase();
    lower.contains("prompt_cache_key")
        && [
            "unrecognized",
            "unsupported",
            "unknown",
            "not support",
            "not supported",
            "unexpected",
            "extra input",
            "extra field",
        ]
        .iter()
        .any(|signal| lower.contains(signal))
}

fn log_vision_runtime_disabled(
    provider_config: Option<&crate::provider::ProviderConfig>,
    model: &str,
    status: u16,
    error_text: &str,
) {
    let Some(logger) = crate::get_logger() else {
        return;
    };
    let (provider_id, provider_name) = provider_config
        .map(|p| (Some(p.id.clone()), p.name.clone()))
        .unwrap_or((None, "Unknown Provider".to_string()));
    let error_fingerprint = crate::cache_routing::audit_fingerprint(
        "openai-chat-vision-autofix",
        error_text.as_bytes(),
    );
    logger.log(
        "warn",
        "agent",
        "agent::chat_openai_chat::vision_autofix",
        &format!(
            "Detected text-only image capability for {} / {} after image_url rejection",
            provider_name, model
        ),
        Some(
            json!({
                "provider_id": provider_id,
                "provider_name": provider_name,
                "model": model,
                "status": status,
                "error_bytes": error_text.len(),
                "error_fingerprint": error_fingerprint,
            })
            .to_string(),
        ),
        None,
        None,
    );
}

async fn send_chat_request(
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    body: Arc<[u8]>,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<reqwest::Response>> {
    let mut http_req = client
        .post(api_url)
        .header("Content-Type", "application/json");
    if !api_key.is_empty() {
        http_req = http_req.header("Authorization", format!("Bearer {}", api_key));
    }
    let Some(resp) = super::cancel::send_with_cancel(http_req.body(body.to_vec()), cancel)
        .await
        .map_err(|e| ProviderDispatchUnknown(e.to_string()))?
    else {
        return Ok(None);
    };
    Ok(Some(resp))
}

#[async_trait]
impl<'a> StreamingChatAdapter for OpenAIChatStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::OpenAIChat
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::OpenAI
    }

    fn vision_runtime_disabled(&self) -> bool {
        self.vision_runtime_disabled.load(Ordering::Relaxed)
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_chat(history);
    }

    fn token_count_tool_schemas_for(
        &self,
        tool_schemas: &[Value],
        _deferred_tool_schemas: &[Value],
        _eager_tool_count: usize,
        is_final_round: bool,
    ) -> Vec<Value> {
        if is_final_round {
            return Vec::new();
        }
        tool_schemas
            .iter()
            .map(|tool| json!({ "type": "function", "function": tool }))
            .collect()
    }

    fn token_count_history_for(&self, history: &[Value]) -> Vec<Value> {
        // Keep the capacity proof on the exact same modality branch as
        // `chat_round`: catalog-declared text-only models must count the
        // placeholder form even before a runtime image rejection occurs.
        let model_supports_vision = self
            .provider_config
            .map(|pc| pc.model_supports_vision(self.model))
            .unwrap_or(true)
            && !self.vision_runtime_disabled();
        project_openai_chat_image_markers_for_token_count(history, model_supports_vision)
    }

    fn prepare_history_for_api(&self, history: &[Value]) -> Vec<Value> {
        self.prepared_history_had_images
            .store(openai_chat_history_has_images(history), Ordering::Relaxed);
        let model_supports_vision = self
            .provider_config
            .map(|pc| pc.model_supports_vision(self.model))
            .unwrap_or(true)
            && !self.vision_runtime_disabled();
        expand_openai_chat_image_markers_for_api(history, model_supports_vision)
    }

    fn token_count_input_for(&self, req: &RoundRequest<'_>) -> ProviderAccountingInput {
        let stable_message = json!({ "role": "system", "content": req.system_prompt });
        let mut dynamic_messages: Vec<Value> =
            super::super::streaming_adapter::dynamic_instruction_suffixes(req)
                .into_iter()
                .map(|content| json!({ "role": "system", "content": content }))
                .collect();
        if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
            dynamic_messages.push(json!({ "role": "user", "content": content }));
        }
        ProviderAccountingInput {
            stable_prompt: serde_json::to_string(&stable_message).unwrap_or_default(),
            dynamic_prompt: serde_json::to_string(&dynamic_messages).unwrap_or_default(),
            history: self.token_count_history_for(req.history_for_api),
        }
    }

    fn prepare_round_request(&self, req: &RoundRequest<'_>) -> Result<PreparedProviderRequest> {
        let model_supports_vision = self
            .provider_config
            .map(|pc| pc.model_supports_vision(self.model))
            .unwrap_or(true)
            && !self.vision_runtime_disabled();
        self.prepare_chat_variant(req, false, model_supports_vision)
    }

    fn reprepare_round_request(
        &self,
        req: &RoundRequest<'_>,
        previous: &PreparedProviderRequest,
        reason: ProviderReprepareReason,
    ) -> Result<PreparedProviderRequest> {
        let PreparedRequestVariant::OpenAIChat {
            thinking_disabled,
            model_supports_vision,
            ..
        } = previous.variant
        else {
            anyhow::bail!("OpenAI Chat received an incompatible prepared request")
        };
        self.prepare_chat_variant(
            req,
            thinking_disabled || reason == ProviderReprepareReason::Thinking,
            model_supports_vision && reason != ProviderReprepareReason::Vision,
        )
    }

    async fn dispatch_prepared(
        &self,
        client: &reqwest::Client,
        prepared: &PreparedProviderRequest,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
        observer: &dyn ProviderDispatchObserver,
    ) -> Result<RoundOutcome> {
        let PreparedRequestVariant::OpenAIChat {
            thinking_disabled,
            model_supports_vision,
            prompt_cache_key_included,
            proactive_vision_notice,
        } = prepared.variant
        else {
            anyhow::bail!("OpenAI Chat received an incompatible prepared request")
        };
        if proactive_vision_notice && !self.vision_notice_emitted.swap(true, Ordering::Relaxed) {
            emit_vision_auto_disabled(on_delta, self.provider_config, self.model);
        }

        let api_url = build_api_url(self.base_url, "/v1/chat/completions");
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }
        observe_before_send(observer, prepared).await?;
        let request_start = std::time::Instant::now();
        let Some(resp) =
            send_chat_request(client, &api_url, self.api_key, prepared.body(), cancel).await?
        else {
            return Err(ProviderDispatchUnknown(
                "cancelled after dispatch claim and before response headers".to_string(),
            )
            .into());
        };
        observe_response_started(observer, prepared, 1, &resp).await?;
        log_openai_chat_response(&resp, request_start, prepared.identity.round);

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
            log_openai_chat_error(status, &error_text, prepared.identity.round);

            if prompt_cache_key_included
                && is_unsupported_prompt_cache_key_error(status, &error_text)
            {
                mark_prompt_cache_key_unsupported(self.base_url);
                return Err(ReprepareRequired {
                    reason: ProviderReprepareReason::PromptCacheKey,
                }
                .into());
            }
            if !thinking_disabled {
                let active_style = self.thinking_style;
                if let Some(autofix) = maybe_auto_disable_thinking(
                    self.provider_config,
                    self.model,
                    active_style,
                    status,
                    &error_text,
                ) {
                    on_delta(&autofix.payload.to_string());
                    return Err(ReprepareRequired {
                        reason: ProviderReprepareReason::Thinking,
                    }
                    .into());
                }
            }
            if model_supports_vision && is_unsupported_image_url_error(status, &error_text) {
                log_vision_runtime_disabled(self.provider_config, self.model, status, &error_text);
                self.vision_runtime_disabled.store(true, Ordering::Relaxed);
                if prepared.vision_bridge_available {
                    return Err(VisionInputRejected.into());
                }
                if !self.vision_notice_emitted.swap(true, Ordering::Relaxed) {
                    emit_vision_auto_disabled(on_delta, self.provider_config, self.model);
                }
                return Err(ReprepareRequired {
                    reason: ProviderReprepareReason::Vision,
                }
                .into());
            }
            return Err(crate::failover::ProviderApiError::from_http_response(
                "OpenAI Chat",
                status,
                &error_text,
            )
            .with_retry_after_header(retry_after.as_deref())
            .into());
        }

        let (text, tool_calls, mut usage, thinking_text, ttft_ms) = parse_chat_completions_sse(
            resp,
            request_start,
            prepared.reasoning_effort.as_deref(),
            cancel,
            on_delta,
        )
        .await?;
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }

        if let Some(logger) = crate::get_logger() {
            let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            if !tool_names.is_empty() {
                logger.log(
                    "info",
                    "agent",
                    "agent::chat_openai_chat::tool_loop",
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
            "OpenAIChat",
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
            provider_history_items: Vec::new(),
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
        // Build assistant message: {role, content?, reasoning_content?, tool_calls}
        let tc_json: Vec<Value> = outcome
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.call_id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }
                })
            })
            .collect();

        let mut assistant_msg = json!({ "role": "assistant" });
        if !outcome.text.is_empty() {
            assistant_msg["content"] = json!(outcome.text);
        }
        if !outcome.thinking.is_empty() {
            assistant_msg["reasoning_content"] = json!(outcome.thinking);
        }
        assistant_msg["tool_calls"] = json!(tc_json);
        crate::context_compact::push_and_stamp(history, assistant_msg, round);

        // One {role: tool, tool_call_id, content} message per executed tool.
        for et in executed {
            crate::context_compact::push_and_stamp(
                history,
                json!({
                    "role": "tool",
                    "tool_call_id": et.call_id,
                    "content": et.clean_result,
                }),
                round,
            );
        }
    }

    fn append_final_assistant(
        &self,
        history: &mut Vec<Value>,
        final_text: &str,
        last_thinking: &str,
    ) {
        if !final_text.is_empty() {
            let mut final_msg = json!({ "role": "assistant", "content": final_text });
            if !last_thinking.is_empty() {
                final_msg["reasoning_content"] = json!(last_thinking);
            }
            history.push(final_msg);
        }
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        outcome.tool_calls.is_empty()
    }
}

/// Parse OpenAI Chat Completions SSE stream.
/// Returns `(collected_text, tool_calls, usage, thinking, ttft_ms)`.
fn take_next_chat_sse_event_block(buffer: &mut Vec<u8>) -> Result<Option<String>> {
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
        .map_err(|_| anyhow::anyhow!("OpenAI Chat SSE event contained invalid UTF-8"))?;
    Ok(Some(block))
}

fn validate_chat_sse_eof_tail(cancelled: bool, buffer: &[u8]) -> Result<()> {
    if cancelled || buffer.is_empty() {
        return Ok(());
    }
    std::str::from_utf8(buffer)
        .map_err(|_| anyhow::anyhow!("OpenAI Chat SSE ended with invalid UTF-8"))?;
    anyhow::bail!("OpenAI Chat SSE ended with an incomplete event")
}

fn finalize_chat_completion_stream(
    cancelled: bool,
    saw_done: bool,
    finish_reason: Option<&str>,
    mut pending_calls: std::collections::HashMap<usize, FunctionCallItem>,
) -> Result<Vec<FunctionCallItem>> {
    if cancelled {
        return Ok(Vec::new());
    }
    if !saw_done {
        anyhow::bail!("OpenAI Chat SSE ended before [DONE]")
    }
    let finish_reason = finish_reason
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| anyhow::anyhow!("OpenAI Chat SSE ended without a finish_reason"))?;
    if !matches!(
        finish_reason,
        "stop" | "length" | "tool_calls" | "content_filter" | "function_call"
    ) {
        anyhow::bail!("OpenAI Chat SSE ended with an unknown finish_reason={finish_reason}")
    }
    let tool_finish = matches!(finish_reason, "tool_calls" | "function_call");
    if !pending_calls.is_empty() && !tool_finish {
        anyhow::bail!(
            "OpenAI Chat SSE ended with pending tool calls but finish_reason={finish_reason}"
        )
    }
    if pending_calls.is_empty() && tool_finish {
        anyhow::bail!("OpenAI Chat SSE declared tool completion without a tool call")
    }

    let mut sorted_keys: Vec<usize> = pending_calls.keys().copied().collect();
    sorted_keys.sort_unstable();
    let mut tool_calls = Vec::with_capacity(sorted_keys.len());
    for key in sorted_keys {
        let call = pending_calls
            .remove(&key)
            .expect("key collected from pending tool calls");
        if call.call_id.is_empty() || call.name.is_empty() {
            anyhow::bail!("OpenAI Chat SSE completed with an invalid tool call")
        }
        serde_json::from_str::<Value>(&call.arguments).map_err(|err| {
            anyhow::anyhow!(
                "OpenAI Chat SSE completed with invalid tool arguments for {}: {}",
                call.name,
                err
            )
        })?;
        tool_calls.push(call);
    }
    Ok(tool_calls)
}

fn decode_chat_completion_sse_data(data: &str) -> Result<Value> {
    serde_json::from_str::<Value>(data)
        .map_err(|err| anyhow::anyhow!("OpenAI Chat SSE event could not be decoded: {err}"))
}

pub(crate) async fn parse_chat_completions_sse(
    resp: reqwest::Response,
    request_start: std::time::Instant,
    reasoning_effort: Option<&str>,
    cancel: &Arc<AtomicBool>,
    on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
) -> Result<(
    String,
    Vec<FunctionCallItem>,
    ChatUsage,
    String,
    Option<u64>,
)> {
    let mut collected_text = String::new();
    let mut collected_thinking = String::new();
    let mut pending_calls: std::collections::HashMap<usize, FunctionCallItem> =
        std::collections::HashMap::new();
    let mut usage = ChatUsage::default();
    let mut think_filter = ThinkTagFilter::new();
    let mut first_token_time: Option<u64> = None;

    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut saw_done = false;
    let mut finish_reason: Option<String> = None;

    'chat_stream: while let Some(chunk) =
        super::cancel::next_chunk_or_cancel(&mut stream, cancel).await
    {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) if cancel.load(Ordering::SeqCst) => break,
            Err(err) => return Err(err.into()),
        };
        buffer.extend_from_slice(&chunk);

        while let Some(event_block) = take_next_chat_sse_event_block(&mut buffer)? {
            for line in event_block.lines() {
                let data = if let Some(d) = line.strip_prefix("data:") {
                    d.trim()
                } else {
                    continue;
                };

                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    saw_done = true;
                    continue;
                }
                if saw_done {
                    anyhow::bail!("OpenAI Chat SSE emitted data after [DONE]")
                }

                let chunk = decode_chat_completion_sse_data(data)?;
                if let Some(error) = chunk.get("error") {
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("OpenAI Chat streaming error");
                    return Err(crate::failover::ProviderApiError::from_stream_event(
                        "OpenAI Chat",
                        error.get("code").and_then(Value::as_str),
                        error.get("type").and_then(Value::as_str),
                        Some(message),
                        message.to_string(),
                    )
                    .into());
                }
                // Parse usage from stream (when stream_options.include_usage is set).
                if let Some(u) = chunk.get("usage") {
                    if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                        usage.input_tokens = pt;
                        usage.input_coverage = crate::token_accounting::UsageCoverage::Complete;
                    }
                    if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                        usage.output_tokens = ct;
                        usage.output_coverage = crate::token_accounting::UsageCoverage::Complete;
                    }
                    // Anthropic-style at top level (some gateways forward).
                    if let Some(cr) = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
                        usage.cache_read_input_tokens = cr;
                    }
                    if let Some(cc) = u
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64())
                    {
                        usage.cache_creation_input_tokens = cc;
                    }
                    // Fallback: OpenAI prompt_tokens_details.cached_tokens or top-level cached_tokens.
                    if usage.cache_read_input_tokens == 0 {
                        usage.cache_read_input_tokens = u
                            .get("prompt_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|v| v.as_u64())
                            .or_else(|| u.get("cached_tokens").and_then(|v| v.as_u64()))
                            .unwrap_or(0);
                    }
                    if usage.cache_creation_input_tokens == 0 {
                        usage.cache_creation_input_tokens = u
                            .get("prompt_tokens_details")
                            .and_then(|d| d.get("cache_write_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    }
                }
                if let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) {
                    for choice in choices {
                        if let Some(reason) = choice.get("finish_reason") {
                            if !reason.is_null() {
                                let reason = reason
                                    .as_str()
                                    .filter(|value| !value.is_empty())
                                    .ok_or_else(|| {
                                        anyhow::anyhow!(
                                            "OpenAI Chat SSE emitted an invalid finish_reason"
                                        )
                                    })?;
                                if finish_reason
                                    .as_deref()
                                    .is_some_and(|existing| existing != reason)
                                {
                                    anyhow::bail!(
                                        "OpenAI Chat SSE emitted conflicting finish_reason values"
                                    )
                                }
                                finish_reason = Some(reason.to_string());
                            }
                        }
                        let delta = match choice.get("delta") {
                            Some(d) => d,
                            None => continue,
                        };

                        // Reasoning/thinking content (DeepSeek, OpenAI o-series, etc.)
                        if let Some(reasoning) =
                            delta.get("reasoning_content").and_then(|c| c.as_str())
                        {
                            if !reasoning.is_empty() {
                                if first_token_time.is_none() {
                                    first_token_time =
                                        Some(request_start.elapsed().as_millis() as u64);
                                }
                                emit_thinking_delta(&on_delta, reasoning);
                                collected_thinking.push_str(reasoning);
                            }
                        }

                        // Text content — filter <think>...</think> tags. Qwen models embed
                        // thinking via <think> tags. With effort=none, discard entirely.
                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                            let (text_part, think_part) = think_filter.process(content);
                            if !think_part.is_empty() && reasoning_effort != Some("none") {
                                emit_thinking_delta(&on_delta, &think_part);
                                collected_thinking.push_str(&think_part);
                            }
                            if !text_part.is_empty() {
                                if first_token_time.is_none() {
                                    first_token_time =
                                        Some(request_start.elapsed().as_millis() as u64);
                                }
                                emit_text_delta(&on_delta, &text_part);
                                collected_text.push_str(&text_part);
                            }
                        }

                        // Tool calls — accumulated by index (parallel calls supported).
                        if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                            for tc_delta in tcs {
                                let idx =
                                    tc_delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                        as usize;

                                if let Some(func) = tc_delta.get("function") {
                                    let entry = pending_calls.entry(idx).or_insert_with(|| {
                                        FunctionCallItem {
                                            call_id: tc_delta
                                                .get("id")
                                                .and_then(|i| i.as_str())
                                                .unwrap_or("")
                                                .to_string(),
                                            name: String::new(),
                                            arguments: String::new(),
                                        }
                                    });
                                    if let Some(id) = tc_delta.get("id").and_then(|i| i.as_str()) {
                                        if !id.is_empty() {
                                            entry.call_id = id.to_string();
                                        }
                                    }
                                    if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                        entry.name.push_str(name);
                                    }
                                    if let Some(args) =
                                        func.get("arguments").and_then(|a| a.as_str())
                                    {
                                        entry.arguments.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if saw_done {
                break 'chat_stream;
            }
        }
    }

    let cancelled = cancel.load(Ordering::SeqCst);
    validate_chat_sse_eof_tail(cancelled, &buffer)?;
    let tool_calls = finalize_chat_completion_stream(
        cancelled,
        saw_done,
        finish_reason.as_deref(),
        pending_calls,
    )?;

    if let Some(logger) = crate::get_logger() {
        let tool_names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        logger.log(
            "debug",
            "agent",
            "agent::parse_chat_completions_sse::done",
            &format!(
                "OpenAI Chat SSE done: {}chars text, {} tool_calls",
                collected_text.len(),
                tool_calls.len()
            ),
            Some(
                json!({
                    "text_length": collected_text.len(),
                    "tool_calls": tool_names,
                    "tool_call_count": tool_calls.len(),
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
        usage,
        collected_thinking,
        first_token_time,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_official_chat_effort, build_chat_body, decode_chat_completion_sse_data,
        emit_vision_auto_disabled, finalize_chat_completion_stream, is_unsupported_image_url_error,
        is_unsupported_prompt_cache_key_error, mark_prompt_cache_key_unsupported,
        take_next_chat_sse_event_block, validate_chat_sse_eof_tail, OpenAIChatStreamingAdapter,
    };
    use crate::agent::api_types::FunctionCallItem;
    use crate::agent::streaming_adapter::{RoundRequest, StreamingChatAdapter};
    use crate::provider::ThinkingStyle;
    use std::collections::HashMap;
    use std::sync::{atomic::AtomicBool, Arc};

    #[test]
    fn official_chat_effort_is_scoped_to_endpoint_model_and_style() {
        for (base_url, model, effort, expected) in [
            (
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "gemini-3.7-flash",
                "minimal",
                "low",
            ),
            ("https://api.x.ai/v1", "grok-4.6", "xhigh", "xhigh"),
            ("https://api.x.ai/v1", "grok-4.6", "max", "xhigh"),
            ("https://api.openai.com/v1", "gpt-5.6-sol", "max", "max"),
            ("https://api.openai.com", "gpt-5.6-terra", "xhigh", "xhigh"),
            ("https://gateway.example/v1", "grok-4.6", "xhigh", "high"),
            ("https://api.x.ai.example/v1", "grok-4.6", "xhigh", "high"),
            ("http://api.x.ai/v1", "grok-4.6", "xhigh", "high"),
            ("https://api.x.ai:444/v1", "grok-4.6", "xhigh", "high"),
            ("https://api.x.ai/v1", "grok-4", "xhigh", "high"),
            ("https://api.openai.com/v1", "custom-gpt-5.6", "max", "high"),
        ] {
            let mut body = serde_json::json!({});
            crate::agent::config::apply_thinking_to_chat_body(
                &mut body,
                &ThinkingStyle::Openai,
                Some(effort),
                100,
            );
            apply_official_chat_effort(
                &mut body,
                base_url,
                model,
                &ThinkingStyle::Openai,
                Some(effort),
            );
            assert_eq!(
                body["reasoning_effort"], expected,
                "{base_url} {model} {effort}"
            );
        }
        for (style, effort) in [
            (ThinkingStyle::None, Some("max")),
            (ThinkingStyle::Openai, None),
            (ThinkingStyle::Openai, Some("none")),
        ] {
            let mut body = serde_json::json!({});
            apply_official_chat_effort(
                &mut body,
                "https://api.openai.com",
                "gpt-5.6-sol",
                &style,
                effort,
            );
            assert!(body.get("reasoning_effort").is_none());
        }
    }

    #[test]
    fn chat_stream_requires_done_and_finish_reason() {
        assert!(
            finalize_chat_completion_stream(false, false, Some("stop"), HashMap::new()).is_err()
        );
        assert!(finalize_chat_completion_stream(false, true, None, HashMap::new()).is_err());
        assert!(finalize_chat_completion_stream(
            false,
            true,
            Some("non_standard_terminal"),
            HashMap::new()
        )
        .is_err());
        assert!(finalize_chat_completion_stream(false, true, Some("stop"), HashMap::new()).is_ok());
        assert!(finalize_chat_completion_stream(true, false, None, HashMap::new()).is_ok());
    }

    #[test]
    fn chat_stream_only_promotes_complete_tool_calls() {
        let mut pending = HashMap::new();
        pending.insert(
            0,
            FunctionCallItem {
                call_id: "call_1".into(),
                name: "read".into(),
                arguments: r#"{"path":"README.md"}"#.into(),
            },
        );
        assert!(
            finalize_chat_completion_stream(false, true, Some("stop"), pending.clone()).is_err()
        );
        let completed =
            finalize_chat_completion_stream(false, true, Some("tool_calls"), pending).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].call_id, "call_1");
    }

    #[test]
    fn chat_sse_framing_accepts_crlf_done_block() {
        let mut buffer = b"data: [DONE]\r\n\r\nrest".to_vec();
        assert_eq!(
            take_next_chat_sse_event_block(&mut buffer)
                .unwrap()
                .as_deref(),
            Some("data: [DONE]")
        );
        assert_eq!(buffer, b"rest");
        assert!(decode_chat_completion_sse_data(r#"{"choices":[]}"#).is_ok());
        assert!(decode_chat_completion_sse_data("{").is_err());
    }

    #[test]
    fn chat_sse_framing_preserves_tool_json_split_inside_emoji_scalar() {
        let payload = serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": {
                            "name": "write",
                            "arguments": "{\"text\":\"🙂\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let wire = format!("data: {payload}\n\n").into_bytes();
        let scalar_start = wire
            .windows("🙂".len())
            .position(|window| window == "🙂".as_bytes())
            .expect("emoji scalar in fixture");
        let split = scalar_start + 2;
        let mut buffer = wire[..split].to_vec();
        assert!(take_next_chat_sse_event_block(&mut buffer)
            .unwrap()
            .is_none());
        buffer.extend_from_slice(&wire[split..]);
        let block = take_next_chat_sse_event_block(&mut buffer)
            .unwrap()
            .expect("complete SSE frame");
        let data = block
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .expect("data line")
            .trim();
        let decoded = decode_chat_completion_sse_data(data).expect("valid Unicode JSON event");
        assert_eq!(
            decoded["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"text\":\"🙂\"}"
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn chat_sse_eof_and_invalid_utf8_fail_closed() {
        assert!(validate_chat_sse_eof_tail(false, b"data: partial").is_err());
        assert!(validate_chat_sse_eof_tail(false, &[0xff]).is_err());
        assert!(validate_chat_sse_eof_tail(false, b"").is_ok());
        let mut invalid_frame = b"data: ".to_vec();
        invalid_frame.push(0xff);
        invalid_frame.extend_from_slice(b"\n\n");
        assert!(take_next_chat_sse_event_block(&mut invalid_frame).is_err());
    }

    #[test]
    fn openai_chat_request_golden_appends_dynamic_blocks_after_stable_system() {
        let tools = vec![serde_json::json!({
            "name": "read",
            "description": "Read a file",
            "parameters": { "type": "object" }
        })];
        let deferred = vec![serde_json::json!({ "name": "browser" })];
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
            eager_tool_count: 1,
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
        let thinking_style = ThinkingStyle::None;
        let (body, messages, request_tools) = build_chat_body(
            "https://api.openai.com",
            "gpt-5.4",
            &thinking_style,
            false,
            &req,
        );
        assert_eq!(
            messages,
            vec![
                serde_json::json!({ "role": "system", "content": "stable" }),
                serde_json::json!({ "role": "system", "content": "run" }),
                serde_json::json!({ "role": "system", "content": "coding" }),
                serde_json::json!({ "role": "user", "content": "question" }),
                serde_json::json!({
                    "role": "user",
                    "content": crate::agent::streaming_adapter::render_dynamic_data_envelope(&req)
                        .expect("data envelope")
                }),
            ]
        );
        assert_eq!(body["prompt_cache_key"], "stable-key");
        assert_eq!(request_tools.len(), 1);
        assert_eq!(body["tools"][0]["function"]["name"], "read");
        assert!(body.to_string().find("browser").is_none());
        let adapter = OpenAIChatStreamingAdapter {
            api_key: "sk-test-must-stay-in-header",
            base_url: "https://api.openai.com",
            model: "gpt-5.4",
            thinking_style: &thinking_style,
            provider_config: None,
            vision_runtime_disabled: Arc::new(AtomicBool::new(false)),
            vision_notice_emitted: Arc::new(AtomicBool::new(false)),
            prepared_history_had_images: AtomicBool::new(false),
        };
        let accounting = adapter.token_count_input_for(&req);
        let counted_stable: serde_json::Value =
            serde_json::from_str(&accounting.stable_prompt).unwrap();
        let counted_dynamic: Vec<serde_json::Value> =
            serde_json::from_str(&accounting.dynamic_prompt).unwrap();
        assert_eq!(counted_stable, messages[0]);
        assert_eq!(
            counted_dynamic,
            vec![
                messages[1].clone(),
                messages[2].clone(),
                messages[4].clone()
            ]
        );
        assert_eq!(accounting.history, history);
        assert_eq!(adapter.token_count_tool_schemas(&req), request_tools);
        let prepared = adapter.prepare_round_request(&req).unwrap();
        let prepared_body = prepared.body();
        assert_eq!(prepared_body.as_ref(), serde_json::to_vec(&body).unwrap());
        assert!(!String::from_utf8_lossy(prepared_body.as_ref())
            .contains("sk-test-must-stay-in-header"));
        assert!(!prepared.identity.body_keyed_fingerprint.contains("sk-test"));
        let prepared_json: serde_json::Value =
            serde_json::from_slice(prepared_body.as_ref()).unwrap();
        for transport_field in ["authorization", "api_key", "access_token", "account_id"] {
            assert!(prepared_json.get(transport_field).is_none());
        }
        req.is_final_round = true;
        assert!(adapter.token_count_tool_schemas(&req).is_empty());
    }

    #[test]
    fn compatible_backend_uses_cache_key_until_explicitly_rejected() {
        let tools = Vec::new();
        let history = Vec::new();
        let req = RoundRequest {
            session_id: None,
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
            deferred_tool_schemas: &tools,
            eager_tool_count: 0,
            deferred_tool_count: 0,
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
        let base_url = "https://cache-key-test.invalid/v1";
        let (initial, _, _) = build_chat_body(
            base_url,
            "compatible-model",
            &ThinkingStyle::None,
            false,
            &req,
        );
        assert_eq!(initial["prompt_cache_key"], "stable-key");

        mark_prompt_cache_key_unsupported(base_url);
        let (after_rejection, _, _) = build_chat_body(
            base_url,
            "compatible-model",
            &ThinkingStyle::None,
            false,
            &req,
        );
        assert!(after_rejection.get("prompt_cache_key").is_none());
    }

    #[test]
    fn ignored_image_notice_carries_vision_bridge_guidance_action() {
        let payload = std::sync::Mutex::new(None);
        emit_vision_auto_disabled(
            &|event| {
                *payload.lock().expect("payload lock") =
                    serde_json::from_str::<serde_json::Value>(event).ok();
            },
            None,
            "text-only-model",
        );
        assert_eq!(
            payload
                .into_inner()
                .expect("payload lock")
                .and_then(|value| value.get("action").cloned())
                .and_then(|value| value.as_str().map(str::to_string))
                .as_deref(),
            Some("configure_vision_bridge")
        );
    }

    #[test]
    fn detects_only_explicit_prompt_cache_key_capability_errors() {
        assert!(is_unsupported_prompt_cache_key_error(
            400,
            r#"{"error":{"message":"Unknown parameter: prompt_cache_key"}}"#,
        ));
        assert!(is_unsupported_prompt_cache_key_error(
            400,
            r#"{"detail":"Extra inputs are not permitted: prompt_cache_key"}"#,
        ));
        assert!(!is_unsupported_prompt_cache_key_error(
            429,
            "prompt_cache_key temporarily unavailable",
        ));
        assert!(!is_unsupported_prompt_cache_key_error(
            400,
            "invalid tools schema",
        ));
    }

    #[test]
    fn detects_deepseek_unknown_variant_rejection() {
        // Exact phrasing observed from DeepSeek v4-flash; this is the
        // failure mode that motivated the retry path.
        let body = r#"{"error":{"message":"Failed to deserialize the JSON body into the target type: messages[15]: unknown variant `image_url`, expected `text` at line 1 column 1906020","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}"#;
        assert!(is_unsupported_image_url_error(400, body));
    }

    #[test]
    fn detects_invalid_type_phrasing() {
        let body = r#"{"error":{"message":"invalid_type at messages[3].content: image_url is not supported"}}"#;
        assert!(is_unsupported_image_url_error(400, body));
    }

    #[test]
    fn ignores_non_400_status() {
        let body = r#"{"error":{"message":"unknown variant `image_url`"}}"#;
        assert!(!is_unsupported_image_url_error(500, body));
        assert!(!is_unsupported_image_url_error(429, body));
    }

    #[test]
    fn ignores_400_without_image_url_signal() {
        // 400 from a different cause (e.g. bad tool schema) must not
        // trigger the vision-disable retry — that would hide the real
        // error and waste an HTTP round-trip.
        let body = r#"{"error":{"message":"missing required field `tools[0].function.name`"}}"#;
        assert!(!is_unsupported_image_url_error(400, body));
    }

    #[test]
    fn ignores_image_url_appearance_without_rejection_words() {
        // image_url merely appearing in an error (e.g. content quoted back
        // in a 401 / rate-limit message) must not trigger retry.
        let body =
            r#"{"error":{"message":"rate limit exceeded; last request had image_url content"}}"#;
        assert!(!is_unsupported_image_url_error(400, body));
    }
}
