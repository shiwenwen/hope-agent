use anyhow::Result;
use serde_json::json;

use super::llm_adapter::{OneShotMode, OneShotRequest};
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

        // Pre-compute cache-TTL throttle state as two booleans for CompactionContext.
        let (cache_ttl_throttled, cache_ttl_emergency) =
            if self.compact_config.cache_ttl_secs > 0 {
                let within_ttl = {
                    let guard = self
                        .last_tier2_compaction_at
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    matches!(*guard, Some(ts) if ts.elapsed().as_secs() < self.compact_config.cache_ttl_secs)
                };
                if within_ttl {
                    let tokens_now = context_compact::estimate_request_tokens(
                        system_prompt,
                        messages,
                        max_tokens,
                    );
                    let usage_now = tokens_now as f64 / self.context_window as f64;
                    let emergency = usage_now >= CACHE_TTL_EMERGENCY_RATIO;
                    if emergency {
                        app_debug!(
                            "context",
                            "compact",
                            "Cache-TTL throttle overridden: usage {:.1}% >= {:.0}%, forcing Tier 2+",
                            usage_now * 100.0,
                            CACHE_TTL_EMERGENCY_RATIO * 100.0
                        );
                    } else {
                        app_debug!(
                            "context",
                            "compact",
                            "Cache-TTL throttle: skipping Tier 2+ (cache still hot)"
                        );
                    }
                    (true, emergency)
                } else {
                    (false, false)
                }
            } else {
                (false, false)
            };

        let ctx = context_compact::CompactionContext {
            system_prompt,
            context_window: self.context_window,
            max_output_tokens: max_tokens,
            config: &self.compact_config,
            cache_ttl_throttled,
            cache_ttl_emergency,
        };
        let compact_result = self.context_engine.compact_sync(messages, &ctx);

        if compact_result.tier_applied == 0 {
            return;
        }

        // Touch timer after synchronous Tier 2 completes.
        // Tier 3 touches the timer separately in its own success path (after async LLM call).
        if compact_result.tier_applied == 2 {
            self.touch_compaction_timer();
        }

        // Tier 2+ already invalidated the prompt cache; piggyback and force
        // an awareness suffix rebuild on the next turn at zero extra cost.
        // Respect the per-session `refresh_on_compaction` flag.
        if compact_result.tier_applied >= 2 {
            let should_piggyback = self
                .awareness
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|a| {
                    a.cfg
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .refresh_on_compaction
                })
                .unwrap_or(true);
            if should_piggyback {
                self.force_refresh_awareness();
            }
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
                            // Insert immediately after the summary message emitted by
                            // `apply_summary`. The summary is always at index 0 (clear →
                            // push summary → extend preserved), so index 1 is the first
                            // preserved slot. Compute from `len()` so that if the summary
                            // layout ever changes we fail loudly instead of silently
                            // misplacing the recovery content.
                            let insert_at = context_compact::POST_SUMMARY_INSERT_INDEX
                                .min(messages.len());
                            messages.insert(insert_at, recovery_msg);
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
    /// If a CompactionProvider is configured, tries it first; on failure falls back
    /// to side_query (prompt cache sharing) or direct HTTP call.
    async fn summarize_with_model(&self, prompt: &str) -> Result<String> {
        use crate::context_compact::SUMMARIZATION_SYSTEM_PROMPT;

        // Try pluggable CompactionProvider first (if configured)
        if let Some(ref provider) = self.compaction_provider {
            app_info!(
                "agent",
                "summarize",
                "Trying CompactionProvider '{}' for Tier 3 summarization",
                provider.name()
            );
            match provider
                .summarize(prompt, self.compact_config.summary_max_tokens)
                .await
            {
                Ok(summary) if !summary.is_empty() => return Ok(summary),
                Ok(_) => {
                    app_warn!(
                        "agent",
                        "summarize",
                        "CompactionProvider '{}' returned empty summary, falling back to conversation model",
                        provider.name()
                    );
                }
                Err(e) => {
                    app_warn!(
                        "agent",
                        "summarize",
                        "CompactionProvider '{}' failed: {}, falling back to conversation model",
                        provider.name(),
                        e
                    );
                }
            }
        }

        // Try cache-friendly side_query path
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
        summarize_direct(&self.provider, &self.user_agent, prompt, self.compact_config.summary_max_tokens).await
    }

    /// Build an LlmProvider from a ProviderConfig and model ID.
    pub(crate) fn build_llm_provider(config: &crate::provider::ProviderConfig, model_id: &str) -> LlmProvider {
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

    /// Build an LlmProvider for a specific auth profile. Used by the
    /// `execute_with_failover` closure in `side_query` and
    /// `DedicatedModelProvider::summarize` to rebuild the provider after
    /// the executor selects a profile to try.
    ///
    /// `profile = None` falls back to `config.api_key` + `config.base_url`
    /// (Codex / OAuth path where `effective_profiles()` is empty).
    pub(crate) fn build_llm_provider_with_profile(
        config: &crate::provider::ProviderConfig,
        model_id: &str,
        profile: Option<&crate::provider::AuthProfile>,
    ) -> LlmProvider {
        use crate::provider::ApiType;
        let (api_key, base_url) = match profile {
            Some(p) => (p.api_key.clone(), config.resolve_base_url(p).to_string()),
            None => (config.api_key.clone(), config.base_url.clone()),
        };
        match config.api_type {
            ApiType::Anthropic => LlmProvider::Anthropic {
                api_key,
                base_url,
                model: model_id.to_string(),
            },
            ApiType::OpenaiChat => LlmProvider::OpenAIChat {
                api_key,
                base_url,
                model: model_id.to_string(),
            },
            ApiType::OpenaiResponses => LlmProvider::OpenAIResponses {
                api_key,
                base_url,
                model: model_id.to_string(),
            },
            ApiType::Codex => LlmProvider::Codex {
                access_token: api_key,
                account_id: String::new(),
                model: model_id.to_string(),
            },
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
                        // String-content role message (typically Chat Completions shape).
                        // Responses API rejects Chat-only fields (`tool_calls`,
                        // `tool_call_id`) and the `tool` role — it uses separate
                        // `function_call` / `function_call_output` input items instead.
                        // Skip the Chat tool-result message entirely; strip the tool
                        // payload off assistant messages and keep their natural-language
                        // content (drop if nothing remains).
                        if item.get("role").and_then(|r| r.as_str()) == Some("tool") {
                            continue;
                        }
                        let mut msg = item.clone();
                        if let Some(obj) = msg.as_object_mut() {
                            obj.remove("reasoning_content");
                            obj.remove("tool_calls");
                            obj.remove("tool_call_id");
                        }
                        let has_content = msg
                            .get("content")
                            .map(|c| match c {
                                serde_json::Value::String(s) => !s.is_empty(),
                                serde_json::Value::Array(a) => !a.is_empty(),
                                _ => false,
                            })
                            .unwrap_or(false);
                        if !has_content {
                            continue;
                        }
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

// ── Standalone summarization helpers ─────────────────────────────────

/// Direct one-shot summarization call (decoupled from AssistantAgent).
/// Used by both the default fallback path and `DedicatedModelProvider`.
///
/// Routes through [`super::llm_adapter::LlmApiAdapter`] so all four providers
/// share one body builder per protocol — no more 4-branch HTTP duplication.
pub(crate) async fn summarize_direct(
    provider: &LlmProvider,
    user_agent: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<String> {
    use crate::context_compact::SUMMARIZATION_SYSTEM_PROMPT;

    let client = crate::provider::apply_proxy(reqwest::Client::builder().user_agent(user_agent))
        .build()
        .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

    let result = provider
        .as_adapter()
        .one_shot(
            &client,
            OneShotRequest {
                instruction: prompt,
                max_tokens,
                mode: OneShotMode::Independent {
                    system: SUMMARIZATION_SYSTEM_PROMPT,
                },
            },
        )
        .await?;

    if result.text.is_empty() {
        return Err(anyhow::anyhow!("No text in summarization response"));
    }
    Ok(result.text)
}

// ── DedicatedModelProvider ───────────────────────────────────────────

/// Dedicated model provider for Tier 3 summarization.
/// Uses a specific provider/model pair, independent of the main conversation.
///
/// Holds an `Arc<ProviderConfig>` + `model_id` + `session_id` so each
/// `summarize()` call can route through `failover::execute_with_failover`
/// for retry-with-backoff against the configured `summarization_model`'s
/// own auth profiles. Profile rotation is intentionally **disabled** by
/// [`FailoverPolicy::summarize_default`] — Tier 3 must fail fast so the
/// caller can drop to side_query / emergency_compact.
pub(crate) struct DedicatedModelProvider {
    provider_config: std::sync::Arc<crate::provider::ProviderConfig>,
    model_id: String,
    session_id: String,
    user_agent: String,
    display_name: String,
}

impl DedicatedModelProvider {
    pub(crate) fn new(
        provider_config: std::sync::Arc<crate::provider::ProviderConfig>,
        model_id: String,
        session_id: String,
        user_agent: String,
        display_name: String,
    ) -> Self {
        Self {
            provider_config,
            model_id,
            session_id,
            user_agent,
            display_name,
        }
    }
}

#[async_trait::async_trait]
impl crate::context_compact::CompactionProvider for DedicatedModelProvider {
    async fn summarize(&self, prompt: &str, max_tokens: u32) -> anyhow::Result<String> {
        use crate::failover::executor::{
            execute_with_failover, ExecutorError, FailoverPolicy,
        };

        let provider_config = self.provider_config.as_ref();
        let model_id = self.model_id.as_str();
        let user_agent = self.user_agent.as_str();

        let exec_result = execute_with_failover(
            provider_config,
            &self.session_id,
            FailoverPolicy::summarize_default(),
            None,
            |profile| {
                let provider = AssistantAgent::build_llm_provider_with_profile(
                    provider_config,
                    model_id,
                    profile,
                );
                async move { summarize_direct(&provider, user_agent, prompt, max_tokens).await }
            },
        )
        .await;

        match exec_result {
            Ok(text) => Ok(text),
            Err(ExecutorError::NeedsCompaction { .. }) => Err(anyhow::anyhow!(
                "dedicated summarize: context overflow"
            )),
            Err(ExecutorError::Exhausted {
                last_reason,
                last_error,
            }) => Err(anyhow::anyhow!(
                "dedicated summarize exhausted ({:?}): {}",
                last_reason,
                last_error
            )),
            Err(ExecutorError::NoProfileAvailable) => Err(anyhow::anyhow!(
                "dedicated summarize: no auth profile available"
            )),
        }
    }

    fn name(&self) -> &str {
        &self.display_name
    }
}

/// Parse `"providerId:modelId"` and construct a `DedicatedModelProvider`.
/// Returns `None` (with a warning log) if the format is invalid or the provider is not found/disabled.
///
/// `session_id` is used as the failover sticky/cooldown key so summarize
/// cooldowns are scoped to one session (and inherit cross-call sticky
/// affinity within that session).
pub(crate) fn build_compaction_provider(
    model_ref: &str,
    providers: &[crate::provider::ProviderConfig],
    session_id: &str,
) -> Option<DedicatedModelProvider> {
    let (provider_id, model_id) = match model_ref.split_once(':') {
        Some(pair) => pair,
        None => {
            app_warn!(
                "agent",
                "compaction_provider",
                "Invalid summarization_model format '{}' (expected 'providerId:modelId')",
                model_ref
            );
            return None;
        }
    };

    let prov_config = crate::provider::find_provider(providers, provider_id)?;
    let display_name = format!("{}:{}", prov_config.name, model_id);

    Some(DedicatedModelProvider::new(
        std::sync::Arc::new(prov_config.clone()),
        model_id.to_string(),
        session_id.to_string(),
        prov_config.user_agent.clone(),
        display_name,
    ))
}
