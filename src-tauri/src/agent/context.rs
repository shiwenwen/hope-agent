use anyhow::Result;
use serde_json::json;

use super::config::{build_api_url, ANTHROPIC_API_VERSION};
use super::types::{AssistantAgent, LlmProvider};

impl AssistantAgent {
    /// Replace the conversation history (used to restore context from DB).
    pub fn set_conversation_history(&self, history: Vec<serde_json::Value>) {
        *self.conversation_history.lock().unwrap() = history;
    }

    /// Get a clone of the current conversation history (used to persist context to DB).
    pub fn get_conversation_history(&self) -> Vec<serde_json::Value> {
        self.conversation_history.lock().unwrap().clone()
    }

    /// Run context compaction (Tier 1-3) on messages before API call.
    /// If Tier 3 summarization is needed, performs a non-streaming LLM call to summarize old messages.
    pub(super) async fn run_compaction(
        &self,
        messages: &mut Vec<serde_json::Value>,
        system_prompt: &str,
        max_tokens: u32,
        on_delta: &(impl Fn(&str) + Send),
    ) {
        use crate::context_compact;

        let compact_result = context_compact::compact_if_needed(
            messages,
            system_prompt,
            self.context_window,
            max_tokens,
            &self.compact_config,
        );

        if compact_result.tier_applied == 0 {
            return;
        }

        // Log compaction
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info", "context", "compact",
                &format!(
                    "Context compacted: tier={}, {} → {} tokens, {} messages affected",
                    compact_result.tier_applied,
                    compact_result.tokens_before,
                    compact_result.tokens_after,
                    compact_result.messages_affected,
                ),
                None, None, None,
            );
        }

        // Tier 3: LLM summarization needed
        if compact_result.description == "summarization_needed" {
            if let Some(split) = context_compact::split_for_summarization(messages, &self.compact_config) {
                // Notify frontend that summarization is starting
                if let Ok(event) = serde_json::to_string(&json!({
                    "type": "context_compacted",
                    "data": {
                        "tier_applied": 3,
                        "description": "summarizing",
                        "messages_to_summarize": split.summarizable.len(),
                    }
                })) {
                    on_delta(&event);
                }

                let prompt = context_compact::build_summarization_prompt(
                    &split.summarizable,
                    None,
                    &self.compact_config,
                );

                // Try non-streaming summarization call with timeout
                match tokio::time::timeout(
                    std::time::Duration::from_secs(self.compact_config.summarization_timeout_secs),
                    self.summarize_with_model(&prompt),
                ).await {
                    Ok(Ok(summary)) => {
                        context_compact::apply_summary(
                            messages,
                            &summary,
                            split.preserved_start_index,
                            &self.compact_config,
                        );
                        if let Some(logger) = crate::get_logger() {
                            logger.log(
                                "info", "context", "compact",
                                &format!(
                                    "Tier 3 summarization complete: {} messages → {} chars summary, {} messages preserved",
                                    split.summarizable.len(),
                                    summary.len(),
                                    split.preserved.len(),
                                ),
                                None, None, None,
                            );
                        }
                    }
                    Ok(Err(e)) => {
                        if let Some(logger) = crate::get_logger() {
                            logger.log(
                                "warn", "context", "compact",
                                &format!("Tier 3 summarization failed: {}", e),
                                None, None, None,
                            );
                        }
                    }
                    Err(_) => {
                        if let Some(logger) = crate::get_logger() {
                            logger.log(
                                "warn", "context", "compact",
                                &format!("Tier 3 summarization timed out after {}s", self.compact_config.summarization_timeout_secs),
                                None, None, None,
                            );
                        }
                    }
                }
            }
        }

        // Emit compaction event to frontend
        let tokens_after = context_compact::estimate_request_tokens(system_prompt, messages, max_tokens);
        if let Ok(event) = serde_json::to_string(&json!({
            "type": "context_compacted",
            "data": {
                "tier_applied": compact_result.tier_applied,
                "tokens_before": compact_result.tokens_before,
                "tokens_after": tokens_after,
                "messages_affected": compact_result.messages_affected,
                "description": compact_result.description,
            }
        })) {
            on_delta(&event);
        }
    }

    /// Non-streaming LLM call for context summarization.
    /// Uses the current provider to generate a summary.
    async fn summarize_with_model(&self, prompt: &str) -> Result<String> {
        use crate::context_compact::SUMMARIZATION_SYSTEM_PROMPT;

        let client = crate::provider::apply_proxy(
            reqwest::Client::builder().user_agent(&self.user_agent)
        )
            .build()
            .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        match &self.provider {
            LlmProvider::Anthropic { api_key, base_url, model } => {
                let api_url = build_api_url(base_url, "/v1/messages");
                let body = json!({
                    "model": model,
                    "max_tokens": self.compact_config.summary_max_tokens,
                    "system": SUMMARIZATION_SYSTEM_PROMPT,
                    "messages": [{ "role": "user", "content": prompt }],
                });
                let resp = client.post(&api_url)
                    .header("x-api-key", api_key)
                    .header("anthropic-version", ANTHROPIC_API_VERSION)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send().await
                    .map_err(|e| anyhow::anyhow!("Summarization request failed: {}", e))?;

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Summarization API error: {}", err));
                }

                let result: serde_json::Value = resp.json().await
                    .map_err(|e| anyhow::anyhow!("Failed to parse summarization response: {}", e))?;

                // Extract text from Anthropic response
                result.get("content")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.iter().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")))
                    .and_then(|b| b.get("text"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("No text in summarization response"))
            }
            LlmProvider::OpenAIChat { api_key, base_url, model } | LlmProvider::OpenAIResponses { api_key, base_url, model } => {
                let api_url = build_api_url(base_url, "/v1/chat/completions");
                let body = json!({
                    "model": model,
                    "max_tokens": self.compact_config.summary_max_tokens,
                    "messages": [
                        { "role": "system", "content": SUMMARIZATION_SYSTEM_PROMPT },
                        { "role": "user", "content": prompt },
                    ],
                });
                let resp = client.post(&api_url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("content-type", "application/json")
                    .json(&body)
                    .send().await
                    .map_err(|e| anyhow::anyhow!("Summarization request failed: {}", e))?;

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Summarization API error: {}", err));
                }

                let result: serde_json::Value = resp.json().await
                    .map_err(|e| anyhow::anyhow!("Failed to parse summarization response: {}", e))?;

                result.get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("No text in summarization response"))
            }
            LlmProvider::Codex { access_token, account_id, model } => {
                // Codex uses OpenAI-compatible endpoint
                let api_url = format!("https://chatgpt.com/backend-api/codex/v1/chat/completions");
                let body = json!({
                    "model": model,
                    "max_tokens": self.compact_config.summary_max_tokens,
                    "messages": [
                        { "role": "system", "content": SUMMARIZATION_SYSTEM_PROMPT },
                        { "role": "user", "content": prompt },
                    ],
                });
                let resp = client.post(&api_url)
                    .header("Authorization", format!("Bearer {}", access_token))
                    .header("X-Account-ID", account_id.as_str())
                    .header("content-type", "application/json")
                    .json(&body)
                    .send().await
                    .map_err(|e| anyhow::anyhow!("Summarization request failed: {}", e))?;

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Summarization API error: {}", err));
                }

                let result: serde_json::Value = resp.json().await
                    .map_err(|e| anyhow::anyhow!("Failed to parse summarization response: {}", e))?;

                result.get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("No text in summarization response"))
            }
        }
    }

    /// Push a user message, merging with the last message if it's also a user message.
    /// This avoids consecutive user messages which Anthropic API rejects.
    pub(super) fn push_user_message(messages: &mut Vec<serde_json::Value>, new_content: serde_json::Value) {
        if let Some(last) = messages.last_mut() {
            if last.get("role").and_then(|r| r.as_str()) == Some("user") {
                // Merge into existing user message
                let old_content = last.get("content").cloned();
                let merged = match (old_content, &new_content) {
                    (Some(serde_json::Value::String(old)), serde_json::Value::String(new)) => {
                        serde_json::Value::String(format!("{}\n\n{}", old, new))
                    }
                    (Some(serde_json::Value::Array(mut old_arr)), serde_json::Value::Array(new_arr)) => {
                        old_arr.extend(new_arr.iter().cloned());
                        serde_json::Value::Array(old_arr)
                    }
                    (Some(serde_json::Value::Array(mut old_arr)), serde_json::Value::String(s)) => {
                        old_arr.push(json!({"type": "text", "text": s}));
                        serde_json::Value::Array(old_arr)
                    }
                    (Some(serde_json::Value::String(old)), serde_json::Value::Array(new_arr)) => {
                        let mut arr = vec![json!({"type": "text", "text": old})];
                        arr.extend(new_arr.iter().cloned());
                        serde_json::Value::Array(arr)
                    }
                    (_, _) => new_content.clone(),
                };
                last["content"] = merged;
                return;
            }
        }
        messages.push(json!({ "role": "user", "content": new_content }));
    }
}
