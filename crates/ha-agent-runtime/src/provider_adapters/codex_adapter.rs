//! Codex (ChatGPT subscription) adapter implementing [`StreamingChatAdapter`].
//!
//! Same wire protocol as OpenAI Responses (uses [`ResponsesRequest`] body and
//! [`super::openai_responses_adapter::parse_openai_sse`] for streaming) — the
//! difference is the endpoint ([`CODEX_API_URL`]), the auth scheme (OAuth
//! `access_token` + `chatgpt-account-id` header + special user agent). One
//! prepared request performs exactly one network send; retries are owned by
//! the request-plan/failover layer so dispatch ambiguity remains explicit.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::api_types::ResponsesRequest;
use super::super::config::CODEX_API_URL;
use super::super::errors::{os_version, parse_error_response};
use super::super::events::{
    expand_responses_image_markers_for_api, project_responses_image_markers_for_token_count,
};
use super::super::streaming_adapter::{
    observe_before_send, observe_response_started, ExecutedTool, PreparedProviderRequest,
    PreparedRequestVariant, ProviderAccountingInput, ProviderDispatchObserver,
    ProviderDispatchUnknown, ProviderEndpointKind, ProviderRequestShape, RoundOutcome,
    RoundRequest, StreamingChatAdapter,
};
use super::super::types::{AssistantAgent, ProviderFormat};
use super::openai_responses_adapter::{parse_openai_sse, push_responses_assistant_message};
use crate::tool_defs::ToolProvider;

/// Process-stable User-Agent for Codex requests.
pub(crate) fn codex_user_agent() -> &'static str {
    static UA: OnceLock<String> = OnceLock::new();
    UA.get_or_init(|| {
        format!(
            "Hope Agent ({} {}; {})",
            std::env::consts::OS,
            os_version(),
            std::env::consts::ARCH,
        )
    })
}

/// Apply Codex's OAuth + SSE headers to a [`reqwest::RequestBuilder`].
/// Shared by streaming chat_round and one-shot side_query.
pub(crate) fn apply_codex_headers(
    builder: reqwest::RequestBuilder,
    access_token: &str,
    account_id: &str,
    user_agent: &str,
) -> reqwest::RequestBuilder {
    builder
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("chatgpt-account-id", account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .header("originator", "hope-agent")
        .header("User-Agent", user_agent)
        .header("accept", "text/event-stream")
}

fn build_codex_request(
    model: &str,
    reasoning: Option<super::super::api_types::ReasoningConfig>,
    req: &RoundRequest<'_>,
) -> (ResponsesRequest, Vec<Value>) {
    let mut api_input: Vec<Value> = Vec::new();
    for content in super::super::streaming_adapter::dynamic_instruction_suffixes(req) {
        api_input.push(json!({ "role": "system", "content": content }));
    }
    api_input.extend(expand_responses_image_markers_for_api(req.history_for_api));
    if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
        api_input.push(json!({ "role": "user", "content": content }));
    }
    let request = ResponsesRequest {
        model: model.to_string(),
        store: false,
        stream: true,
        instructions: Some(req.system_prompt.to_string()),
        input: api_input.clone(),
        reasoning,
        include: None,
        tools: (!req.is_final_round).then(|| req.tool_schemas.to_vec()),
        temperature: req.temperature,
        // A Responses-shaped wire format is not evidence that Codex supports
        // hosted tool search or OpenAI prompt-cache routing fields.
        prompt_cache_key: None,
        prompt_cache_options: None,
    };
    (request, api_input)
}

pub(crate) struct CodexStreamingAdapter<'a> {
    pub access_token: &'a str,
    pub account_id: &'a str,
    pub model: &'a str,
    pub reasoning: Option<super::super::api_types::ReasoningConfig>,
}

#[async_trait]
impl<'a> StreamingChatAdapter for CodexStreamingAdapter<'a> {
    fn provider_format(&self) -> ProviderFormat {
        ProviderFormat::Codex
    }

    fn tool_provider(&self) -> ToolProvider {
        ToolProvider::OpenAI
    }

