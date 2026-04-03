//! Side-query mechanism for cache-friendly LLM calls.
//!
//! Reuses the main conversation's system_prompt + tool_schemas + conversation_history
//! as API request prefix, enabling prompt cache hits on Anthropic (explicit `cache_control`)
//! and OpenAI (automatic prefix caching). Side queries are non-streaming, single-turn,
//! no tool loop, no compaction.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use super::config::{build_api_url, ANTHROPIC_API_VERSION};
use super::types::{
    AssistantAgent, CacheSafeParams, ChatUsage, LlmProvider, ProviderFormat, SideQueryResult,
};

impl AssistantAgent {
    /// Save cache-safe params after building the main chat request.
    /// Called from each provider's chat method after compaction, before the tool loop.
    /// Uses Arc to avoid deep-cloning conversation data on every chat turn.
    pub(super) fn save_cache_safe_params(
        &self,
        system_prompt: String,
        tool_schemas: Vec<serde_json::Value>,
        conversation_history: Vec<serde_json::Value>,
    ) {
        let format = ProviderFormat::from(&self.provider);
        *self
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(CacheSafeParams {
            system_prompt,
            tool_schemas,
            conversation_history,
            provider_format: format,
        }));
    }

    /// Execute a side query that reuses the main conversation's cached prefix.
    ///
    /// - Non-streaming, single-turn, no tool loop, no compaction
    /// - Falls back to a minimal request if no cache-safe params are available
    /// - Returns response text + usage metrics (including cache hit info)
    pub async fn side_query(
        &self,
        instruction: &str,
        max_tokens: u32,
    ) -> Result<SideQueryResult> {
        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        // Arc::clone is cheap (pointer bump), no deep copy
        let cached = self
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        match &self.provider {
            LlmProvider::Anthropic {
                api_key,
                base_url,
                model,
            } => {
                self.side_query_anthropic(
                    &client,
                    api_key,
                    base_url,
                    model,
                    instruction,
                    max_tokens,
                    cached.as_deref(),
                )
                .await
            }
            LlmProvider::OpenAIChat {
                api_key,
                base_url,
                model,
            } => {
                self.side_query_openai_chat(
                    &client,
                    api_key,
                    base_url,
                    model,
                    instruction,
                    max_tokens,
                    cached.as_deref(),
                )
                .await
            }
            LlmProvider::OpenAIResponses {
                api_key,
                base_url,
                model,
            } => {
                let api_url = build_api_url(base_url, "/v1/responses");
                self.side_query_responses(
                    &client,
                    &api_url,
                    api_key,
                    "",
                    model,
                    instruction,
                    max_tokens,
                    cached.as_deref(),
                    ProviderFormat::OpenAIResponses,
                )
                .await
            }
            LlmProvider::Codex {
                access_token,
                account_id,
                model,
            } => {
                self.side_query_responses(
                    &client,
                    "https://chatgpt.com/backend-api/codex/v1/responses",
                    access_token,
                    account_id,
                    model,
                    instruction,
                    max_tokens,
                    cached.as_deref(),
                    ProviderFormat::Codex,
                )
                .await
            }
        }
    }

    // ── Anthropic ────────────────────────────────────────────────────

    async fn side_query_anthropic(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        base_url: &str,
        model: &str,
        instruction: &str,
        max_tokens: u32,
        cached: Option<&CacheSafeParams>,
    ) -> Result<SideQueryResult> {
        let api_url = build_api_url(base_url, "/v1/messages");

        let body = if let Some(params) =
            cached.filter(|p| p.provider_format == ProviderFormat::Anthropic)
        {
            // Cache-friendly path: reuse system + tools + history prefix.
            // Tools are included despite no tool loop to maintain byte-identical prefix
            // with the main chat request, which is required for prompt cache hits.
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
            Self::push_user_message(&mut messages, json!(instruction));

            json!({
                "model": model,
                "max_tokens": max_tokens,
                "system": system_with_cache,
                "tools": tools_with_cache,
                "messages": messages,
            })
        } else {
            json!({
                "model": model,
                "max_tokens": max_tokens,
                "messages": [{ "role": "user", "content": instruction }],
            })
        };

        let result = send_json_request(
            client,
            &api_url,
            &body,
            &[
                ("x-api-key", api_key),
                ("anthropic-version", ANTHROPIC_API_VERSION),
            ],
        )
        .await?;

        let text = extract_anthropic_text(&result);
        let usage = extract_anthropic_usage(&result);
        Ok(SideQueryResult { text, usage })
    }

    // ── OpenAI Chat Completions ──────────────────────────────────────

    async fn side_query_openai_chat(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        base_url: &str,
        model: &str,
        instruction: &str,
        max_tokens: u32,
        cached: Option<&CacheSafeParams>,
    ) -> Result<SideQueryResult> {
        let api_url = build_api_url(base_url, "/v1/chat/completions");

        let body = if let Some(params) =
            cached.filter(|p| p.provider_format == ProviderFormat::OpenAIChat)
        {
            // Tools included for prefix alignment (OpenAI auto-caches matching prefixes)
            let mut api_messages =
                vec![json!({ "role": "system", "content": &params.system_prompt })];
            api_messages.extend(params.conversation_history.iter().cloned());
            api_messages.push(json!({ "role": "user", "content": instruction }));

            let tools_array: Vec<serde_json::Value> = params
                .tool_schemas
                .iter()
                .map(|t| json!({ "type": "function", "function": t }))
                .collect();

            json!({
                "model": model,
                "max_tokens": max_tokens,
                "messages": api_messages,
                "tools": tools_array,
            })
        } else {
            json!({
                "model": model,
                "max_tokens": max_tokens,
                "messages": [{ "role": "user", "content": instruction }],
            })
        };

        let bearer = format!("Bearer {}", api_key);
        let result =
            send_json_request(client, &api_url, &body, &[("Authorization", &bearer)]).await?;

        let text = extract_chat_text(&result);
        let usage = extract_openai_usage(&result);
        Ok(SideQueryResult { text, usage })
    }

    // ── OpenAI Responses / Codex (shared) ────────────────────────────

    async fn side_query_responses(
        &self,
        client: &reqwest::Client,
        api_url: &str,
        token: &str,
        account_id: &str,
        model: &str,
        instruction: &str,
        max_tokens: u32,
        cached: Option<&CacheSafeParams>,
        expected_format: ProviderFormat,
    ) -> Result<SideQueryResult> {
        let body = if let Some(params) =
            cached.filter(|p| p.provider_format == expected_format)
        {
            // Tools included for prefix alignment
            let mut input = params.conversation_history.clone();
            Self::push_user_message(&mut input, json!(instruction));

            json!({
                "model": model,
                "store": false,
                "stream": false,
                "instructions": &params.system_prompt,
                "input": input,
                "tools": &params.tool_schemas,
                "max_output_tokens": max_tokens,
            })
        } else {
            json!({
                "model": model,
                "store": false,
                "stream": false,
                "input": [{ "role": "user", "content": instruction }],
                "max_output_tokens": max_tokens,
            })
        };

        let bearer = format!("Bearer {}", token);
        let mut headers: Vec<(&str, &str)> = vec![("Authorization", bearer.as_str())];
        if !account_id.is_empty() {
            headers.push(("X-Account-ID", account_id));
        }

        let result = send_json_request(client, api_url, &body, &headers).await?;

        let text = extract_responses_text(&result);
        let usage = extract_openai_usage(&result);
        Ok(SideQueryResult { text, usage })
    }
}

