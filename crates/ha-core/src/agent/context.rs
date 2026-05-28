use anyhow::Result;
use serde_json::json;

use super::llm_adapter::{OneShotMode, OneShotRequest};
use super::types::{AssistantAgent, LlmProvider};

/// Count tool-use signals in a single conversation-history item across
/// all three provider shapes. See `AssistantAgent::history_tail_stats`.
fn count_tool_uses(msg: &serde_json::Value) -> usize {
    // OpenAI Responses: top-level `{ "type": "function_call" }` item.
    if msg.get("type").and_then(|t| t.as_str()) == Some("function_call") {
        return 1;
    }
    // OpenAI Chat: assistant message with `tool_calls: [...]`.
    if let Some(arr) = msg.get("tool_calls").and_then(|v| v.as_array()) {
        if !arr.is_empty() {
            return arr.len();
        }
    }
    // Anthropic: assistant message with `content[].type == "tool_use"`.
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
            .count();
    }
    0
}

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

    /// Compute trailing-slice stats under a single lock — avoids cloning
    /// the whole history just to count messages and tool_use blocks in
    /// the post-turn hot path. Returns `(new_message_count, tool_use_count)`.
    ///
    /// Recognises all three provider history shapes:
    /// - **Anthropic**: assistant message with `content[].type == "tool_use"`
    /// - **OpenAI Chat**: assistant message with non-empty `tool_calls: []`
    /// - **OpenAI Responses**: top-level item `{ "type": "function_call" }`
    ///
    /// If you only check the Anthropic shape, OpenAI users have a
    /// permanent `tool_use_count == 0`, which collapses the skill
    /// auto-review trigger entirely under the default `require_tool_use`.
    pub fn history_tail_stats(&self, since_len: usize) -> (usize, usize) {
        let guard = self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tail = guard.get(since_len..).unwrap_or(&[]);
        let messages = tail.len();
        let tool_use = tail.iter().map(count_tool_uses).sum();
        (messages, tool_use)
    }

    /// Sync the in-flight round-loop snapshot back to `self.conversation_history`
    /// and persist it to `sessions.context_json`. Called at every round
    /// boundary so a mid-turn crash leaves all completed rounds durable.
    ///
    /// Skipped silently when:
    /// - `session_id` is empty (e.g. side-query or detached agent)
    /// - the global `SessionDB` is not initialized yet
    /// - serialization fails (logged as warn, never blocks the round)
    pub(crate) fn persist_round_context(&self, messages: &[serde_json::Value]) {
        let Some(sid) = self.session_id.as_deref() else {
            return;
        };
        let Some(db) = crate::get_session_db() else {
            return;
        };

        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages.to_vec();

        match serde_json::to_string(messages) {
            Ok(json) => {
                if let Err(e) = db.save_context(sid, &json) {
                    app_warn!(
                        "session",
                        "round_persist",
                        "save_context failed for {}: {}",
                        sid,
                        e
                    );
                }
            }
            Err(e) => {
                app_warn!(
                    "session",
                    "round_persist",
                    "serialize history failed for {}: {}",
                    sid,
                    e
                );
            }
        }
    }

    /// Run context compaction (Tier 1-3) on messages before API call.
    /// If Tier 3 summarization is needed, performs a non-streaming LLM call to summarize old messages.
    /// If flush_before_compact is enabled, extracts memories from messages before they are summarized.
    pub(super) async fn run_compaction(
        &self,
        messages: &mut Vec<serde_json::Value>,
        system_prompt: &str,
        model: &str,
        max_tokens: u32,
        on_delta: &(impl Fn(&str) + Send),
    ) {
        use crate::context_compact;

        /// Usage ratio that overrides cache-TTL throttle to prevent ContextOverflow → Tier 4.
        const CACHE_TTL_EMERGENCY_RATIO: f64 = 0.95;

        // Pre-compute cache-TTL throttle state as two booleans for CompactionContext.
        let (cache_ttl_throttled, cache_ttl_emergency) = if self.compact_config.cache_ttl_secs > 0 {
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

        // PreCompact hook (blocking; design §5.3.1). A hook may `block` to skip
        // this compaction — but a fill ratio ≥ 0.95 forces it anyway, since
        // skipping would let the request overflow the context window. Gate is
        // multi-scope (project/local hooks for this session's working dir too).
        let precompact_wd =
            crate::session::effective_session_working_dir(self.session_id.as_deref());
        if crate::hooks::scopes::any_handlers_for(
            crate::hooks::HookEvent::PreCompact,
            precompact_wd.as_deref().map(std::path::Path::new),
        ) {
            let tokens_now =
                context_compact::estimate_request_tokens(system_prompt, messages, max_tokens);
            let usage_now = tokens_now as f64 / self.context_window.max(1) as f64;
            // `run_compaction` runs every turn but is a no-op far below the
            // reactive trigger — only consult the PreCompact hook when a
            // compaction is actually plausible, so it precedes a real
            // compaction instead of firing every idle turn.
            let sid = self.session_id.clone().unwrap_or_default();
            if usage_now >= self.compact_config.reactive_trigger_ratio {
                let input = crate::hooks::HookInput::PreCompact {
                    common: self.hook_common_input("PreCompact"),
                    trigger: crate::hooks::CompactTrigger::Auto,
                    usage_ratio: usage_now.min(1.0),
                };
                let outcome = crate::hooks::HookDispatcher::dispatch(
                    crate::hooks::HookEvent::PreCompact,
                    input,
                )
                .await;
                // A blocking decision OR an explicit `continue:false` from any
                // hook stops the compaction (same emergency-override band as a
                // block). Aggregating both here keeps the gate aligned with the
                // dispatcher's `outcome.continue_execution` fold.
                let blocked = matches!(
                    outcome.decision,
                    crate::hooks::HookDecision::Deny { .. }
                        | crate::hooks::HookDecision::Block { .. }
                ) || !outcome.continue_execution;
                if blocked {
                    if usage_now >= CACHE_TTL_EMERGENCY_RATIO {
                        app_warn!(
                        "hooks",
                        "dispatch",
                        "PreCompact block overridden: usage {:.1}% >= {:.0}%, compacting anyway",
                        usage_now * 100.0,
                        CACHE_TTL_EMERGENCY_RATIO * 100.0
                    );
                        crate::hooks::reset_precompact_blocks(&sid);
                    } else if crate::hooks::honor_precompact_block(&sid) {
                        app_info!(
                            "hooks",
                            "dispatch",
                            "PreCompact hook blocked compaction (usage {:.1}%)",
                            usage_now * 100.0
                        );
                        return;
                    } else {
                        // Consecutive-block cap exceeded: a hook can't defer
                        // compaction forever while usage sits in the band.
                        app_warn!(
                            "hooks",
                            "dispatch",
                            "PreCompact block overridden after repeated blocks (usage {:.1}%), compacting anyway",
                            usage_now * 100.0
                        );
                    }
                } else {
                    crate::hooks::reset_precompact_blocks(&sid);
                }
            } else {
                // Usage fell back below the trigger band — clear any block streak.
                crate::hooks::reset_precompact_blocks(&sid);
            }
        }

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
                    } && !self.session_is_incognito();

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
                            let insert_at =
                                context_compact::POST_SUMMARY_INSERT_INDEX.min(messages.len());
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

        // PostCompact + SessionStart(compact) hooks (observation): fire after a
        // real compaction (tier ≥ 2; tier 0 returned early above). Queues any
        // additionalContext for the next round's reminder suffix.
        self.fire_compaction_hooks(compact_result.tier_applied, tokens_after, model)
            .await;

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

    /// Append hook-injected context to the pending queue, drained into the next
    /// round's reminder suffix. ArcSwap `rcu` so no `&mut self` is needed.
    pub(super) fn push_pending_hook_context(&self, ctx: String) {
        if ctx.trim().is_empty() {
            return;
        }
        self.pending_hook_context.rcu(|cur| {
            let mut v = Vec::with_capacity(cur.len() + 1);
            v.extend(cur.iter().cloned());
            v.push(ctx.clone());
            v
        });
    }

    /// Take and clear the pending hook context, joined into one block.
    pub(super) fn drain_pending_hook_context(&self) -> Option<String> {
        let taken = self
            .pending_hook_context
            .swap(std::sync::Arc::new(Vec::new()));
        if taken.is_empty() {
            None
        } else {
            Some(taken.join("\n\n"))
        }
    }

    /// Build common hook-input fields from agent-level state, for hooks that
    /// fire outside a tool context (compaction, etc.). `cwd` is the session
    /// working dir (falling back to home); `permission_mode` defaults.
    pub(super) fn hook_common_input(&self, event: &str) -> crate::hooks::CommonHookInput {
        let session_id = self.session_id.clone().unwrap_or_default();
        // Empty session_id (a session-less agent) → no transcript path, rather
        // than a bogus shared `sessions/transcript.jsonl` (mirrors the guard in
        // hooks::observation_common).
        let transcript_path = if session_id.is_empty() {
            std::path::PathBuf::default()
        } else {
            crate::paths::session_dir(&session_id)
                .map(|d| d.join("transcript.jsonl"))
                .unwrap_or_default()
        };
        let cwd = crate::session::effective_session_working_dir(self.session_id.as_deref())
            .map(std::path::PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        crate::hooks::CommonHookInput {
            session_id,
            transcript_path,
            cwd,
            permission_mode: crate::hooks::PermissionMode::Default,
            hook_event_name: event.to_string(),
            agent_id: Some(self.agent_id.clone()),
            agent_type: None,
        }
    }

    /// Fire `PostCompact` + `SessionStart(source=compact)` after a real
    /// compaction. Both observation events; any `additionalContext` they return
    /// is queued for the next round's reminder suffix.
    async fn fire_compaction_hooks(&self, tier: u8, tokens_after: u32, model: &str) {
        use crate::hooks::{HookDispatcher, HookEvent, HookInput};

        // Failover rebuilds the agent and re-runs compaction per retry from the
        // same history (identical tier + tokens_after → identical key, deduped);
        // a genuinely distinct compaction differs in tier or tokens_after and
        // fires even within the window.
        let sid = self.session_id.clone().unwrap_or_default();
        let dedup_key = format!("{sid}:{tier}:{tokens_after}");
        if !crate::hooks::claim_compaction_hooks(&dedup_key) {
            return;
        }

        // `usage_ratio` is the post-compaction context *fill* ratio (tokens /
        // window), matching the protocol field hooks branch on (design §5.3.1,
        // the same ≥0.95 metric that forces compaction) — not a before/after
        // compression ratio. Clamped to [0,1] so a hook expecting a ratio never
        // sees >1.0 when an estimate (incl. the output reservation) overshoots.
        let usage_ratio = if self.context_window > 0 {
            (tokens_after as f64 / self.context_window as f64).min(1.0)
        } else {
            0.0
        };

        let post = HookInput::PostCompact {
            common: self.hook_common_input("PostCompact"),
            trigger: crate::hooks::CompactTrigger::Auto,
            tier,
            usage_ratio,
        };
        let out = HookDispatcher::dispatch(HookEvent::PostCompact, post).await;
        if let Some(extra) = out.merged_additional_context() {
            self.push_pending_hook_context(extra);
        }

        let start = HookInput::SessionStart {
            common: self.hook_common_input("SessionStart"),
            source: crate::hooks::SessionStartSource::Compact,
            model: model.to_string(),
            agent_type: None,
        };
        let out = HookDispatcher::dispatch(HookEvent::SessionStart, start).await;
        if let Some(extra) = out.merged_additional_context() {
            self.push_pending_hook_context(extra);
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
        summarize_direct(
            &self.provider,
            &self.user_agent,
            prompt,
            self.compact_config.summary_max_tokens,
        )
        .await
    }

    /// Build `LlmProvider` from config + optional [`AuthProfile`] override.
    /// `profile = None` uses `config.api_key` / `config.base_url`.
    /// Codex ignores `profile` and loads the OAuth token from disk.
    pub(crate) async fn build_llm_provider(
        config: &crate::provider::ProviderConfig,
        model_id: &str,
        profile: Option<&crate::provider::AuthProfile>,
    ) -> anyhow::Result<LlmProvider> {
        use crate::provider::ApiType;

        if config.api_type == ApiType::Codex {
            let (access_token, account_id) = crate::oauth::load_fresh_codex_token().await?;
            return Ok(LlmProvider::Codex {
                access_token,
                account_id,
                model: model_id.to_string(),
            });
        }

        let (api_key, base_url) = match profile {
            Some(p) => (p.api_key.clone(), config.resolve_base_url(p).to_string()),
            None => (config.api_key.clone(), config.base_url.clone()),
        };
        Ok(match config.api_type {
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
            ApiType::Codex => unreachable!("Codex handled above"),
        })
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
                // Reasoning items are never replayed. Hope Agent always calls
                // the Responses API with `store: false`, which makes `rs_*`
                // ids dangling references — the server has no record of them
                // and 404s the request. Even payloads carrying
                // `encrypted_content` still get matched by id first, so the
                // safest invariant is "drop every reasoning item, every time."
                // Streamed thinking is still surfaced to the UI live; it just
                // never persists into history.
                "reasoning" => continue,
                // Native Responses API items — pass through
                "message" | "function_call" | "function_call_output" => {
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
        use crate::failover::executor::{execute_with_failover, FailoverPolicy};

        let provider_config = self.provider_config.as_ref();
        let model_id = self.model_id.as_str();
        let user_agent = self.user_agent.as_str();

        execute_with_failover(
            provider_config,
            &self.session_id,
            FailoverPolicy::summarize_default(),
            None,
            |profile| {
                // profile is `Option<&AuthProfile>`; clone to own it across
                // the `.await` inside build_llm_provider (Codex branch).
                let profile_owned = profile.cloned();
                async move {
                    let provider = AssistantAgent::build_llm_provider(
                        provider_config,
                        model_id,
                        profile_owned.as_ref(),
                    )
                    .await?;
                    summarize_direct(&provider, user_agent, prompt, max_tokens).await
                }
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("dedicated summarize: {}", e))
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

#[cfg(test)]
mod count_tool_uses_tests {
    use super::count_tool_uses;
    use serde_json::json;

    #[test]
    fn anthropic_content_block_tool_use_counted() {
        let msg = json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "ok"},
                {"type": "tool_use", "name": "exec", "input": {}},
                {"type": "tool_use", "name": "read", "input": {}},
            ],
        });
        assert_eq!(count_tool_uses(&msg), 2);
    }

    #[test]
    fn openai_chat_tool_calls_counted() {
        let msg = json!({
            "role": "assistant",
            "content": "ok",
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "exec"}},
                {"id": "c2", "type": "function", "function": {"name": "read"}},
            ],
        });
        assert_eq!(count_tool_uses(&msg), 2);
    }

    #[test]
    fn openai_responses_function_call_item_counted_as_one() {
        let msg = json!({
            "type": "function_call",
            "call_id": "c1",
            "name": "exec",
            "arguments": "{}",
        });
        assert_eq!(count_tool_uses(&msg), 1);
    }

    #[test]
    fn pure_text_message_returns_zero() {
        let msg = json!({"role": "user", "content": "hello"});
        assert_eq!(count_tool_uses(&msg), 0);
    }

    #[test]
    fn empty_tool_calls_returns_zero() {
        let msg = json!({"role": "assistant", "content": "ok", "tool_calls": []});
        assert_eq!(count_tool_uses(&msg), 0);
    }

    #[test]
    fn function_call_output_does_not_count() {
        // Output items are paired with function_call but only the
        // function_call side represents a model-emitted tool use.
        let msg = json!({"type": "function_call_output", "call_id": "c1", "output": "ok"});
        assert_eq!(count_tool_uses(&msg), 0);
    }
}