    fn normalize_history(&self, history: &mut Vec<Value>) {
        *history = AssistantAgent::normalize_history_for_responses(history);
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
                .map(|content| json!({ "role": "system", "content": content }))
                .collect();
        if let Some(content) = super::super::streaming_adapter::render_dynamic_data_envelope(req) {
            dynamic_items.push(json!({ "role": "user", "content": content }));
        }
        ProviderAccountingInput {
            stable_prompt: serde_json::to_string(&json!({
                "instructions": req.system_prompt
            }))
            .unwrap_or_default(),
            dynamic_prompt: serde_json::to_string(&dynamic_items).unwrap_or_default(),
            history: self.token_count_history_for(req.history_for_api),
        }
    }

    fn prepare_round_request(&self, req: &RoundRequest<'_>) -> Result<PreparedProviderRequest> {
        let (request, api_input) = build_codex_request(self.model, self.reasoning.clone(), req);
        let tool_count = request.tools.as_ref().map_or(0, Vec::len);
        let prepared = PreparedProviderRequest::from_json(
            ProviderEndpointKind::CodexResponses,
            ProviderRequestShape::CodexResponses,
            self.model,
            req.round,
            req.session_id,
            req.reasoning_effort,
            req.vision_bridge_available,
            PreparedRequestVariant::Codex,
            &request,
        )?;
        super::super::token_manifest::log_round_manifest(
            "Codex",
            self.model,
            "codex_responses",
            req,
            request.tools.as_deref().unwrap_or(&[]),
            prepared.identity.body_len as usize,
            false,
        );
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "debug",
                "agent",
                "agent::chat_codex::request",
                &format!(
                    "Codex API request round {}: {} input items, {} tools, body {}B",
                    req.round,
                    api_input.len(),
                    tool_count,
                    prepared.identity.body_len
                ),
                Some(
                    json!({
                        "round": req.round,
                        "endpoint_kind": "codex_responses",
                        "model": self.model,
                        "input_count": api_input.len(),
                        "tool_count": tool_count,
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
        // One claimed plan performs at most one network send. Every retry,
        // including a retry after a received 5xx, requires a new plan/claim.
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
        }
        let request_start = std::time::Instant::now();
        observe_before_send(observer, prepared).await?;

        let attempt = 0_u32;
        let builder = apply_codex_headers(
            client.post(CODEX_API_URL),
            self.access_token,
            self.account_id,
            codex_user_agent(),
        );
        let response =
            super::cancel::send_with_cancel(builder.body(prepared.body().to_vec()), cancel).await;

        let resp = match response {
            Ok(Some(resp)) => {
                observe_response_started(observer, prepared, attempt + 1, &resp).await?;
                if resp.status().is_success() {
                    if let Some(logger) = crate::get_logger() {
                        let ttfb_ms = request_start.elapsed().as_millis() as u64;
                        let headers = resp.headers();
                        let request_id = headers
                            .get("x-request-id")
                            .or_else(|| headers.get("request-id"))
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("-")
                            .to_string();
                        let response_headers = json!({
                                "x-request-id": request_id,
                                "x-ratelimit-limit-requests": headers.get("x-ratelimit-limit-requests").and_then(|v| v.to_str().ok()),
                                "x-ratelimit-limit-tokens": headers.get("x-ratelimit-limit-tokens").and_then(|v| v.to_str().ok()),
                                "x-ratelimit-remaining-requests": headers.get("x-ratelimit-remaining-requests").and_then(|v| v.to_str().ok()),
                                "x-ratelimit-remaining-tokens": headers.get("x-ratelimit-remaining-tokens").and_then(|v| v.to_str().ok()),
                                "openai-model": headers.get("openai-model").and_then(|v| v.to_str().ok()),
                                "retry-after": headers.get("retry-after").and_then(|v| v.to_str().ok()),
                        });
                        logger.log("debug", "agent", "agent::chat_codex::response",
                                &format!("Codex API response: status=200, request_id={}, ttfb={}ms, attempt={}", request_id, ttfb_ms, attempt + 1),
                                Some(json!({
                                    "status": 200,
                                    "ttfb_ms": ttfb_ms,
                                    "attempt": attempt + 1,
                                    "round": prepared.identity.round,
                                    "response_headers": response_headers,
                                }).to_string()),
                                None, None);
                    }
                    resp
                } else {
                    let status = resp.status().as_u16();
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let error_text = match super::cancel::read_text_with_cancel(resp, cancel).await
                    {
                        Ok(Some(text)) => text,
                        Ok(None) => return Ok(super::cancel::cancelled_round_outcome()),
                        Err(_) => String::new(),
                    };

                    if let Some(logger) = crate::get_logger() {
                        let error_fingerprint = crate::cache_routing::audit_fingerprint(
                            "codex-error",
                            error_text.as_bytes(),
                        );
                        logger.log(
                            "error",
                            "agent",
                            "agent::chat_codex::error",
                            &format!("Codex API error ({status})"),
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
                    let friendly = parse_error_response(status, &error_text);
                    return Err(crate::failover::ProviderApiError::from_http_response(
                        "Codex",
                        status,
                        &error_text,
                    )
                    .with_retry_after_header(retry_after.as_deref())
                    .with_display(friendly)
                    .into());
                }
            }
            Ok(None) => {
                return Err(ProviderDispatchUnknown(
                    "cancelled after dispatch claim and before response headers".to_string(),
                )
                .into());
            }
            Err(e) => {
                return Err(ProviderDispatchUnknown(e.to_string()).into());
            }
        };

        // Cancel check before SSE parse begins.
        if cancel.load(Ordering::SeqCst) {
            return Ok(super::cancel::cancelled_round_outcome());
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
                    "agent::chat_codex::tool_loop",
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
            "Codex",
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
        push_responses_assistant_message(history, None, final_text);
    }

    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool {
        outcome.tool_calls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_codex_request, CodexStreamingAdapter};
    use crate::agent::streaming_adapter::{RoundRequest, StreamingChatAdapter};

    #[test]
    fn codex_request_golden_uses_client_deferred_and_no_cache_assumptions() {
        let tools = vec![serde_json::json!({ "type": "function", "name": "read" })];
        let deferred = vec![serde_json::json!({ "type": "function", "name": "browser" })];
        let history = vec![serde_json::json!({ "role": "user", "content": "question" })];
        let req = RoundRequest {
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
            prompt_cache_key: Some("must-not-be-sent"),
            history_for_api: &history,
            vision_bridge_available: false,
            reasoning_effort: None,
            temperature: None,
            max_tokens: 100,
            is_final_round: false,
            round: 0,
        };
        let (request, input) = build_codex_request("gpt-5.4-codex", None, &req);
        let body = serde_json::to_value(&request).unwrap();
        assert_eq!(body["instructions"], "stable");
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("prompt_cache_options").is_none());
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["name"], "read");
        assert!(body.to_string().find("browser").is_none());
        let contents = input
            .iter()
            .map(|item| item["content"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(&contents[..3], &["run", "coding", "question"]);
        assert_eq!(input[3]["role"], "user");
        assert!(contents[3].contains("source=\"active_memory\""));
        assert!(contents[3].contains("source=\"legacy_memory\""));
        assert!(contents[3].contains("source=\"related_notes\""));
        let adapter = CodexStreamingAdapter {
            access_token: "oauth-test-must-stay-in-header",
            account_id: "account-test-must-stay-in-header",
            model: "gpt-5.4-codex",
            reasoning: None,
        };
        let accounting = adapter.token_count_input_for(&req);
        let counted_stable: serde_json::Value =
            serde_json::from_str(&accounting.stable_prompt).unwrap();
        let counted_dynamic: Vec<serde_json::Value> =
            serde_json::from_str(&accounting.dynamic_prompt).unwrap();
        assert_eq!(counted_stable["instructions"], body["instructions"]);
        assert_eq!(
            counted_dynamic,
            vec![input[0].clone(), input[1].clone(), input[3].clone()]
        );
        assert_eq!(accounting.history, history);
        assert_eq!(adapter.token_count_tool_schemas(&req), tools);
        let prepared = adapter.prepare_round_request(&req).unwrap();
        assert_eq!(
            prepared.body_bytes_for_test(),
            serde_json::to_vec(&request).unwrap()
        );
        let wire = String::from_utf8_lossy(prepared.body_bytes_for_test());
        assert!(!wire.contains("oauth-test-must-stay-in-header"));
        assert!(!wire.contains("account-test-must-stay-in-header"));
        assert!(!prepared
            .identity
            .body_keyed_fingerprint
            .contains("oauth-test"));
        let prepared_json: serde_json::Value =
            serde_json::from_slice(prepared.body_bytes_for_test()).unwrap();
        for transport_field in ["authorization", "api_key", "access_token", "account_id"] {
            assert!(prepared_json.get(transport_field).is_none());
        }
    }
}
