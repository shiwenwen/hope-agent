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
    /// If flush_before_compact is enabled, extracts memories from messages before they are summarized.
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
                "info",
                "context",
                "compact",
                &format!(
                    "Context compacted: tier={}, {} → {} tokens, {} messages affected",
                    compact_result.tier_applied,
                    compact_result.tokens_before,
                    compact_result.tokens_after,
                    compact_result.messages_affected,
                ),
                None,
                None,
                None,
            );
        }

        // Tier 3: LLM summarization needed
        if compact_result.description == "summarization_needed" {
            if let Some(split) =
                context_compact::split_for_summarization(messages, &self.compact_config)
            {
                // Memory Flush: extract memories from messages about to be summarized
                {
                    let flush_enabled = {
                        let global = crate::memory::load_extract_config();
                        let agent_flush = crate::agent_loader::load_agent(&self.agent_id)
                            .ok()
                            .and_then(|d| d.config.memory.flush_before_compact);
                        agent_flush.unwrap_or(global.flush_before_compact)
                    };

                    if flush_enabled {
                        // Resolve provider config on the current thread before spawning
                        let flush_provider = crate::provider::load_store()
                            .ok()
                            .and_then(|s| s.providers.first().cloned());

                        if let Some(prov) = flush_provider {
                            if let Some(model) = prov.models.first().cloned() {
                                let agent_id = self.agent_id.clone();
                                let session_id = self.session_id.clone().unwrap_or_default();
                                let msgs = split.summarizable.clone();
                                let model_id = model.id.clone();

                                // Use a new tokio runtime on a background thread to avoid
                                // Send bounds issues with the parent async context.
                                std::thread::spawn(move || {
                                    let rt = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build();
                                    if let Ok(rt) = rt {
                                        let result = rt.block_on(async {
                                            tokio::time::timeout(
                                                std::time::Duration::from_secs(30),
                                                crate::memory_extract::flush_before_compact(
                                                    &msgs,
                                                    &agent_id,
                                                    &session_id,
                                                    &prov,
                                                    &model_id,
                                                ),
                                            )
                                            .await
                                        });
                                        match result {
                                            Ok(Ok(count)) if count > 0 => {
                                                app_info!(
                                                    "memory",
                                                    "flush",
                                                    "Flushed {} memories before compaction",
                                                    count
                                                );
                                            }
                                            Ok(Err(e)) => {
                                                app_warn!(
                                                    "memory",
                                                    "flush",
                                                    "Memory flush failed: {}",
                                                    e
                                                );
                                            }
                                            Err(_) => {
                                                app_warn!(
                                                    "memory",
                                                    "flush",
                                                    "Memory flush timed out (30s)"
                                                );
                                            }
                                            _ => {}
                                        }
                                    }
                                });
                            }
                        }
                    }
                }

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
                )
                .await
                {
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
                                "warn",
                                "context",
                                "compact",
                                &format!("Tier 3 summarization failed: {}", e),
                                None,
                                None,
                                None,
                            );
                        }
                    }
                    Err(_) => {
                        if let Some(logger) = crate::get_logger() {
                            logger.log(
                                "warn",
                                "context",
                                "compact",
                                &format!(
                                    "Tier 3 summarization timed out after {}s",
                                    self.compact_config.summarization_timeout_secs
                                ),
                                None,
                                None,
                                None,
                            );
                        }
                    }
                }
            }
        }

        // Emit compaction event to frontend
        let tokens_after =
            context_compact::estimate_request_tokens(system_prompt, messages, max_tokens);
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

        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        match &self.provider {
            LlmProvider::Anthropic {
                api_key,
                base_url,
                model,
            } => {
                let api_url = build_api_url(base_url, "/v1/messages");
                let body = json!({
                    "model": model,
                    "max_tokens": self.compact_config.summary_max_tokens,
                    "system": SUMMARIZATION_SYSTEM_PROMPT,
                    "messages": [{ "role": "user", "content": prompt }],
                });
                let resp = client
                    .post(&api_url)
                    .header("x-api-key", api_key)
                    .header("anthropic-version", ANTHROPIC_API_VERSION)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Summarization request failed: {}", e))?;

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Summarization API error: {}", err));
                }

                let result: serde_json::Value = resp.json().await.map_err(|e| {
                    anyhow::anyhow!("Failed to parse summarization response: {}", e)
                })?;

                // Extract text from Anthropic response
                result
                    .get("content")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    })
                    .and_then(|b| b.get("text"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("No text in summarization response"))
            }
            LlmProvider::OpenAIChat {
                api_key,
                base_url,
                model,
            }
            | LlmProvider::OpenAIResponses {
                api_key,
                base_url,
                model,
            } => {
                let api_url = build_api_url(base_url, "/v1/chat/completions");
                let body = json!({
                    "model": model,
                    "max_tokens": self.compact_config.summary_max_tokens,
                    "messages": [
                        { "role": "system", "content": SUMMARIZATION_SYSTEM_PROMPT },
                        { "role": "user", "content": prompt },
                    ],
                });
                let resp = client
                    .post(&api_url)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Summarization request failed: {}", e))?;

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Summarization API error: {}", err));
                }

                let result: serde_json::Value = resp.json().await.map_err(|e| {
                    anyhow::anyhow!("Failed to parse summarization response: {}", e)
                })?;

                result
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("No text in summarization response"))
            }
            LlmProvider::Codex {
                access_token,
                account_id,
                model,
            } => {
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
                let resp = client
                    .post(&api_url)
                    .header("Authorization", format!("Bearer {}", access_token))
                    .header("X-Account-ID", account_id.as_str())
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Summarization request failed: {}", e))?;

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Summarization API error: {}", err));
                }

                let result: serde_json::Value = resp.json().await.map_err(|e| {
                    anyhow::anyhow!("Failed to parse summarization response: {}", e)
                })?;

                result
                    .get("choices")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("No text in summarization response"))
            }
        }
    }

    /// Normalize conversation history for Anthropic Messages API.
    /// Converts foreign format items (Responses API / Chat Completions) to Anthropic format.
    pub(super) fn normalize_history_for_anthropic(
        history: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        for item in history {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                // Skip OpenAI Responses reasoning items (encrypted, Anthropic can't use them)
                "reasoning" => continue,
                // Skip Responses API tool items (Anthropic uses tool_use/tool_result)
                "function_call" | "function_call_output" => continue,
                // Convert Responses API message format to Anthropic format
                "message" => {
                    let role = item
                        .get("role")
                        .and_then(|r| r.as_str())
                        .unwrap_or("assistant");
                    if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                        let text: String = parts
                            .iter()
                            .filter(|p| {
                                p.get("type").and_then(|t| t.as_str()) == Some("output_text")
                            })
                            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            result.push(json!({ "role": role, "content": text }));
                        }
                    }
                }
                _ => {
                    // Standard role-based messages — pass through, but strip reasoning_content
                    let mut msg = item.clone();
                    if msg.get("reasoning_content").is_some() {
                        // Convert Chat API reasoning_content to Anthropic thinking block
                        if let Some(reasoning) =
                            msg.get("reasoning_content").and_then(|r| r.as_str())
                        {
                            if !reasoning.is_empty() {
                                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                                    // Convert string content + reasoning to content array with thinking block
                                    msg["content"] = json!([
                                        { "type": "thinking", "thinking": reasoning },
                                        { "type": "text", "text": content }
                                    ]);
                                }
                            }
                        }
                        msg.as_object_mut().map(|o| o.remove("reasoning_content"));
                    }
                    result.push(msg);
                }
            }
        }
        result
    }

    /// Normalize conversation history for OpenAI Chat Completions API.
    /// Converts foreign format items (Responses API / Anthropic) to Chat format.
    pub(super) fn normalize_history_for_chat(
        history: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        for item in history {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                // Skip OpenAI Responses reasoning items
                "reasoning" => continue,
                // Skip Responses API tool items (Chat uses tool_calls array)
                "function_call" | "function_call_output" => continue,
                // Convert Responses API message format to Chat format
                "message" => {
                    let role = item
                        .get("role")
                        .and_then(|r| r.as_str())
                        .unwrap_or("assistant");
                    if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                        let text: String = parts
                            .iter()
                            .filter(|p| {
                                p.get("type").and_then(|t| t.as_str()) == Some("output_text")
                            })
                            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            result.push(json!({ "role": role, "content": text }));
                        }
                    }
                }
                _ => {
                    // Standard role-based messages — handle Anthropic content arrays
                    if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                        // Anthropic format: content is array of blocks
                        let has_tool_use = content_arr
                            .iter()
                            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));
                        if has_tool_use {
                            // Pass through Anthropic tool messages as-is (already role-based)
                            result.push(item.clone());
                        } else {
                            // Extract text and thinking from Anthropic content blocks
                            let mut thinking = String::new();
                            let mut text = String::new();
                            for block in content_arr {
                                let block_type =
                                    block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                match block_type {
                                    "thinking" => {
                                        if let Some(t) =
                                            block.get("thinking").and_then(|t| t.as_str())
                                        {
                                            thinking.push_str(t);
                                        }
                                    }
                                    "text" => {
                                        if let Some(t) = block.get("text").and_then(|t| t.as_str())
                                        {
                                            text.push_str(t);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let role = item
                                .get("role")
                                .and_then(|r| r.as_str())
                                .unwrap_or("assistant");
                            if !text.is_empty() || !thinking.is_empty() {
                                let content = if text.is_empty() { &thinking } else { &text };
                                let mut msg = json!({ "role": role, "content": content });
                                if !thinking.is_empty() && !text.is_empty() {
                                    msg["reasoning_content"] = json!(&thinking);
                                }
                                result.push(msg);
                            }
                        }
                    } else {
                        // String content or other — pass through
                        result.push(item.clone());
                    }
                }
            }
        }
        result
    }

    /// Normalize conversation history for OpenAI Responses API.
    /// Converts foreign format items (Anthropic / Chat) to Responses input format.
    /// The Responses API is flexible and accepts both `{ "role": "...", "content": "..." }`
    /// and `{ "type": "message", ... }` formats, so we mainly need to strip incompatible items.
    pub(super) fn normalize_history_for_responses(
        history: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        for item in history {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                // Native Responses API items — pass through
                "reasoning" | "message" | "function_call" | "function_call_output" => {
                    result.push(item.clone());
                }
                _ => {
                    // Role-based messages (from Anthropic/Chat)
                    if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                        // Anthropic format: extract text from content blocks, skip thinking/tool blocks
                        let has_tool_use = content_arr
                            .iter()
                            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));
                        let has_tool_result = content_arr
                            .iter()
                            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
                        if has_tool_use || has_tool_result {
                            // Skip Anthropic tool messages (Responses API uses function_call format)
                            continue;
                        }
                        let text: String = content_arr
                            .iter()
                            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        let role = item
                            .get("role")
                            .and_then(|r| r.as_str())
                            .unwrap_or("assistant");
                        if !text.is_empty() {
                            result.push(json!({ "role": role, "content": text }));
                        }
                    } else {
                        // String content or tool role messages — pass through (Responses accepts them)
                        let mut msg = item.clone();
                        // Strip reasoning_content (not part of Responses API)
                        msg.as_object_mut().map(|o| o.remove("reasoning_content"));
                        result.push(msg);
                    }
                }
            }
        }
        result
    }

    /// Push a user message, merging with the last message if it's also a user message.
    /// This avoids consecutive user messages which Anthropic API rejects.
    pub(super) fn push_user_message(
        messages: &mut Vec<serde_json::Value>,
        new_content: serde_json::Value,
    ) {
        if let Some(last) = messages.last_mut() {
            if last.get("role").and_then(|r| r.as_str()) == Some("user") {
                // Merge into existing user message
                let old_content = last.get("content").cloned();
                let merged = match (old_content, &new_content) {
                    (Some(serde_json::Value::String(old)), serde_json::Value::String(new)) => {
                        serde_json::Value::String(format!("{}\n\n{}", old, new))
                    }
                    (
                        Some(serde_json::Value::Array(mut old_arr)),
                        serde_json::Value::Array(new_arr),
                    ) => {
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
