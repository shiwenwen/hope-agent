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
//! Streaming + tool-loop chat is **not** in scope here — see Phase 2.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::config::{build_api_url, ANTHROPIC_API_VERSION, CODEX_API_URL};
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
}

fn build_anthropic_body(model: &str, req: &OneShotRequest<'_>) -> Value {
    if let OneShotMode::Independent { system } = req.mode {
        return json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "system": system,
            "messages": [{ "role": "user", "content": req.instruction }],
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
        AssistantAgent::push_user_message(&mut messages, json!(req.instruction));

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
        "messages": [{ "role": "user", "content": req.instruction }],
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
}

fn build_openai_chat_body(model: &str, req: &OneShotRequest<'_>) -> Value {
    if let OneShotMode::Independent { system } = req.mode {
        return json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": req.instruction },
            ],
        });
    }

    if let Some(params) = req.mode.cached_for(ProviderFormat::OpenAIChat) {
        let mut api_messages =
            vec![json!({ "role": "system", "content": &params.system_prompt })];
        api_messages.extend(params.conversation_history.iter().cloned());
        api_messages.push(json!({ "role": "user", "content": req.instruction }));

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
        "messages": [{ "role": "user", "content": req.instruction }],
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
        let body = build_responses_body(self.model, &req, ProviderFormat::Codex);
        let bearer = format!("Bearer {}", self.token);
        let mut headers: Vec<(&str, &str)> = vec![("Authorization", bearer.as_str())];
        if !self.account_id.is_empty() {
            headers.push(("X-Account-ID", self.account_id));
        }
        let result = send_json_request(client, CODEX_API_URL, &body, &headers).await?;
        Ok(OneShotResult {
            text: extract_responses_text(&result),
            usage: extract_openai_usage(&result),
        })
    }
}

// ── Shared Responses-protocol body builder ───────────────────────────

/// Build the request body for OpenAI Responses or Codex (same protocol,
/// different auth/endpoint handled by the caller). `expected_format`
/// gates whether `cached` is honored.
fn build_responses_body(
    model: &str,
    req: &OneShotRequest<'_>,
    expected_format: ProviderFormat,
) -> Value {
    if let OneShotMode::Independent { system } = req.mode {
        return json!({
            "model": model,
            "store": false,
            "stream": false,
            "instructions": system,
            "input": [{ "role": "user", "content": req.instruction }],
            "max_output_tokens": req.max_tokens,
        });
    }

    if let Some(params) = req.mode.cached_for(expected_format) {
        let mut input = params.conversation_history.clone();
        AssistantAgent::push_user_message(&mut input, json!(req.instruction));
        return json!({
            "model": model,
            "store": false,
            "stream": false,
            "instructions": &params.system_prompt,
            "input": input,
            "tools": &params.tool_schemas,
            "max_output_tokens": req.max_tokens,
        });
    }

    json!({
        "model": model,
        "store": false,
        "stream": false,
        "input": [{ "role": "user", "content": req.instruction }],
        "max_output_tokens": req.max_tokens,
    })
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
    ChatUsage {
        input_tokens,
        output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: usage
            .and_then(|u| u.get("cache_creation_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_read_input_tokens: usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        last_input_tokens: input_tokens,
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
            mode: OneShotMode::Independent { system: "SUMMARIZER" },
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
            mode: OneShotMode::Independent { system: "SUMMARIZER" },
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
            })
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
            mode: OneShotMode::Independent { system: "SUMMARIZER" },
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
            })
        );
    }

    #[test]
    fn codex_shares_responses_body_builder() {
        // Codex and OpenAIResponses must produce byte-identical bodies for
        // equivalent inputs — only URL and auth header differ between adapters.
        let params = cached_codex();
        let req = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
        };
        let codex_body = build_responses_body("gpt-5", &req, ProviderFormat::Codex);

        let params2 = cached_responses();
        let req2 = OneShotRequest {
            instruction: "do X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params2),
        };
        let responses_body = build_responses_body("gpt-5", &req2, ProviderFormat::OpenAIResponses);

        assert_eq!(codex_body, responses_body);
    }

    #[test]
    fn responses_format_mismatch_falls_back() {
        let params = cached_anthropic(); // wrong format for Responses adapter
        let req = OneShotRequest {
            instruction: "X",
            max_tokens: 100,
            mode: OneShotMode::Cached(&params),
        };
        let body = build_responses_body("gpt-5", &req, ProviderFormat::OpenAIResponses);
        assert_eq!(
            body,
            json!({
                "model": "gpt-5",
                "store": false,
                "stream": false,
                "input": [{"role": "user", "content": "X"}],
                "max_output_tokens": 100,
            })
        );
    }

    // ── Response parsing ─────────────────────────────────────────────

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