// ── Shared helpers ───────────────────────────────────────────────────

/// Send a JSON request and parse the JSON response.
async fn send_json_request(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
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
        .map_err(|e| anyhow::anyhow!("Side query request failed: {}", e))?;

    if !resp.status().is_success() {
        let err = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Side query API error: {}", err));
    }

    resp.json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse side query response: {}", e))
}

/// Extract text from Anthropic Messages API response.
fn extract_anthropic_text(result: &serde_json::Value) -> String {
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
fn extract_chat_text(result: &serde_json::Value) -> String {
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
fn extract_responses_text(result: &serde_json::Value) -> String {
    result
        .get("output")
        .and_then(|o| o.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("message"))
                .filter_map(|item| item.get("content").and_then(|c| c.as_array()))
                .flat_map(|blocks| blocks.iter())
                .filter(|block| {
                    block.get("type").and_then(|t| t.as_str()) == Some("output_text")
                })
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Extract usage from Anthropic Messages API response.
fn extract_anthropic_usage(result: &serde_json::Value) -> ChatUsage {
    let usage = result.get("usage");
    ChatUsage {
        input_tokens: usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
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
    }
}

/// Extract usage from OpenAI Chat/Responses API response.
fn extract_openai_usage(result: &serde_json::Value) -> ChatUsage {
    let usage = result.get("usage");
    let cached = usage
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    ChatUsage {
        input_tokens: usage
            .and_then(|u| u.get("input_tokens").or_else(|| u.get("prompt_tokens")))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: usage
            .and_then(|u| {
                u.get("output_tokens")
                    .or_else(|| u.get("completion_tokens"))
            })
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached,
    }
}
