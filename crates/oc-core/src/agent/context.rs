use anyhow::Result;
use serde_json::json;

use super::config::{build_api_url, ANTHROPIC_API_VERSION};
use super::types::{AssistantAgent, LlmProvider};

impl AssistantAgent {
    /// Replace the conversation history (used to restore context from DB).
    pub fn set_conversation_history(&self, history: Vec<serde_json::Value>) {
        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = history;
    }

    /// Get a clone of the current conversation history (used to persist context to DB).
    pub fn get_conversation_history(&self) -> Vec<serde_json::Value> {
        self.conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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

        /// Usage ratio that overrides cache-TTL throttle to prevent ContextOverflow → Tier 4.
        const CACHE_TTL_EMERGENCY_RATIO: f64 = 0.95;

        // Cache-TTL throttle: skip Tier 2+ if last compaction was recent.
        // Only clone config when we need to mutate thresholds; borrow otherwise.
        let mut owned_config;
        let effective_config = if self.compact_config.cache_ttl_secs > 0 {
            let within_ttl = {
                let guard = self
                    .last_tier2_compaction_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                matches!(*guard, Some(ts) if ts.elapsed().as_secs() < self.compact_config.cache_ttl_secs)
            };
            if within_ttl {
                let tokens_now =
                    context_compact::estimate_request_tokens(system_prompt, messages, max_tokens);
                let usage_now = tokens_now as f64 / self.context_window as f64;
                if usage_now >= CACHE_TTL_EMERGENCY_RATIO {
                    app_debug!(
                        "context",
                        "compact",
                        "Cache-TTL throttle overridden: usage {:.1}% >= {:.0}%, forcing Tier 2+",
                        usage_now * 100.0,
                        CACHE_TTL_EMERGENCY_RATIO * 100.0
                    );
                    &self.compact_config
                } else {
                    owned_config = self.compact_config.clone();
                    owned_config.soft_trim_ratio = f64::INFINITY;
                    owned_config.hard_clear_ratio = f64::INFINITY;
                    owned_config.summarization_threshold = f64::INFINITY;
                    app_debug!(
                        "context",
                        "compact",
                        "Cache-TTL throttle: skipping Tier 2+ (cache still hot)"
                    );
                    &owned_config
                }
            } else {
                &self.compact_config
            }
        } else {
            &self.compact_config
        };

        let compact_result = context_compact::compact_if_needed(
            messages,
            system_prompt,
            self.context_window,
            max_tokens,
            effective_config,
        );

        if compact_result.tier_applied == 0 {
            return;
        }

        // Touch timer after synchronous Tier 2 completes.
        // Tier 3 touches the timer separately in its own success path (after async LLM call).
        if compact_result.tier_applied == 2 {
            self.touch_compaction_timer();
        }

        // Tier 2+ already invalidated the prompt cache; piggyback and force
        // a cross-session suffix rebuild on the next turn at zero extra cost.
        if compact_result.tier_applied >= 2 {
            self.force_refresh_cross_session();
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
                        let flush_provider =
                            crate::config::cached_config().providers.first().cloned();

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
                        // Update cache-TTL timer after successful Tier 3 summarization
                        self.touch_compaction_timer();
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

                        // Post-compaction file recovery: re-inject recently-edited file contents
                        let tokens_after_summary = context_compact::estimate_request_tokens(
                            system_prompt,
                            messages,
                            max_tokens,
                        );
                        let tokens_freed = compact_result
                            .tokens_before
                            .saturating_sub(tokens_after_summary);
                        if let Some(recovery_msg) = context_compact::build_recovery_message(
                            &split.summarizable,
                            &split.preserved,
                            tokens_freed,
                            &self.compact_config,
                        ) {
                            // Insert after summary message (index 0), before preserved messages
                            messages.insert(1, recovery_msg);
                            app_info!(
                                "context",
                                "compact",
                                "Post-compaction recovery: injected file contents after summary"
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
    /// If a custom summarization model is configured, uses that model directly (no cache sharing).
    /// Otherwise prefers side_query() when cache-safe params are available (prompt cache sharing),
    /// falls back to direct HTTP call otherwise.
    async fn summarize_with_model(&self, prompt: &str) -> Result<String> {
        use crate::context_compact::SUMMARIZATION_SYSTEM_PROMPT;

        // Check for custom summarization model override
        if let Some(ref model_ref) = self.compact_config.summarization_model {
            if let Some((provider_id, model_id)) = model_ref.split_once(':') {
                let store = crate::config::cached_config();
                if let Some(provider_config) = store
                    .providers
                    .iter()
                    .find(|p| p.id == provider_id && p.enabled)
                {
                    let provider = Self::build_llm_provider(provider_config, model_id);
                    app_info!(
                        "agent",
                        "summarize",
                        "Using custom summarization model: {} ({}:{})",
                        provider_config.name,
                        provider_id,
                        model_id
                    );
                    return self.summarize_with_provider_direct(&provider, prompt).await;
                }
                app_warn!(
                    "agent",
                    "summarize",
                    "Custom summarization provider '{}' not found or disabled, falling back to conversation model",
                    provider_id
                );
            } else {
                app_warn!(
                    "agent",
                    "summarize",
                    "Invalid summarization_model format '{}' (expected 'providerId:modelId'), falling back to conversation model",
                    model_ref
                );
            }
        }

        // Try cache-friendly side_query path first
        let has_cache = self
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();

        if has_cache {
            let instruction = format!(
                "<summarization_instructions>\n{}\n</summarization_instructions>\n\n{}",
                SUMMARIZATION_SYSTEM_PROMPT, prompt
            );
            let result = self
                .side_query(&instruction, self.compact_config.summary_max_tokens)
                .await?;

            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "info",
                    "agent",
                    "side_query::summarize",
                    &format!(
                        "Summarization via side_query: cache_read={}, input={}, output={}",
                        result.usage.cache_read_input_tokens,
                        result.usage.input_tokens,
                        result.usage.output_tokens,
                    ),
                    None,
                    None,
                    None,
                );
            }

            if !result.text.is_empty() {
                return Ok(result.text);
            }
            app_warn!(
                "agent",
                "side_query::summarize",
                "Side query returned empty text, falling back to direct HTTP call"
            );
        }

        // Fallback: direct HTTP call (no cache sharing, used before first chat turn)
        self.summarize_with_provider_direct(&self.provider, prompt)
            .await
    }

    /// Build an LlmProvider from a ProviderConfig and model ID.
    fn build_llm_provider(config: &crate::provider::ProviderConfig, model_id: &str) -> LlmProvider {
        use crate::provider::ApiType;
        match config.api_type {
            ApiType::Anthropic => LlmProvider::Anthropic {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiChat => LlmProvider::OpenAIChat {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiResponses => LlmProvider::OpenAIResponses {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::Codex => LlmProvider::Codex {
                access_token: config.api_key.clone(),
                account_id: String::new(),
                model: model_id.to_string(),
            },
        }
    }

    /// Direct HTTP summarization call with a specific provider.
    async fn summarize_with_provider_direct(
        &self,
        provider: &LlmProvider,
        prompt: &str,
    ) -> Result<String> {
        use crate::context_compact::SUMMARIZATION_SYSTEM_PROMPT;

        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        match provider {
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
                let api_url = "https://chatgpt.com/backend-api/codex/v1/chat/completions";
                let body = json!({
                    "model": model,
                    "max_tokens": self.compact_config.summary_max_tokens,
                    "messages": [
                        { "role": "system", "content": SUMMARIZATION_SYSTEM_PROMPT },
                        { "role": "user", "content": prompt },
                    ],
                });
                let resp = client
                    .post(api_url)
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