#[cfg(test)]
mod responses_history_tests {
    use super::*;
    use serde_json::json;

    // Hope Agent always calls Responses with `store: false`, where
    // any reasoning item — id-only OR with encrypted_content — is a
    // landmine for the next request. The invariant: normalize must drop
    // every `reasoning` item regardless of payload completeness.
    #[test]
    fn responses_history_drops_all_reasoning_items() {
        let history = vec![
            json!({"role": "user", "content": "hello"}),
            json!({
                "type": "reasoning",
                "id": "rs_missing",
                "summary": [],
                "status": "completed"
            }),
            json!({
                "type": "reasoning",
                "id": "rs_with_payload",
                "summary": [],
                "encrypted_content": "sealed",
                "status": "completed"
            }),
            json!({"role": "assistant", "content": "hi back"}),
        ];

        let normalized = AssistantAgent::normalize_history_for_responses(&history);

        assert!(
            normalized
                .iter()
                .all(|v| v.get("type").and_then(|t| t.as_str()) != Some("reasoning")),
            "reasoning item leaked into normalized history: {:?}",
            normalized
        );
        // user + assistant survive; both reasoning items dropped.
        assert_eq!(normalized.len(), 2);
    }
}

#[cfg(test)]
mod build_provider_tests {
    use super::*;
    use crate::provider::{ApiType, AuthProfile, ProviderConfig};

