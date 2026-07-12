//! Unified one-shot LLM adapter.
//!
//! Replaces the four-branch HTTP code that used to live in `side_query.rs` and
//! `context.rs::summarize_direct`. Each provider gets a thin adapter that
//! borrows from the existing `LlmProvider` enum (zero clone), and a single
//! `LlmApiAdapter::one_shot` method covers all three call shapes via
//! [`OneShotMode`]:
//!
//! - [`OneShotMode::Cached`]: reuse the main conversation's
//!   `system_prompt + tool_schemas + history` prefix so prompt-cache hits flow
//!   through. If the snapshot's format mismatches the adapter, falls back to
//!   `Bare`.
//! - [`OneShotMode::Independent`]: fresh request with the given system prompt
//!   and a single user message — used by Tier 3 summarization.
//! - [`OneShotMode::Bare`]: minimal user-only request, no system, no tools.
//!
//! Tool-loop chat is **not** in scope here. A streaming *sibling*
//! [`LlmApiAdapter::one_shot_stream`] does exist — same body/prefix/SSE
//! machinery as `one_shot`, but it forwards per-token text deltas to a callback
//! (used by the design space's live generation preview). The non-streaming
//! `one_shot` path — and every caller of it (side_query / recap / judge) — is
//! byte-for-byte untouched.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::config::{build_api_url, ANTHROPIC_API_VERSION, CODEX_API_URL};
use super::errors::parse_error_response;
use super::providers::anthropic_adapter::parse_anthropic_sse;
use super::providers::codex_adapter::{apply_codex_headers, codex_user_agent};
use super::providers::openai_chat_adapter::parse_chat_completions_sse;
use super::providers::openai_responses_adapter::parse_openai_sse;
use super::types::{AssistantAgent, CacheSafeParams, ChatUsage, LlmProvider, ProviderFormat};

// ── Public types ─────────────────────────────────────────────────────

/// Three mutually exclusive call shapes a one-shot LLM request can take.
/// Modeled as an enum so callers cannot accidentally combine "cached prefix"
/// with "independent system prompt".
pub(super) enum OneShotMode<'a> {
    /// Reuse the main conversation's cache-safe prefix when format matches.
    Cached(&'a CacheSafeParams),
    /// Fresh request with an independent system prompt (e.g. Tier 3 summarizer).
    Independent { system: &'a str },
    /// Minimal user-only request, no prefix, no system.
    Bare,
}

impl<'a> OneShotMode<'a> {
    /// Returns the cached params iff this mode is `Cached` AND the snapshot's
    /// format matches the adapter's expected format. Otherwise returns `None`,
    /// signaling the body builder to fall back to its `Bare` shape.
    fn cached_for(&self, format: ProviderFormat) -> Option<&'a CacheSafeParams> {
        match self {
            OneShotMode::Cached(p) if p.provider_format == format => Some(*p),
            _ => None,
        }
    }
}

pub(super) struct OneShotRequest<'a> {
    pub instruction: &'a str,
    pub max_tokens: u32,
    pub mode: OneShotMode<'a>,
    pub user_content: Option<Value>,
}

pub(super) struct OneShotResult {
    pub text: String,
    pub usage: ChatUsage,
}

#[async_trait]
pub(super) trait LlmApiAdapter: Send + Sync {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult>;

    /// Streaming sibling of [`one_shot`](Self::one_shot): identical body /
    /// cache-prefix / SSE machinery, but forwards each text delta to `on_delta`
    /// as it arrives instead of discarding it. Reuses the same battle-tested SSE
    /// parsers the main chat loop runs. Only the design-space live-preview path
    /// calls this; every non-streaming caller keeps using `one_shot` unchanged.
    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult>;
}

/// Build an error from a non-2xx streaming response by reading its (non-SSE)
/// error body — mirrors [`send_json_request`]'s error path for SSE endpoints
/// where we consume the body stream ourselves.
async fn stream_error(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status().as_u16();
    let err_text = resp.text().await.unwrap_or_default();
    anyhow::anyhow!("{}", parse_error_response(status, &err_text))
}

/// The SSE parsers wrap each text increment in the main-loop streaming envelope
/// `{"type":"text_delta","content":"…"}` (via `events::emit_text_delta`) before
/// calling `on_delta`. `one_shot_stream`'s contract is **raw text deltas**, so
/// this unwraps the envelope: forward only `content` for `text_delta` events,
/// dropping `thinking_delta` / `tool_call` envelopes. Keeps the streamed text
/// byte-aligned with the parser's own `collected_text` (which is also text-only).
fn unwrap_text_delta<'a>(
    on_text: &'a (dyn for<'s> Fn(&'s str) + Send + Sync),
) -> impl Fn(&str) + Send + Sync + 'a {
    move |envelope: &str| {
        if let Ok(v) = serde_json::from_str::<Value>(envelope) {
            if v.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                if let Some(c) = v.get("content").and_then(|c| c.as_str()) {
                    on_text(c);
                }
            }
        }
    }
}

impl LlmProvider {
    pub(super) fn as_adapter(&self) -> Box<dyn LlmApiAdapter + '_> {
        match self {
            LlmProvider::Anthropic {
                api_key,
                base_url,
                model,
            } => Box::new(AnthropicAdapter {
                key: api_key,
                base_url,
                model,
            }),
            LlmProvider::OpenAIChat {
                api_key,
                base_url,
                model,
            } => Box::new(OpenAIChatAdapter {
                key: api_key,
                base_url,
                model,
            }),
            LlmProvider::OpenAIResponses {
                api_key,
                base_url,
                model,
            } => Box::new(OpenAIResponsesAdapter {
                key: api_key,
                base_url,
                model,
            }),
            LlmProvider::Codex {
                access_token,
                account_id,
                model,
            } => Box::new(CodexAdapter {
                token: access_token,
                account_id,
                model,
            }),
        }
    }
}

// ── Anthropic adapter ────────────────────────────────────────────────

pub(super) struct AnthropicAdapter<'a> {
    pub key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
}

#[async_trait]
impl<'a> LlmApiAdapter for AnthropicAdapter<'a> {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult> {
        let body = build_anthropic_body(self.model, &req);
        let api_url = build_api_url(self.base_url, "/v1/messages");
        let result = send_json_request(
            client,
            &api_url,
            &body,
            &[
                ("x-api-key", self.key),
                ("anthropic-version", ANTHROPIC_API_VERSION),
            ],
        )
        .await?;
        Ok(OneShotResult {
            text: extract_anthropic_text(&result),
            usage: extract_anthropic_usage(&result),
        })
    }

    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult> {
        let mut body = build_anthropic_body(self.model, &req);
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".into(), json!(true));
        }
        let api_url = build_api_url(self.base_url, "/v1/messages");
        let request_start = std::time::Instant::now();
        let resp = client
            .post(&api_url)
            .header("x-api-key", self.key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Anthropic stream request failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(stream_error(resp).await);
        }
        let fwd = unwrap_text_delta(on_delta);
        let (text, _tool_calls, _stop, mut usage, _thinking, _ttft) =
            parse_anthropic_sse(resp, request_start, cancel, &fwd).await?;
        usage.last_input_tokens = usage.input_tokens;
        usage.last_cache_creation_input_tokens = usage.cache_creation_input_tokens;
        usage.last_cache_read_input_tokens = usage.cache_read_input_tokens;
        Ok(OneShotResult { text, usage })
    }
}

fn build_anthropic_body(model: &str, req: &OneShotRequest<'_>) -> Value {
    if let OneShotMode::Independent { system } = req.mode {
        return json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "system": system,
            "messages": [{ "role": "user", "content": one_shot_user_content(req) }],
        });
    }

    if let Some(params) = req.mode.cached_for(ProviderFormat::Anthropic) {
        // Tools must be included even though side queries don't execute them:
        // Anthropic's prompt cache requires byte-identical prefix with the main
        // chat request, and the main request always carries tools.
        let system_with_cache = json!([{
            "type": "text",
            "text": &params.system_prompt,
            "cache_control": { "type": "ephemeral" }
        }]);
        let mut tools_with_cache = params.tool_schemas.clone();
        if let Some(last_tool) = tools_with_cache.last_mut() {
            last_tool["cache_control"] = json!({ "type": "ephemeral" });
        }

        let mut messages = params.conversation_history.clone();
        AssistantAgent::push_user_message(&mut messages, one_shot_user_content(req));

        return json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "system": system_with_cache,
            "tools": tools_with_cache,
            "messages": messages,
        });
    }

    json!({
        "model": model,
        "max_tokens": req.max_tokens,
            "messages": [{ "role": "user", "content": one_shot_user_content(req) }],
    })
}

// ── OpenAI Chat Completions adapter ──────────────────────────────────

pub(super) struct OpenAIChatAdapter<'a> {
    pub key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
}