    #[tokio::test]
    async fn anthropic_builds_with_profile_overrides() {
        let mut cfg = ProviderConfig::new(
            "anthropic-test".into(),
            ApiType::Anthropic,
            "https://api.anthropic.com/".into(),
            "legacy-key".into(),
        );
        cfg.auth_profiles = vec![AuthProfile::new(
            "primary".into(),
            "profile-key".into(),
            Some("https://override.example/".into()),
        )];

        let profile = cfg.auth_profiles[0].clone();
        let provider = AssistantAgent::build_llm_provider(&cfg, "claude-3", Some(&profile))
            .await
            .expect("non-codex build must not touch disk");

        match provider {
            LlmProvider::Anthropic {
                api_key,
                base_url,
                model,
            } => {
                assert_eq!(api_key, "profile-key");
                assert_eq!(base_url, "https://override.example/");
                assert_eq!(model, "claude-3");
            }
            _ => panic!("expected Anthropic provider"),
        }
    }

    #[tokio::test]
    async fn openai_chat_falls_back_to_config_when_profile_none() {
        let cfg = ProviderConfig::new(
            "openai-test".into(),
            ApiType::OpenaiChat,
            "https://api.openai.com/".into(),
            "config-key".into(),
        );

        let provider = AssistantAgent::build_llm_provider(&cfg, "gpt-4o", None)
            .await
            .expect("non-codex build must not touch disk");

        match provider {
            LlmProvider::OpenAIChat {
                api_key, base_url, ..
            } => {
                assert_eq!(api_key, "config-key");
                assert_eq!(base_url, "https://api.openai.com/");
            }
            _ => panic!("expected OpenAIChat provider"),
        }
    }
}