#[async_trait]
impl<'a> LlmApiAdapter for OpenAIChatAdapter<'a> {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult> {
        let body = build_openai_chat_body(self.model, &req);
        let api_url = build_api_url(self.base_url, "/v1/chat/completions");
        let bearer = format!("Bearer {}", self.key);
        let result =
            send_json_request(client, &api_url, &body, &[("Authorization", &bearer)]).await?;
        Ok(OneShotResult {
            text: extract_chat_text(&result),
            usage: extract_openai_usage(&result),
        })
    }

    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult> {
        let mut body = build_openai_chat_body(self.model, &req);
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".into(), json!(true));
            // Chat Completions only reports usage in the stream when asked.
            obj.insert("stream_options".into(), json!({ "include_usage": true }));
        }
        let api_url = build_api_url(self.base_url, "/v1/chat/completions");
        let bearer = format!("Bearer {}", self.key);
        let request_start = std::time::Instant::now();
        let resp = client
            .post(&api_url)
            .header("content-type", "application/json")
            .header("Authorization", &bearer)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("OpenAI chat stream request failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(stream_error(resp).await);
        }
        let fwd = unwrap_text_delta(on_delta);
        let (text, _tool_calls, mut usage, _thinking, _ttft) =
            parse_chat_completions_sse(resp, request_start, None, cancel, &fwd).await?;
        usage.last_input_tokens = usage.input_tokens;
        usage.last_cache_read_input_tokens = usage.cache_read_input_tokens;
        Ok(OneShotResult { text, usage })
    }
}

fn build_openai_chat_body(model: &str, req: &OneShotRequest<'_>) -> Value {
    if let OneShotMode::Independent { system } = req.mode {
        return json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": one_shot_user_content(req) },
            ],
        });
    }

    if let Some(params) = req.mode.cached_for(ProviderFormat::OpenAIChat) {
        let mut api_messages = vec![json!({ "role": "system", "content": &params.system_prompt })];
        api_messages.extend(params.conversation_history.iter().cloned());
        api_messages.push(json!({ "role": "user", "content": one_shot_user_content(req) }));

        let tools_array: Vec<Value> = params
            .tool_schemas
            .iter()
            .map(|t| json!({ "type": "function", "function": t }))
            .collect();

        return json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "messages": api_messages,
            "tools": tools_array,
        });
    }

    json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "messages": [{ "role": "user", "content": one_shot_user_content(req) }],
    })
}

// ── OpenAI Responses adapter ─────────────────────────────────────────

pub(super) struct OpenAIResponsesAdapter<'a> {
    pub key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
}

#[async_trait]
impl<'a> LlmApiAdapter for OpenAIResponsesAdapter<'a> {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult> {
        let body = build_responses_body(self.model, &req, ProviderFormat::OpenAIResponses);
        let api_url = build_api_url(self.base_url, "/v1/responses");
        let bearer = format!("Bearer {}", self.key);
        let result =
            send_json_request(client, &api_url, &body, &[("Authorization", &bearer)]).await?;
        Ok(OneShotResult {
            text: extract_responses_text(&result),
            usage: extract_openai_usage(&result),
        })
    }

    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult> {
        let mut body = build_responses_body(self.model, &req, ProviderFormat::OpenAIResponses);
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".into(), json!(true));
        }
        let api_url = build_api_url(self.base_url, "/v1/responses");
        let bearer = format!("Bearer {}", self.key);
        let request_start = std::time::Instant::now();
        let resp = client
            .post(&api_url)
            .header("content-type", "application/json")
            .header("Authorization", &bearer)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("OpenAI responses stream request failed: {}", e))?;
        if !resp.status().is_success() {
            return Err(stream_error(resp).await);
        }
        let fwd = unwrap_text_delta(on_delta);
        let (text, _tool_calls, mut usage, _thinking, _ttft) =
            parse_openai_sse(resp, request_start, cancel.as_ref(), &fwd).await?;
        usage.last_input_tokens = usage.input_tokens;
        usage.last_cache_read_input_tokens = usage.cache_read_input_tokens;
        Ok(OneShotResult { text, usage })
    }
}

// ── Codex adapter (OpenAI Responses protocol + OAuth headers) ────────

pub(super) struct CodexAdapter<'a> {
    pub token: &'a str,
    pub account_id: &'a str,
    pub model: &'a str,
}

#[async_trait]
impl<'a> LlmApiAdapter for CodexAdapter<'a> {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult> {
        // Codex already streams internally (its backend rejects `stream:false`);
        // the non-streaming path just discards deltas via a noop, exactly as
        // before this method was factored into `codex_call`.
        let cancel = AtomicBool::new(false);
        self.codex_call(client, req, &cancel, &|_: &str| {}).await
    }

    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult> {
        // Unwrap the text_delta envelope here (not in codex_call) so the
        // non-streaming `one_shot` noop path stays envelope-parse-free.
        let fwd = unwrap_text_delta(on_delta);
        self.codex_call(client, req, cancel.as_ref(), &fwd).await
    }
}

impl<'a> CodexAdapter<'a> {
    async fn codex_call(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &AtomicBool,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult> {
        let body = build_responses_body(self.model, &req, ProviderFormat::Codex);
        let body_size = serde_json::to_string(&body).map(|s| s.len()).unwrap_or(0);
        let input_count = body
            .get("input")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);
        let tool_count = body
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);
        let instructions_present = body
            .get("instructions")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let request_start = std::time::Instant::now();
        let resp = match apply_codex_headers(
            client.post(CODEX_API_URL),
            self.token,
            self.account_id,
            codex_user_agent(),
        )
        .json(&body)
        .send()
        .await
        {
            Ok(resp) => resp,
            Err(e) => {
                app_warn!(
                    "agent",
                    "codex_one_shot",
                    "Codex one-shot network error: model={} has_token={} has_account_id={} instructions_present={} input_count={} tool_count={} body={}B err={}",
                    self.model,
                    !self.token.is_empty(),
                    !self.account_id.is_empty(),
                    instructions_present,
                    input_count,
                    tool_count,
                    body_size,
                    e
                );
                return Err(anyhow::anyhow!("Codex one-shot request failed: {}", e));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err_text = resp.text().await.unwrap_or_default();
            let friendly = parse_error_response(status, &err_text);
            let error_preview = if err_text.len() > 500 {
                format!("{}...", crate::truncate_utf8(&err_text, 500))
            } else {
                err_text.clone()
            };
            let error_preview = crate::logging::redact_sensitive(&error_preview);
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "error",
                    "agent",
                    "agent::codex_one_shot::error",
                    &format!(
                        "Codex one-shot API error ({}): model={} has_token={} has_account_id={} instructions_present={} input_count={} tool_count={} body={}B",
                        status,
                        self.model,
                        !self.token.is_empty(),
                        !self.account_id.is_empty(),
                        instructions_present,
                        input_count,
                        tool_count,
                        body_size
                    ),
                    Some(
                        json!({
                            "status": status,
                            "model": self.model,
                            "has_token": !self.token.is_empty(),
                            "has_account_id": !self.account_id.is_empty(),
                            "instructions_present": instructions_present,
                            "input_count": input_count,
                            "tool_count": tool_count,
                            "body_size_bytes": body_size,
                            "error_preview": error_preview,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }
            return Err(anyhow::anyhow!("{}", friendly));
        }

        let (text, _tool_calls, mut usage, _thinking, _ttft) = match parse_openai_sse(
            resp,
            request_start,
            cancel,
            on_delta,
        )
        .await
        {
            Ok(parsed) => parsed,
            Err(e) => {
                app_warn!(
                    "agent",
                    "codex_one_shot",
                    "Codex one-shot SSE parse failed: model={} has_token={} has_account_id={} instructions_present={} input_count={} tool_count={} body={}B err={}",
                    self.model,
                    !self.token.is_empty(),
                    !self.account_id.is_empty(),
                    instructions_present,
                    input_count,
                    tool_count,
                    body_size,
                    e
                );
                return Err(e);
            }
        };
        // One-shot is single-round: last == only round.
        usage.last_input_tokens = usage.input_tokens;
        usage.last_cache_creation_input_tokens = usage.cache_creation_input_tokens;
        usage.last_cache_read_input_tokens = usage.cache_read_input_tokens;

        Ok(OneShotResult { text, usage })
    }
}

// ── Shared Responses-protocol body builder ───────────────────────────

/// Build a Responses-protocol body for OpenAI Responses or Codex.
///
/// Codex diverges from OpenAI Responses on two fields only:
/// - `stream: true` (Codex backend rejects `stream: false` with
///   `{"detail":"Stream must be set to true"}`).
/// - no `max_output_tokens` (Codex rejects it as unsupported).
///
/// Side queries deliberately don't forward temperature / awareness suffixes
/// from the main turn — see [`OneShotRequest`]. Reasoning effort is pinned
/// to `low` here (rather than inherited or omitted) because side_query is
/// always a short, low-stakes background task: recall-shortlist selection,
/// title generation, memory extraction, summary. Omitting the field falls
/// back to the account/model default (often `medium`), which on reasoning
/// models routinely blows past the bounded timeouts (active_memory's 3–8s,
/// title 10s) before the first token arrives.
const BARE_RESPONSES_INSTRUCTIONS: &str =
    "You are a helpful assistant. Follow the user's instruction exactly.";

fn build_responses_body(
    model: &str,
    req: &OneShotRequest<'_>,
    expected_format: ProviderFormat,
) -> Value {
    let is_codex = matches!(expected_format, ProviderFormat::Codex);
    let stream = is_codex;

    let mut body = match req.mode {
        OneShotMode::Independent { system } => json!({
            "model": model,
            "store": false,
            "stream": stream,
            "instructions": system,
            "input": [{ "role": "user", "content": one_shot_user_content(req) }],
        }),
        _ => {
            if let Some(params) = req.mode.cached_for(expected_format) {
                let mut input =
                    AssistantAgent::normalize_history_for_responses(&params.conversation_history);
                AssistantAgent::push_user_message(&mut input, one_shot_user_content(req));
                json!({
                    "model": model,
                    "store": false,
                    "stream": stream,
                    "instructions": &params.system_prompt,
                    "input": input,
                    "tools": &params.tool_schemas,
                })
            } else {
                json!({
                    "model": model,
                    "store": false,
                    "stream": stream,
                    "instructions": BARE_RESPONSES_INSTRUCTIONS,
                    "input": [{ "role": "user", "content": one_shot_user_content(req) }],
                })
            }
        }
    };
    let body_obj = body.as_object_mut().expect("json! always produces object");
    if !is_codex {
        body_obj.insert("max_output_tokens".into(), json!(req.max_tokens));
    }
    body_obj.insert("reasoning".into(), json!({ "effort": "low" }));
    body
}

fn one_shot_user_content(req: &OneShotRequest<'_>) -> Value {
    req.user_content
        .clone()
        .unwrap_or_else(|| json!(req.instruction))
}

// ── Shared HTTP + response-extraction helpers ───────────────────────

/// Send a JSON request and parse the JSON response.
pub(super) async fn send_json_request(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    headers: &[(&str, &str)],
) -> Result<Value> {
    let mut req = client
        .post(url)
        .header("content-type", "application/json")
        .json(body);

    for (key, value) in headers {
        req = req.header(*key, *value);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("LLM request failed: {}", e))?;

    if !resp.status().is_success() {
        let err = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("LLM API error: {}", err));
    }

    resp.json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse LLM response: {}", e))
}

/// Extract text from Anthropic Messages API response.
pub(super) fn extract_anthropic_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        })
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract text from OpenAI Chat Completions response.
pub(super) fn extract_chat_text(result: &Value) -> String {
    result
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract text from OpenAI Responses API non-streaming response.
pub(super) fn extract_responses_text(result: &Value) -> String {
    result
        .get("output")
        .and_then(|o| o.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("message"))
                .filter_map(|item| item.get("content").and_then(|c| c.as_array()))
                .flat_map(|blocks| blocks.iter())
                .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some("output_text"))
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Extract usage from Anthropic Messages API response.
pub(super) fn extract_anthropic_usage(result: &Value) -> ChatUsage {
    let usage = result.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_creation = usage
        .and_then(|u| u.get("cache_creation_input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    ChatUsage {
        input_tokens,
        output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: cache_creation,
        cache_read_input_tokens: cache_read,
        last_input_tokens: input_tokens,
        last_cache_creation_input_tokens: cache_creation,
        last_cache_read_input_tokens: cache_read,
    }
}

/// Extract usage from OpenAI Chat/Responses API response.
pub(super) fn extract_openai_usage(result: &Value) -> ChatUsage {
    let usage = result.get("usage");
    let cached = usage
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens").or_else(|| u.get("prompt_tokens")))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    ChatUsage {
        input_tokens,
        output_tokens: usage
            .and_then(|u| {
                u.get("output_tokens")
                    .or_else(|| u.get("completion_tokens"))
            })
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
        // One-shot calls are single-round: last == only == total.
        last_input_tokens: input_tokens,
        last_cache_creation_input_tokens: 0,
        last_cache_read_input_tokens: cached,
    }
}

// ── Tests ────────────────────────────────────────────────────────────
//
// The body builders are pure functions, so we can verify their output
// shape without any HTTP. This is the only line of defense against
// byte-level prompt-cache regressions: if a JSON key insertion order
// changes, Anthropic's `cache_control` will miss and OpenAI's prefix
// cache will rebuild from scratch.

#[cfg(test)]
mod tests {
    use super::*;

    fn cached_anthropic() -> CacheSafeParams {
        CacheSafeParams {
            system_prompt: "SYS".to_string(),
            tool_schemas: vec![
                json!({"name": "tool_a", "input_schema": {}}),
                json!({"name": "tool_b", "input_schema": {}}),
            ],
            conversation_history: vec![
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "hi"}),
            ],
            provider_format: ProviderFormat::Anthropic,
        }
    }

    fn cached_openai_chat() -> CacheSafeParams {
        CacheSafeParams {
            provider_format: ProviderFormat::OpenAIChat,
            ..cached_anthropic()
        }
    }

    fn cached_responses() -> CacheSafeParams {
        CacheSafeParams {
            provider_format: ProviderFormat::OpenAIResponses,
            ..cached_anthropic()
        }
    }

    fn cached_codex() -> CacheSafeParams {
        CacheSafeParams {
            provider_format: ProviderFormat::Codex,
            ..cached_anthropic()
        }
    }

    // ── Anthropic ────────────────────────────────────────────────────

    #[test]
    fn anthropic_cache_friendly_body_shape() {
        let params = cached_anthropic();
        let req = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };
        let body = build_anthropic_body("claude-test", &req);
        assert_eq!(
            body,
            json!({
                "model": "claude-test",
                "max_tokens": 100,
                "system": [{
                    "type": "text",
                    "text": "SYS",
                    "cache_control": { "type": "ephemeral" }
                }],
                "tools": [
                    {"name": "tool_a", "input_schema": {}},
                    {"name": "tool_b", "input_schema": {}, "cache_control": {"type": "ephemeral"}},
                ],
                "messages": [
                    {"role": "user", "content": "hello"},
                    {"role": "assistant", "content": "hi"},
                    {"role": "user", "content": "do X"},
                ],
            })
        );
    }

    #[test]
    fn anthropic_system_override_body_matches_summarize_direct() {
        let req = OneShotRequest {
            instruction: "PROMPT",
            max_tokens: 4096,
            mode: OneShotMode::Independent {
                system: "SUMMARIZER",
            },
            user_content: None,
        };
        let body = build_anthropic_body("claude-test", &req);
        assert_eq!(
            body,
            json!({
                "model": "claude-test",
                "max_tokens": 4096,
                "system": "SUMMARIZER",
                "messages": [{"role": "user", "content": "PROMPT"}],
            })
        );
    }

    #[test]
    fn anthropic_fallback_body_when_no_cache_no_override() {
        let req = OneShotRequest {
            instruction: "X",
            max_tokens: 100,
            mode: OneShotMode::Bare,
            user_content: None,
        };
        let body = build_anthropic_body("claude-test", &req);
        assert_eq!(
            body,
            json!({
                "model": "claude-test",
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "X"}],
            })
        );
    }

    #[test]
    fn anthropic_format_mismatch_falls_back() {
        // cached is OpenAIChat format → Anthropic adapter degrades to bare.
        let params = cached_openai_chat();
        let req = OneShotRequest {
            instruction: "X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };
        let body = build_anthropic_body("claude-test", &req);
        assert_eq!(
            body,
            json!({
                "model": "claude-test",
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "X"}],
            })
        );
    }

    // ── OpenAI Chat ──────────────────────────────────────────────────

    #[test]
    fn openai_chat_cache_friendly_body_shape() {
        let params = cached_openai_chat();
        let req = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };
        let body = build_openai_chat_body("gpt-test", &req);
        assert_eq!(
            body,
            json!({
                "model": "gpt-test",
                "max_tokens": 100,
                "messages": [
                    {"role": "system", "content": "SYS"},
                    {"role": "user", "content": "hello"},
                    {"role": "assistant", "content": "hi"},
                    {"role": "user", "content": "do X"},
                ],
                "tools": [
                    {"type": "function", "function": {"name": "tool_a", "input_schema": {}}},
                    {"type": "function", "function": {"name": "tool_b", "input_schema": {}}},
                ],
            })
        );
    }

    #[test]
    fn openai_chat_system_override_body_matches_summarize_direct() {
        let req = OneShotRequest {
            instruction: "PROMPT",
            max_tokens: 4096,
            mode: OneShotMode::Independent {
                system: "SUMMARIZER",
            },
            user_content: None,
        };
        let body = build_openai_chat_body("gpt-test", &req);
        assert_eq!(
            body,
            json!({
                "model": "gpt-test",
                "max_tokens": 4096,
                "messages": [
                    {"role": "system", "content": "SUMMARIZER"},
                    {"role": "user", "content": "PROMPT"},
                ],
            })
        );
    }

    // ── Responses (OpenAI Responses + Codex) ─────────────────────────

    #[test]
    fn responses_cache_friendly_body_shape() {
        let params = cached_responses();
        let req = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };
        let body = build_responses_body("gpt-5", &req, ProviderFormat::OpenAIResponses);
        assert_eq!(
            body,
            json!({
                "model": "gpt-5",
                "store": false,
                "stream": false,
                "instructions": "SYS",
                "input": [
                    {"role": "user", "content": "hello"},
                    {"role": "assistant", "content": "hi"},
                    {"role": "user", "content": "do X"},
                ],
                "tools": [
                    {"name": "tool_a", "input_schema": {}},
                    {"name": "tool_b", "input_schema": {}},
                ],
                "max_output_tokens": 100,
                "reasoning": {"effort": "low"},
            })
        );
    }

    // With `store: false`, *any* reasoning item replayed in a follow-up
    // request 404s on the server (id is a dangling reference; even
    // encrypted_content-bearing items get looked up by id first). The
    // cached body must therefore drop every reasoning item, not just the
    // ones missing encrypted_content.
    #[test]
    fn responses_cached_body_drops_all_reasoning_items() {
        let mut params = cached_responses();
        params.conversation_history.push(json!({
            "type": "reasoning",
            "id": "rs_missing",
            "summary": [],
            "status": "completed"
        }));
        params.conversation_history.push(json!({
            "type": "reasoning",
            "id": "rs_with_payload",
            "summary": [],
            "encrypted_content": "enc",
            "status": "completed"
        }));
        let req = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };

        let body = build_responses_body("gpt-5", &req, ProviderFormat::OpenAIResponses);
        let input = body.get("input").and_then(|v| v.as_array()).unwrap();
        let reasoning_items: Vec<&serde_json::Value> = input
            .iter()
            .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("reasoning"))
            .collect();

        assert!(
            reasoning_items.is_empty(),
            "reasoning item leaked into cached body: {:?}",
            reasoning_items
        );
    }

    #[test]
    fn responses_system_override_uses_responses_protocol_not_chat_completions() {
        // Regression guard: legacy summarize_direct sent a Chat Completions
        // body here (worked by accident on dual-protocol providers, 404'd on
        // Codex). The unified adapter must use the proper Responses body.
        let req = OneShotRequest {
            instruction: "PROMPT",
            max_tokens: 4096,
            mode: OneShotMode::Independent {
                system: "SUMMARIZER",
            },
            user_content: None,
        };
        let body = build_responses_body("gpt-5", &req, ProviderFormat::OpenAIResponses);
        assert_eq!(
            body,
            json!({
                "model": "gpt-5",
                "store": false,
                "stream": false,
                "instructions": "SUMMARIZER",
                "input": [{"role": "user", "content": "PROMPT"}],
                "max_output_tokens": 4096,
                "reasoning": {"effort": "low"},
            })
        );
    }

    #[test]
    fn codex_body_diverges_from_openai_responses_on_dialect_only() {
        // Codex and OpenAIResponses speak the same Responses protocol but
        // diverge in exactly two places:
        //   - Codex demands `stream: true` (the backend rejects `false`).
        //   - Codex rejects `max_output_tokens` as unsupported.
        // Anything else MUST stay identical, so downstream prompt-cache
        // invariants and tool-call dispatch keep working uniformly.
        let params = cached_codex();
        let req = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };
        let codex_body = build_responses_body("gpt-5", &req, ProviderFormat::Codex);
        assert_eq!(
            codex_body,
            json!({
                "model": "gpt-5",
                "store": false,
                "stream": true,
                "instructions": "SYS",
                "input": [
                    {"role": "user", "content": "hello"},
                    {"role": "assistant", "content": "hi"},
                    {"role": "user", "content": "do X"},
                ],
                "tools": [
                    {"name": "tool_a", "input_schema": {}},
                    {"name": "tool_b", "input_schema": {}},
                ],
                "reasoning": {"effort": "low"},
            })
        );

        // Cross-check the two dialects against each other: strip the known
        // diffs from the OpenAI body and the remainder must match Codex.
        let params2 = cached_responses();
        let req2 = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params2),
            user_content: None,
        };
        let mut responses_body =
            build_responses_body("gpt-5", &req2, ProviderFormat::OpenAIResponses);
        let responses_obj = responses_body.as_object_mut().unwrap();
        responses_obj.insert("stream".to_string(), Value::Bool(true));
        responses_obj.remove("max_output_tokens");
        assert_eq!(codex_body, responses_body);
    }

    #[test]
    fn codex_bare_body_still_sends_required_instructions() {
        let req = OneShotRequest {
            instruction: "pick the relevant memory",
            max_tokens: 100,
            mode: OneShotMode::Bare,
            user_content: None,
        };
        let body = build_responses_body("gpt-5", &req, ProviderFormat::Codex);

        let instructions = body
            .get("instructions")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        assert!(
            !instructions.trim().is_empty(),
            "Codex rejects one-shot requests without instructions"
        );
    }

    #[test]
    fn responses_format_mismatch_falls_back() {
        let params = cached_anthropic(); // wrong format for Responses adapter
        let req = OneShotRequest {
            instruction: "X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
            user_content: None,
        };
        let body = build_responses_body("gpt-5", &req, ProviderFormat::OpenAIResponses);
        assert_eq!(
            body,
            json!({
                "model": "gpt-5",
                "store": false,
                "stream": false,
                "instructions": BARE_RESPONSES_INSTRUCTIONS,
                "input": [{"role": "user", "content": "X"}],
                "max_output_tokens": 100,
                "reasoning": {"effort": "low"},
            })
        );
    }

    // ── Response parsing ─────────────────────────────────────────────

    #[test]
    fn unwrap_text_delta_forwards_only_raw_text_content() {
        use std::sync::Mutex;
        let acc = Mutex::new(String::new());
        let sink: &(dyn for<'s> Fn(&'s str) + Send + Sync) =
            &|s: &str| acc.lock().unwrap().push_str(s);
        let fwd = unwrap_text_delta(sink);
        // text_delta → content forwarded; thinking / tool / non-JSON envelopes dropped.
        fwd(&json!({"type": "text_delta", "content": "<<<CSS>>>\n"}).to_string());
        fwd(&json!({"type": "thinking_delta", "content": "pondering"}).to_string());
        fwd(&json!({"type": "tool_call", "name": "x"}).to_string());
        fwd(&json!({"type": "text_delta", "content": "body{}"}).to_string());
        fwd("not json at all");
        let out = acc.lock().unwrap().clone();
        // Streamed text is byte-identical to the model's raw output — the JSON
        // envelope never leaks into the design preview parser.
        assert_eq!(out, "<<<CSS>>>\nbody{}");
        assert!(!out.contains("thinking"));
        assert!(!out.contains("text_delta"));
    }

    #[test]
    fn extract_anthropic_text_picks_first_text_block() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "tool_use", "name": "x"},
            ]
        });
        assert_eq!(extract_anthropic_text(&resp), "hello");
    }

    #[test]
    fn extract_responses_text_concatenates_output_text_blocks() {
        let resp = json!({
            "output": [
                {"type": "reasoning", "summary": "thinking"},
                {"type": "message", "content": [
                    {"type": "output_text", "text": "part1"},
                    {"type": "output_text", "text": "part2"},
                ]},
            ]
        });
        assert_eq!(extract_responses_text(&resp), "part1part2");
    }

    #[test]
    fn extract_openai_usage_handles_both_chat_and_responses_field_names() {
        // Chat Completions style.
        let chat_resp = json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "prompt_tokens_details": { "cached_tokens": 80 },
            }
        });
        let usage = extract_openai_usage(&chat_resp);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_read_input_tokens, 80);
        assert_eq!(usage.last_input_tokens, 100);

        // Responses style.
        let responses_resp = json!({
            "usage": {
                "input_tokens": 200,
                "output_tokens": 150,
                "prompt_tokens_details": { "cached_tokens": 180 },
            }
        });
        let usage2 = extract_openai_usage(&responses_resp);
        assert_eq!(usage2.input_tokens, 200);
        assert_eq!(usage2.output_tokens, 150);
        assert_eq!(usage2.cache_read_input_tokens, 180);
    }
}
