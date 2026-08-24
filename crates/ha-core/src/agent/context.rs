use anyhow::Result;
use serde_json::json;
use std::sync::{atomic::AtomicBool, Arc};

use super::llm_adapter::{OneShotMode, OneShotRequest};
use super::types::{AssistantAgent, LlmProvider};

const MID_LOOP_SUMMARY_HYSTERESIS_DELTA: f64 = 0.15;
const MID_LOOP_MAX_SUMMARY_ATTEMPTS_PER_TURN: u32 = 2;
const MID_LOOP_MIN_ROUNDS_BETWEEN_SUMMARIES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactionRunTrigger {
    Manual,
    TurnStart,
    ToolLoopCheckpoint,
}

impl CompactionRunTrigger {
    fn hook_trigger(self) -> crate::hooks::CompactTrigger {
        match self {
            Self::Manual => crate::hooks::CompactTrigger::Manual,
            Self::TurnStart => crate::hooks::CompactTrigger::Auto,
            Self::ToolLoopCheckpoint => crate::hooks::CompactTrigger::ToolLoop,
        }
    }

    fn manifest_trigger(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::TurnStart => "turn_start",
            Self::ToolLoopCheckpoint => "tool_loop",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CompactionRunOptions {
    pub trigger: CompactionRunTrigger,
    pub bypass_cache_ttl: bool,
    pub emit_start_event: bool,
    pub allow_memory_flush: bool,
    pub allow_summarization: bool,
    pub force_summary: bool,
    pub summary_reason: crate::context_compact::CompactionSummaryReason,
    /// Capacity recovery is a summary-only operation. The index names the
    /// earliest canonical message that must remain verbatim (the user turn
    /// owning the pending current tool group). Ordinary ratio-driven Tier 0/2
    /// must not run again in this mode.
    pub summary_hard_protected_start: Option<usize>,
    pub cancel: Option<Arc<AtomicBool>>,
    pub tool_schemas: Vec<serde_json::Value>,
}

impl CompactionRunOptions {
    fn turn_start(cancel: Option<Arc<AtomicBool>>) -> Self {
        Self {
            trigger: CompactionRunTrigger::TurnStart,
            bypass_cache_ttl: false,
            emit_start_event: true,
            allow_memory_flush: true,
            allow_summarization: true,
            force_summary: false,
            summary_reason: crate::context_compact::CompactionSummaryReason::HighWatermark,
            summary_hard_protected_start: None,
            cancel,
            tool_schemas: Vec::new(),
        }
    }

    fn manual() -> Self {
        Self {
            trigger: CompactionRunTrigger::Manual,
            bypass_cache_ttl: true,
            emit_start_event: true,
            allow_memory_flush: true,
            allow_summarization: true,
            force_summary: true,
            summary_reason: crate::context_compact::CompactionSummaryReason::Manual,
            summary_hard_protected_start: None,
            cancel: None,
            tool_schemas: Vec::new(),
        }
    }

    fn with_tool_schemas(mut self, tool_schemas: &[serde_json::Value]) -> Self {
        self.tool_schemas = tool_schemas.to_vec();
        self
    }
}

#[derive(Debug, Clone, Default)]
#[doc(hidden)]
pub struct CompactionRunOutcome {
    pub tier_applied: u8,
    pub changed_history: bool,
    pub summary_applied: bool,
    pub summary_timed_out: bool,
    pub summary_reason: Option<crate::context_compact::CompactionSummaryReason>,
    pub cache_decision: Option<crate::context_compact::CacheCompactionDecision>,
    pub tokens_after: u32,
    pub cancelled: bool,
    /// A recovery-state read/claim failure is a correctness barrier, not an
    /// ordinary "no compaction needed" result. The streaming caller must stop
    /// before issuing another Provider request.
    pub fatal_error: Option<String>,
    pub compact_result: Option<crate::context_compact::CompactResult>,
}

impl CompactionRunOutcome {
    fn cancelled(tokens_after: u32, changed_history: bool) -> Self {
        Self {
            tokens_after,
            cancelled: true,
            changed_history,
            ..Self::default()
        }
    }
}

#[derive(Debug, Default)]
#[doc(hidden)]
pub struct MidLoopCompactionState {
    pub summary_attempt_count: u32,
    pub last_summary_attempt_round: Option<u32>,
    pub suppress_tier3_for_turn: bool,
}

impl MidLoopCompactionState {
    fn summary_attempt_throttled(&self, round: u32) -> bool {
        self.summary_attempt_count >= MID_LOOP_MAX_SUMMARY_ATTEMPTS_PER_TURN
            || self.last_summary_attempt_round.is_some_and(|last| {
                round.saturating_sub(last) < MID_LOOP_MIN_ROUNDS_BETWEEN_SUMMARIES
            })
    }

    fn record_summary_attempt(&mut self, round: u32) {
        self.summary_attempt_count += 1;
        self.last_summary_attempt_round = Some(round);
    }
}

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

fn message_content_chars(msg: &serde_json::Value) -> usize {
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => s.len(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .map(|block| {
                block
                    .get("text")
                    .or_else(|| block.get("output"))
                    .and_then(|v| v.as_str())
                    .map(str::len)
                    .unwrap_or_else(|| block.to_string().len())
            })
            .sum(),
        Some(other) => other.to_string().len(),
        None => msg.to_string().len(),
    }
}

fn post_summary_ledger_reserve_chars(
    injection_remaining_chars: usize,
    has_live_runtime_state: bool,
    has_file_touches: bool,
) -> usize {
    if has_live_runtime_state {
        injection_remaining_chars.min(8_000)
    } else if has_file_touches {
        injection_remaining_chars.min(2_000)
    } else {
        0
    }
}

fn sync_tier_from_compact_result(result: &crate::context_compact::CompactResult) -> u8 {
    let Some(details) = result.details.as_ref() else {
        return 0;
    };
    if details.tool_results_soft_trimmed > 0 || details.tool_results_hard_cleared > 0 {
        2
    } else if details.tool_results_truncated > 0 {
        1
    } else {
        0
    }
}

fn record_manual_recovered_tool_cleanup(
    result: &mut crate::context_compact::CompactResult,
    cleanup: crate::context_compact::RecoveredToolCleanup,
) {
    if !cleanup.changed() {
        return;
    }

    result.messages_affected = result
        .messages_affected
        .saturating_add(cleanup.messages_affected());
    if let Some(details) = result.details.as_mut() {
        details.tool_results_soft_trimmed = details
            .tool_results_soft_trimmed
            .saturating_add(cleanup.image_markers_materialized);
        details.tool_results_hard_cleared = details
            .tool_results_hard_cleared
            .saturating_add(cleanup.hard_cleared);
    }
    if let Some(manifest) = result.manifest.as_mut() {
        manifest.tool_results_soft_trimmed = manifest
            .tool_results_soft_trimmed
            .saturating_add(cleanup.image_markers_materialized);
        manifest.tool_results_hard_cleared = manifest
            .tool_results_hard_cleared
            .saturating_add(cleanup.hard_cleared);
        if cleanup.hard_cleared > 0 {
            manifest.warnings.push(format!(
                "manual_recovered_tool_results_cleared:{}",
                cleanup.hard_cleared
            ));
        }
        if cleanup.image_markers_materialized > 0 {
            manifest.warnings.push(format!(
                "manual_recovered_image_markers_materialized:{}",
                cleanup.image_markers_materialized
            ));
        }
    }
}

impl AssistantAgent {
    fn refresh_awareness_after_prefix_rewrite(&self) {
        let should_piggyback = self
            .awareness
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|awareness| {
                awareness
                    .cfg
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .refresh_on_compaction
            })
            .unwrap_or(true);
        if should_piggyback {
            self.force_refresh_awareness();
        }
    }

    async fn pending_tier3_recovery_requirement(
        &self,
    ) -> Result<Option<crate::session::Tier3RecoveryRequirement>> {
        if self.session_is_incognito() {
            return Ok(self
                .session_id
                .as_deref()
                .and_then(crate::session::incognito_tier3_recovery_requirement));
        }
        let Some(session_id) = self.session_id.clone().filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let db = self
            .session_db
            .clone()
            .or_else(|| crate::get_session_db().cloned())
            .ok_or_else(|| anyhow::anyhow!("Tier 3 recovery state DB unavailable"))?;
        db.run(move |db| db.tier3_recovery_requirement(&session_id))
            .await
    }

    /// Claim the one automatic post-Tier-4 summary before starting its model
    /// call. A crash after this point leaves `InProgress`, so the next turn
    /// fails closed and offers manual retry instead of silently paying again.
    async fn claim_tier3_recovery_attempt(&self) -> Result<bool> {
        if self.session_is_incognito() {
            let Some(session_id) = self.session_id.as_deref() else {
                return Ok(false);
            };
            return Ok(crate::session::claim_incognito_tier3_recovery(session_id));
        }
        let Some(session_id) = self.session_id.clone().filter(|id| !id.is_empty()) else {
            return Ok(false);
        };
        let db = self
            .session_db
            .clone()
            .or_else(|| crate::get_session_db().cloned())
            .ok_or_else(|| anyhow::anyhow!("Tier 3 recovery claim DB unavailable"))?;
        db.run(move |db| db.claim_tier3_recovery_attempt(&session_id))
            .await
    }

    async fn exhaust_tier3_recovery_attempt(&self) -> Result<()> {
        if self.session_is_incognito() {
            let Some(session_id) = self.session_id.as_deref() else {
                anyhow::bail!("incognito Tier 3 recovery session is unavailable");
            };
            if !crate::session::exhaust_incognito_tier3_recovery(session_id) {
                anyhow::bail!("incognito Tier 3 recovery attempt state conflict");
            }
            return Ok(());
        }
        let Some(session_id) = self.session_id.clone().filter(|id| !id.is_empty()) else {
            anyhow::bail!("Tier 3 recovery session is unavailable");
        };
        let db = self
            .session_db
            .clone()
            .or_else(|| crate::get_session_db().cloned())
            .ok_or_else(|| anyhow::anyhow!("Tier 3 recovery finalize DB unavailable"))?;
        let finalized = db
            .run(move |db| db.exhaust_tier3_recovery_attempt(&session_id))
            .await?;
        if !finalized {
            anyhow::bail!("Tier 3 recovery attempt state conflict");
        }
        Ok(())
    }

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

    /// User-requested compaction: bypass throttles and force the Tier 3
    /// summarization path whenever there is an older prefix to summarize.
    pub async fn compact_conversation_now(
        &self,
        on_delta: &(impl Fn(&str) + Send),
    ) -> crate::context_compact::CompactResult {
        use crate::context_compact::{estimate_request_tokens, CompactResult};

        let mut canonical_history = self.get_conversation_history();
        if canonical_history.is_empty() {
            return CompactResult {
                tier_applied: 0,
                tokens_before: 0,
                tokens_after: 0,
                messages_affected: 0,
                description: "no_messages".to_string(),
                details: None,
                manifest: None,
            };
        }

        const MANUAL_COMPACT_MAX_OUTPUT_TOKENS: u32 = 16_384;
        let (provider_label, model) = self.current_model_for_compaction();
        let system_prompt = self.build_merged_system_prompt(&model, provider_label);
        let tokens_before = estimate_request_tokens(
            &system_prompt,
            &canonical_history,
            MANUAL_COMPACT_MAX_OUTPUT_TOKENS,
        );
        let mut request_projection = canonical_history.clone();

        let outcome = self
            .run_compaction_with_options(
                &mut request_projection,
                &mut canonical_history,
                &system_prompt,
                &model,
                MANUAL_COMPACT_MAX_OUTPUT_TOKENS,
                on_delta,
                CompactionRunOptions::manual(),
            )
            .await;

        // Tier 0/1/2 are request projections. Manual compaction only changes
        // the durable logical history after a Tier 3 summary is validated and
        // installed; a failed summary must not make the synchronous preview
        // edits canonical.
        if outcome.summary_applied {
            self.set_conversation_history(canonical_history);
        }

        outcome.compact_result.unwrap_or_else(|| CompactResult {
            tier_applied: outcome.tier_applied,
            tokens_before,
            tokens_after: outcome.tokens_after,
            messages_affected: 0,
            description: if outcome.cancelled {
                "cancelled".to_string()
            } else {
                "no_action_needed".to_string()
            },
            details: None,
            manifest: None,
        })
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
    /// Detached/session-less agents are the only silent no-op. A configured
    /// durability sink is fail-closed: checkpoint/CAS failure must stop the
    /// tool loop before another provider round observes an unrecorded result.
    ///
    /// Skipped when:
    /// - `session_id` is empty (e.g. side-query or detached agent)
    /// - the global `SessionDB` is not initialized yet
    #[doc(hidden)]
    pub async fn persist_round_context(&self, messages: &[serde_json::Value]) -> Result<()> {
        let Some(sid) = self.session_id.as_deref() else {
            // Detached agents have no crash-recovery contract. Installing the
            // validated summary in their in-memory canonical history is the
            // publication barrier, so do not leave a durable-publication flag
            // armed forever.
            *self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = messages.to_vec();
            self.tier3_summary_publication_pending
                .store(false, std::sync::atomic::Ordering::Release);
            return Ok(());
        };

        if let Some(sink) = self.turn_durability.as_ref() {
            let expected = sink.context_revision();
            if self.tier3_summary_publication_pending() {
                // The summary and marker removal share one SQLite checkpoint
                // transaction. This is deliberately a one-shot publication:
                // later tool-round checkpoints must not promote a failed
                // attempt's tail to the failover base.
                sink.checkpoint_summarized_context(messages, expected)
                    .await?;
            } else {
                sink.checkpoint_context(messages, expected).await?;
            }
            // Durable-first publication: a failed checkpoint must leave the
            // Agent's canonical history and pending marker untouched. This
            // single ordering protects turn-start, ordinary mid-loop, and
            // current-group capacity-recovery Tier 3 callers alike.
            *self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = messages.to_vec();
            self.tier3_summary_publication_pending
                .store(false, std::sync::atomic::Ordering::Release);
            return Ok(());
        }

        let Some(db) = self
            .session_db
            .clone()
            .or_else(|| crate::get_session_db().cloned())
        else {
            if self.tier3_summary_publication_pending() {
                anyhow::bail!(
                    "cannot publish Tier 3 summary for session {sid}: durability DB unavailable"
                );
            }
            *self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = messages.to_vec();
            return Ok(());
        };
        let json = serde_json::to_string(messages)?;
        let (_, revision) = db.load_context_with_revision(sid)?;
        if self.tier3_summary_publication_pending() {
            db.save_context_at_revision_and_clear_tier3_recovery(sid, revision, &json)?;
            if self.session_is_incognito() {
                crate::session::clear_incognito_tier3_recovery(sid);
            }
        } else {
            db.save_context_at_revision(sid, revision, &json, None)?;
        }
        *self
            .conversation_history
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = messages.to_vec();
        self.tier3_summary_publication_pending
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    /// Run context compaction (Tier 1-3) on messages before API call.
    /// If Tier 3 summarization is needed, performs a non-streaming LLM call to summarize old messages.
    /// If flush_before_compact is enabled, extracts memories from messages before they are summarized.
    #[doc(hidden)]
    pub async fn run_compaction(
        &self,
        request_projection: &mut Vec<serde_json::Value>,
        canonical_history: &mut Vec<serde_json::Value>,
        system_prompt: &str,
        tool_schemas: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
        restored_attempt_base_contains_current_user: bool,
        cancel: Option<Arc<AtomicBool>>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> CompactionRunOutcome {
        // Tier 4 publishes its deterministic emergency history and the durable
        // follow-up marker together, before retrying the same user turn. That
        // retry already carries the exact current user in its adopted base and
        // must not consume the marker it just created; the next user turn is
        // the first safe boundary for the forced Tier 3 summary.
        let recovery_state = match self.pending_tier3_recovery_requirement().await {
            Ok(state) => state,
            Err(error) => {
                return CompactionRunOutcome {
                    fatal_error: Some(format!(
                        "cannot verify pending Tier 3 recovery state: {error}"
                    )),
                    ..CompactionRunOutcome::default()
                };
            }
        };
        if restored_attempt_base_contains_current_user
            && matches!(
                recovery_state.map(|requirement| requirement.state),
                Some(crate::session::Tier3RecoveryState::Required)
            )
        {
            // Same-turn Tier 4 retry: preserve the newly published debt and
            // do not let the ordinary high-watermark path consume it early.
            return CompactionRunOutcome {
                cache_decision: Some(match recovery_state.map(|requirement| requirement.kind) {
                    Some(crate::session::Tier3RecoveryRequirementKind::CapacityProjection) => {
                        crate::context_compact::CacheCompactionDecision::CapacityRecovery
                    }
                    _ => crate::context_compact::CacheCompactionDecision::Emergency,
                }),
                ..CompactionRunOutcome::default()
            };
        }
        if matches!(
            recovery_state.map(|requirement| requirement.state),
            Some(crate::session::Tier3RecoveryState::InProgress)
        ) {
            return CompactionRunOutcome {
                fatal_error: Some(
                    "Tier 3 recovery summary is in progress or its delivery is unknown; refusing concurrent Provider request"
                        .to_string(),
                ),
                ..CompactionRunOutcome::default()
            };
        }
        if restored_attempt_base_contains_current_user {
            // A promoted failover base has already crossed this turn's
            // compaction boundary: either Tier 3 published a validated
            // summary, or Tier 4 published an emergency projection together
            // with the Required marker handled above. Re-running routine
            // compaction on a fallback model would pay for another summary
            // and rewrite an otherwise stable cache prefix. Exact request
            // accounting and the in-loop capacity ladder still run later.
            return CompactionRunOutcome {
                cache_decision: Some(crate::context_compact::CacheCompactionDecision::KeepPrefix),
                ..CompactionRunOutcome::default()
            };
        }
        let force_tier3_recovery = matches!(
            recovery_state.map(|requirement| requirement.state),
            Some(crate::session::Tier3RecoveryState::Required)
        );
        if force_tier3_recovery {
            match self.claim_tier3_recovery_attempt().await {
                Ok(true) => {}
                Ok(false) => {
                    return CompactionRunOutcome {
                        fatal_error: Some(
                            "Tier 3 recovery attempt was already claimed; refusing concurrent Provider request"
                                .to_string(),
                        ),
                        ..CompactionRunOutcome::default()
                    };
                }
                Err(error) => {
                    return CompactionRunOutcome {
                        fatal_error: Some(format!(
                            "cannot claim pending Tier 3 recovery attempt: {error}"
                        )),
                        ..CompactionRunOutcome::default()
                    };
                }
            }
        }
        let mut options = CompactionRunOptions::turn_start(cancel).with_tool_schemas(tool_schemas);
        if force_tier3_recovery {
            // An emergency or capacity-only projection is not a healthy
            // long-term history. The next safe request gets one forced Tier 3
            // attempt, bypassing ordinary watermarks and cache TTL; failure is
            // durably exhausted below to prevent per-turn retry charges.
            options.bypass_cache_ttl = true;
            options.force_summary = true;
            options.summary_reason = match recovery_state.map(|requirement| requirement.kind) {
                Some(crate::session::Tier3RecoveryRequirementKind::CapacityProjection) => {
                    crate::context_compact::CompactionSummaryReason::RequiredAfterRecovery
                }
                _ => crate::context_compact::CompactionSummaryReason::EmergencyFollowup,
            };
        }
        let outcome = self
            .run_compaction_with_options(
                request_projection,
                canonical_history,
                system_prompt,
                model,
                max_tokens,
                on_delta,
                options,
            )
            .await;

        if force_tier3_recovery && !outcome.summary_applied {
            let visible_failure = outcome.compact_result.as_ref().is_some_and(|result| {
                matches!(
                    result.description.as_str(),
                    "summarization_timed_out"
                        | "summarization_timed_out_sync_compaction_only"
                        | "summarization_not_applied"
                        | "summarization_not_applied_sync_compaction_only"
                )
            });
            if !visible_failure {
                self.emit_compaction_progress(on_delta, "failed", "summary", None, None, None);
            }
            if let Err(error) = self.exhaust_tier3_recovery_attempt().await {
                return CompactionRunOutcome {
                    fatal_error: Some(format!(
                        "cannot finalize failed Tier 3 recovery attempt: {error}"
                    )),
                    ..outcome
                };
            }
        }

        outcome
    }

    pub(super) async fn run_compaction_with_options(
        &self,
        request_projection: &mut Vec<serde_json::Value>,
        canonical_history: &mut Vec<serde_json::Value>,
        system_prompt: &str,
        model: &str,
        max_tokens: u32,
        on_delta: &(impl Fn(&str) + Send),
        options: CompactionRunOptions,
    ) -> CompactionRunOutcome {
        use crate::context_compact;

        let provider_format = super::types::ProviderFormat::from(&self.provider);
        let provider_family = match provider_format {
            super::types::ProviderFormat::Anthropic => {
                crate::token_accounting::ProviderFamily::Anthropic
            }
            super::types::ProviderFormat::OpenAIChat => {
                crate::token_accounting::ProviderFamily::OpenAiChat
            }
            super::types::ProviderFormat::OpenAIResponses => {
                crate::token_accounting::ProviderFamily::OpenAiResponses
            }
            super::types::ProviderFormat::Codex => crate::token_accounting::ProviderFamily::Codex,
        };
        let request_shape = match provider_format {
            super::types::ProviderFormat::Anthropic => {
                crate::token_accounting::RequestShape::AnthropicMessages
            }
            super::types::ProviderFormat::OpenAIChat => {
                crate::token_accounting::RequestShape::OpenAiChat
            }
            super::types::ProviderFormat::OpenAIResponses => {
                crate::token_accounting::RequestShape::OpenAiResponses
            }
            super::types::ProviderFormat::Codex => {
                crate::token_accounting::RequestShape::CodexResponses
            }
        };
        let token_counter = crate::token_accounting::CompactionTokenCounter::new(
            provider_family,
            model,
            request_shape,
            &options.tool_schemas,
        );
        let estimate_request = |messages: &[serde_json::Value]| {
            token_counter.count_request_upper(system_prompt, messages, max_tokens)
        };

        /// Usage ratio that overrides cache-TTL throttle to prevent ContextOverflow → Tier 4.
        const CACHE_TTL_EMERGENCY_RATIO: f64 = 0.95;

        let forced_config;
        let compact_config = if options.force_summary {
            forced_config = {
                let mut cfg = self.compact_config.clone();
                cfg.enabled = true;
                cfg.soft_trim_ratio = 0.0;
                // Keep hard-clear at the configured pressure threshold so a
                // manual summary can still see old tool output when possible.
                cfg.summarization_threshold = 0.0;
                cfg
            };
            &forced_config
        } else {
            &self.compact_config
        };

        // Pre-compute cache-TTL throttle state as two booleans for CompactionContext.
        let (cache_ttl_throttled, cache_ttl_emergency) = if options.bypass_cache_ttl {
            (false, false)
        } else if compact_config.cache_ttl_secs > 0 {
            let within_ttl = {
                let guard = self
                    .last_tier2_compaction_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                matches!(*guard, Some(ts) if ts.elapsed().as_secs() < compact_config.cache_ttl_secs)
            };
            if within_ttl {
                let tokens_now = estimate_request(request_projection);
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
            let tokens_now = estimate_request(request_projection);
            let usage_now = tokens_now as f64 / self.context_window.max(1) as f64;
            // `run_compaction` runs every turn but is a no-op below the
            // summary high watermark — only consult the PreCompact hook when a
            // compaction is actually plausible, so it precedes a real
            // compaction instead of firing every idle turn.
            let sid = self.session_id.clone().unwrap_or_default();
            if usage_now >= compact_config.summarization_threshold
                || matches!(options.trigger, CompactionRunTrigger::Manual)
                || options.force_summary
                || options.summary_hard_protected_start.is_some()
            {
                let input = crate::hooks::HookInput::PreCompact {
                    common: self.hook_common_input("PreCompact"),
                    trigger: options.trigger.hook_trigger(),
                    usage_ratio: usage_now.min(1.0),
                    // Live custom compaction instructions, so a PreCompact hook
                    // can enforce / audit the active policy (official field).
                    custom_instructions: compact_config.custom_instructions.clone(),
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
                    if usage_now >= CACHE_TTL_EMERGENCY_RATIO
                        || options.force_summary
                        || options.summary_hard_protected_start.is_some()
                    {
                        app_warn!(
                            "hooks",
                            "dispatch",
                            "PreCompact block overridden for required compaction: usage {:.1}%, compacting anyway",
                            usage_now * 100.0,
                        );
                        crate::hooks::reset_precompact_blocks(&sid);
                    } else if crate::hooks::honor_precompact_block(&sid) {
                        app_info!(
                            "hooks",
                            "dispatch",
                            "PreCompact hook blocked compaction (usage {:.1}%)",
                            usage_now * 100.0
                        );
                        return CompactionRunOutcome {
                            tokens_after: tokens_now,
                            ..CompactionRunOutcome::default()
                        };
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
            config: compact_config,
            cache_ttl_throttled,
            cache_ttl_emergency,
            token_counter: Some(&token_counter),
        };
        let mut compact_result = if options.summary_hard_protected_start.is_some() {
            // The deterministic capacity ladder already ran Tier 0 and Tier 2
            // as separate exact-counted passes. Synthesize only the Tier-3
            // signal here; re-entering compact_sync would silently run the
            // legacy combined Tier0/1/2 path and over-degrade the protected
            // request projection.
            let tokens = estimate_request(request_projection);
            let details = crate::context_compact::CompactDetails {
                tool_results_truncated: 0,
                tool_results_soft_trimmed: 0,
                tool_results_hard_cleared: 0,
                messages_summarized: 0,
                summary_tokens: None,
            };
            let boundary = crate::context_compact::boundary_snapshot(
                request_projection,
                compact_config.preserve_recent_rounds,
            )
            .boundary(
                request_projection,
                crate::context_compact::BoundaryMode::SummarizeUnderPressure,
            );
            let mut manifest = crate::context_compact::CompactionManifest::for_result_with_boundary(
                3,
                options.trigger.manifest_trigger(),
                tokens,
                tokens,
                Some(&details),
                &boundary,
            );
            manifest.protected_start_index = options.summary_hard_protected_start;
            crate::context_compact::CompactResult {
                tier_applied: 3,
                tokens_before: tokens,
                tokens_after: tokens,
                messages_affected: 0,
                description: "summarization_needed".to_string(),
                details: Some(details),
                manifest: Some(manifest),
            }
        } else {
            self.context_engine
                .compact_routine(request_projection, &ctx)
        };
        if let Some(manifest) = compact_result.manifest.as_mut() {
            manifest.trigger = options.trigger.manifest_trigger().to_string();
        }

        if compact_result.tier_applied == 0 {
            return CompactionRunOutcome {
                changed_history: compact_result.messages_affected > 0,
                tokens_after: compact_result.tokens_after,
                cache_decision: Some(crate::context_compact::CacheCompactionDecision::KeepPrefix),
                compact_result: Some(compact_result),
                ..CompactionRunOutcome::default()
            };
        }
        let cache_decision = match compact_result.tier_applied {
            3 => crate::context_compact::CacheCompactionDecision::SummaryOnce,
            4 => crate::context_compact::CacheCompactionDecision::Emergency,
            _ => crate::context_compact::CacheCompactionDecision::CompatibilityProjection,
        };
        let mut run_outcome = CompactionRunOutcome {
            tier_applied: compact_result.tier_applied,
            changed_history: compact_result.messages_affected > 0,
            summary_applied: false,
            summary_timed_out: false,
            summary_reason: None,
            cache_decision: Some(cache_decision),
            tokens_after: compact_result.tokens_after,
            cancelled: false,
            fatal_error: None,
            compact_result: None,
        };

        // Touch timer after synchronous Tier 2 completes.
        // Tier 3 touches the timer separately in its own success path (after async LLM call).
        if compact_result.tier_applied == 2 {
            self.touch_compaction_timer();
        }

        // A compatibility Tier-2 projection has already changed the request
        // prefix. Tier 3 is only a signal at this point; defer its refresh
        // boundary until a validated summary is actually installed.
        if compact_result.tier_applied == 2 {
            self.refresh_awareness_after_prefix_rewrite();
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
            if !options.allow_summarization {
                if let Some(manifest) = compact_result.manifest.as_mut() {
                    manifest
                        .warnings
                        .push("tier3_summary_skipped_by_policy".to_string());
                }
            } else {
                let split = if let Some(hard_start) = options.summary_hard_protected_start {
                    let snapshot = context_compact::boundary_snapshot(
                        canonical_history,
                        compact_config.preserve_recent_rounds,
                    );
                    let mut boundary = snapshot.boundary(
                        canonical_history,
                        context_compact::BoundaryMode::SummarizeUnderPressure,
                    );
                    // Capacity recovery has already exhausted deterministic
                    // old-history tiers. Only the current user/tool suffix is
                    // a hard boundary now; ordinary recent-round preferences
                    // must not keep an otherwise reclaimable old prefix and
                    // turn a recoverable request into a terminal C0 overflow.
                    boundary.protected_start_index = hard_start.min(canonical_history.len());
                    boundary
                        .warnings
                        .push("summary_boundary_clamped_before_current_tool_group".to_string());
                    context_compact::split_for_summarization_with_boundary(
                        canonical_history,
                        &boundary,
                    )
                } else {
                    context_compact::split_for_summarization(canonical_history, compact_config)
                };
                if let Some(mut split) = split {
                    // Tier 0/1/2 operate on `request_projection`. Tier 3 is a
                    // semantic history rewrite and therefore must summarize the
                    // full canonical effective history, not the already-trimmed
                    // request view. Build the candidate off to the side and only
                    // publish it to both views after every validation succeeds.
                    let mut summary_candidate = canonical_history.clone();
                    let canonical_tokens_before = estimate_request(canonical_history);
                    let is_incognito = self.session_is_incognito();
                    if options.force_summary {
                        let preserved_start =
                            split.preserved_start_index.min(summary_candidate.len());
                        let recovered_tool_cleanup =
                            context_compact::compact_oversized_recovered_tool_results(
                                &mut summary_candidate[preserved_start..],
                                compact_config.soft_trim_max_chars,
                                self.session_id.as_deref(),
                                !is_incognito,
                            );
                        if recovered_tool_cleanup.changed() {
                            split.preserved = summary_candidate[preserved_start..].to_vec();
                            record_manual_recovered_tool_cleanup(
                                &mut compact_result,
                                recovered_tool_cleanup,
                            );
                            run_outcome.changed_history = true;
                            app_info!(
                            "context",
                            "compact",
                            "Manual compaction cleaned oversized recovered tool results: hard_cleared={}, image_markers_materialized={}",
                            recovered_tool_cleanup.hard_cleared,
                            recovered_tool_cleanup.image_markers_materialized
                        );
                        }
                    }

                    let runtime_ledger_snapshot = if is_incognito {
                        context_compact::RuntimeLedgerSnapshot::default()
                    } else {
                        self.session_id
                            .as_deref()
                            .filter(|sid| !sid.is_empty())
                            .map(crate::agent::runtime_ledger::build_runtime_ledger_snapshot)
                            .unwrap_or_default()
                    };
                    if let Some(manifest) = compact_result.manifest.as_mut() {
                        manifest
                            .warnings
                            .extend(runtime_ledger_snapshot.warnings.iter().cloned());
                        manifest
                            .warnings
                            .extend(split.boundary_warnings.iter().cloned());
                    }
                    // Memory Flush: extract memories from messages about to be summarized
                    {
                        let flush_enabled = {
                            let global = crate::memory::load_extract_config();
                            let agent_flush = crate::agent_loader::load_agent(&self.agent_id)
                                .ok()
                                .and_then(|d| d.config.memory.flush_before_compact);
                            let runtime = &crate::config::cached_config().memory;
                            if runtime.rollout.enabled {
                                runtime.enabled
                                    && !matches!(
                                        runtime.learning.mode,
                                        crate::memory::MemoryLearningMode::Manual
                                    )
                                    && agent_flush.unwrap_or(global.flush_before_compact)
                            } else {
                                global.enabled && agent_flush.unwrap_or(global.flush_before_compact)
                            }
                        } && !is_incognito
                            && options.allow_memory_flush;

                        if flush_enabled {
                            // Resolve provider config on the current thread before spawning
                            let flush_provider =
                                crate::config::cached_config().providers.first().cloned();

                            if let Some(prov) = flush_provider {
                                if let Some(model) = prov.models.first().cloned() {
                                    let agent_id = self.agent_id.clone();
                                    let session_id = self.session_id.clone().unwrap_or_default();
                                    let msgs = split.summarizable.clone();
                                    let model_ref =
                                        crate::memory::extract_runtime::MemoryExtractModel::capture(
                                            &prov,
                                            model.id.clone(),
                                        );
                                    let session_db = self.session_db.clone();

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
                                                    crate::memory::extract_runtime::flush_before_compact(
                                                        &msgs,
                                                        &agent_id,
                                                        &session_id,
                                                        &model_ref,
                                                        session_db,
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

                    if options.emit_start_event {
                        self.emit_compaction_progress(
                            on_delta,
                            "preparing",
                            "summary",
                            Some(split.summarizable.len()),
                            None,
                            None,
                        );
                        self.emit_compaction_progress(
                            on_delta,
                            "summarizing",
                            "summary",
                            Some(split.summarizable.len()),
                            None,
                            None,
                        );
                    }

                    // The summarization request has its own capacity boundary. It
                    // must fit both the optional dedicated model and the
                    // conversation-model fallback; otherwise a history overflow
                    // simply turns into a second overflow while trying to compact.
                    // Use the model-neutral upper bound here so CJK/code are not
                    // reduced to the old bytes/4 approximation.
                    let dedicated_summary_window = self
                        .compaction_provider
                        .as_ref()
                        .and_then(|provider| provider.context_window())
                        .filter(|window| *window > 0);
                    let summary_context_window = match (
                        (self.context_window > 0).then_some(self.context_window),
                        dedicated_summary_window,
                    ) {
                        (Some(conversation), Some(dedicated)) => conversation.min(dedicated),
                        (Some(conversation), None) => conversation,
                        (None, Some(dedicated)) => dedicated,
                        (None, None) => 0,
                    };
                    let summary_safety_headroom = u64::from(summary_context_window / 50).max(1_024);
                    let summary_target_input_upper = u64::from(summary_context_window)
                        .saturating_sub(u64::from(compact_config.summary_max_tokens))
                        .saturating_sub(summary_safety_headroom);
                    let build_summary_request = |messages: &[serde_json::Value]| {
                        let (prompt_messages, previous_summary) =
                            context_compact::peel_previous_summary(messages);
                        let prompt = context_compact::build_summarization_prompt(
                            &prompt_messages,
                            previous_summary.as_deref(),
                            compact_config,
                        );
                        let capacity_text = format!(
                            "{}\n\n{}",
                            context_compact::SUMMARIZATION_SYSTEM_PROMPT,
                            prompt
                        );
                        let upper = crate::token_accounting::service()
                            .count_text(
                                crate::token_accounting::ProviderFamily::Unknown,
                                "tier3-summary-input",
                                &capacity_text,
                            )
                            .upper_bound;
                        (prompt, upper)
                    };

                    // Prepare only the one-shot summary input. These
                    // deterministic Tier 0/2 edits are never published to the
                    // canonical history or the main request projection, so
                    // they cannot churn the conversation's prompt-cache
                    // prefix. If they are insufficient, the summary attempt
                    // still fails closed before network I/O.
                    let mut summary_input_messages = split.summarizable.clone();
                    let (_, summary_input_before) = build_summary_request(&summary_input_messages);
                    let mut summary_input_projection_used = false;
                    if summary_context_window > 0
                        && summary_input_before > summary_target_input_upper
                    {
                        let protected_start = summary_input_messages.len();
                        for tier in [
                            context_compact::CapacityPressureTier::Tier0,
                            context_compact::CapacityPressureTier::Tier2,
                        ] {
                            let pressure = context_compact::apply_capacity_pressure_tier(
                                &mut summary_input_messages,
                                protected_start,
                                compact_config,
                                tier,
                                summary_target_input_upper,
                                |candidate| Ok(build_summary_request(candidate).1),
                            );
                            match pressure {
                                Ok(result) => {
                                    summary_input_projection_used |= !result.edits.is_empty();
                                    if result.reached_target {
                                        break;
                                    }
                                }
                                Err(error) => {
                                    if let Some(manifest) = compact_result.manifest.as_mut() {
                                        manifest.warnings.push(format!(
                                            "summary_input_projection_failed:{error}"
                                        ));
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    if summary_input_projection_used {
                        if let Some(manifest) = compact_result.manifest.as_mut() {
                            manifest
                                .warnings
                                .push("summary_input_projection_only".to_string());
                        }
                    }
                    let (prompt, summary_input_upper) =
                        build_summary_request(&summary_input_messages);
                    let summary_total_upper = summary_input_upper
                        .saturating_add(u64::from(compact_config.summary_max_tokens))
                        .saturating_add(summary_safety_headroom);
                    let prompt_capacity_error = (summary_context_window == 0
                    || summary_total_upper > u64::from(summary_context_window))
                .then(|| {
                    format!(
                        "summary request does not fit its model window: input_upper={}, output_reservation={}, safety={}, window={}",
                        summary_input_upper,
                        compact_config.summary_max_tokens,
                        summary_safety_headroom,
                        summary_context_window
                    )
                });

                    let summary_future = tokio::time::timeout(
                        std::time::Duration::from_secs(compact_config.summarization_timeout_secs),
                        async {
                            if let Some(error) = prompt_capacity_error {
                                anyhow::bail!(error);
                            }
                            self.summarize_with_model(&prompt).await
                        },
                    );
                    let summary_result = if let Some(cancel) = options.cancel.clone() {
                        tokio::select! {
                            biased;
                            result = summary_future => result,
                            _ = crate::chat_engine::wait_for_chat_cancel(cancel.clone()) => {
                                let tokens_after = estimate_request(request_projection);
                                if let Some(manifest) = compact_result.manifest.as_mut() {
                                    manifest.warnings.push("tier3_summary_cancelled".to_string());
                                }
                                self.emit_compaction_progress(
                                    on_delta,
                                    "failed",
                                    "summary",
                                    Some(split.summarizable.len()),
                                    None,
                                    None,
                                );
                                // The synchronous tiers already changed the request
                                // projection before the awaited summary. Canonical
                                // history remains untouched; report the projection
                                // change so the caller refreshes its request cache.
                                return CompactionRunOutcome::cancelled(
                                    tokens_after,
                                    compact_result.messages_affected > 0,
                                );
                            }
                        }
                    } else {
                        summary_future.await
                    };

                    'summary_attempt: {
                        match summary_result {
                            Ok(Ok(summary)) => {
                                let injection_budget_chars = ((self.context_window as f64
                                    * compact_config.max_compaction_injected_context_share)
                                    .round()
                                    as usize)
                                    .saturating_mul(context_compact::CHARS_PER_TOKEN);
                                if let Err(error) = context_compact::apply_summary(
                                    &mut summary_candidate,
                                    &summary,
                                    split.preserved_start_index,
                                    compact_config,
                                    Some(injection_budget_chars),
                                ) {
                                    app_warn!(
                                "context",
                                "compact",
                                "Tier 3 summary was valid but could not be installed safely: {}",
                                error
                            );
                                    if let Some(manifest) = compact_result.manifest.as_mut() {
                                        manifest
                                            .warnings
                                            .push("tier3_summary_install_rejected".to_string());
                                    }
                                    self.emit_compaction_progress(
                                        on_delta,
                                        "failed",
                                        "summary",
                                        Some(split.summarizable.len()),
                                        None,
                                        None,
                                    );
                                    break 'summary_attempt;
                                }
                                let summarized_count = split.summarizable.len();
                                let summary_tokens =
                                    ((summary.len() + context_compact::CHARS_PER_TOKEN - 1)
                                        / context_compact::CHARS_PER_TOKEN)
                                        as u32;
                                compact_result.messages_affected = compact_result
                                    .messages_affected
                                    .saturating_add(summarized_count);
                                compact_result.tokens_before = canonical_tokens_before;
                                compact_result.tier_applied = compact_result.tier_applied.max(3);
                                compact_result.description = "summarized".to_string();
                                if let Some(details) = compact_result.details.as_mut() {
                                    details.messages_summarized = summarized_count;
                                    details.summary_tokens = Some(summary_tokens);
                                }
                                run_outcome.summary_applied = true;
                                run_outcome.summary_reason = Some(options.summary_reason);
                                self.tier3_summary_applied_this_turn
                                    .store(true, std::sync::atomic::Ordering::Release);
                                self.tier3_summary_publication_pending
                                    .store(true, std::sync::atomic::Ordering::Release);
                                run_outcome.changed_history = true;
                                // Update cache-TTL timer after successful Tier 3 summarization
                                self.touch_compaction_timer();
                                // Core Memory is session-stable between refresh points.
                                // A successful Tier 3 summary is an explicit refresh
                                // boundary, so the next turn captures current files.
                                self.invalidate_core_memory_snapshot();
                                self.refresh_awareness_after_prefix_rewrite();
                                // Record the summarized range in the manifest ONLY after the
                                // summary actually applied — on failure/timeout (arms below)
                                // the messages are untouched, so the manifest must not claim
                                // a summary happened.
                                if let Some(manifest) = compact_result.manifest.as_mut() {
                                    manifest.tokens_before = canonical_tokens_before;
                                    manifest.tier = manifest.tier.max(3);
                                    manifest.protected_start_index =
                                        Some(split.preserved_start_index);
                                    manifest.summarized_range =
                                        Some((0, split.preserved_start_index));
                                    manifest.rounds_summarized =
                                        context_compact::build_message_rounds(&split.summarizable)
                                            .len();
                                }
                                if let Some(logger) = crate::get_logger() {
                                    logger.log(
                                    "info",
                                    "context",
                                    "compact",
                                    &format!(
                                        "Tier 3 summarization complete: {} messages → {} chars summary, {} messages preserved",
                                        split.summarizable.len(),
                                        summary.len(),
                                        split.preserved.len(),
                                    ),
                                    None,
                                    None,
                                    None,
                                );
                                }

                                // Post-compaction file recovery: re-inject recently-edited file contents
                                let tokens_after_summary = estimate_request(&summary_candidate);
                                let tokens_freed =
                                    canonical_tokens_before.saturating_sub(tokens_after_summary);
                                let summary_chars = summary_candidate
                                    .first()
                                    .map(message_content_chars)
                                    .unwrap_or(summary.len());
                                let injection_remaining_after_summary =
                                    injection_budget_chars.saturating_sub(summary_chars);
                                let ledger_has_live_state =
                                    !runtime_ledger_snapshot.background_jobs.is_empty()
                                        || !runtime_ledger_snapshot.subagents.is_empty()
                                        || !runtime_ledger_snapshot.warnings.is_empty();
                                let has_file_touches = !is_incognito
                                    && !context_compact::extract_file_touches(&split.summarizable)
                                        .is_empty();
                                let ledger_reserve = post_summary_ledger_reserve_chars(
                                    injection_remaining_after_summary,
                                    ledger_has_live_state,
                                    has_file_touches,
                                );
                                let recovery_budget = injection_remaining_after_summary
                                    .saturating_sub(ledger_reserve);
                                let recovery = if is_incognito {
                                    context_compact::RecoveryResult {
                                        message: None,
                                        recovered_files: Vec::new(),
                                        skipped_files: Vec::new(),
                                        file_touches: Vec::new(),
                                    }
                                } else {
                                    let recovery_cwd =
                                        crate::session::effective_session_working_dir(
                                            self.session_id.as_deref(),
                                        )
                                        .map(std::path::PathBuf::from);
                                    let recovery_ctx = context_compact::RecoveryContext {
                                        session_working_dir: recovery_cwd.as_deref(),
                                        // Historical tool arguments are not file-read
                                        // authorization. The Result Store/file ledger
                                        // will supply provenance-checked bindings; until
                                        // then recovery deliberately returns references
                                        // without reopening host files.
                                        authorized_files: None,
                                        tokens_freed,
                                        max_total_bytes: Some(recovery_budget),
                                        config: compact_config,
                                    };
                                    context_compact::build_recovery_message(
                                        &split.summarizable,
                                        &split.preserved,
                                        &recovery_ctx,
                                    )
                                };
                                let recovery_chars = recovery
                                    .message
                                    .as_ref()
                                    .map(message_content_chars)
                                    .unwrap_or(0);
                                let ledger_budget = injection_remaining_after_summary
                                    .saturating_sub(recovery_chars)
                                    .min(8_000);
                                let ledger_msg = if is_incognito {
                                    None
                                } else {
                                    context_compact::build_runtime_ledger_message(
                                        &runtime_ledger_snapshot,
                                        &recovery.file_touches,
                                        ledger_budget,
                                    )
                                };
                                if options.emit_start_event && ledger_msg.is_some() {
                                    self.emit_compaction_progress(
                                        on_delta,
                                        "preserving_runtime_state",
                                        "summary",
                                        Some(split.summarizable.len()),
                                        None,
                                        Some(runtime_ledger_snapshot.warnings.len()),
                                    );
                                }
                                if options.emit_start_event && recovery.message.is_some() {
                                    self.emit_compaction_progress(
                                        on_delta,
                                        "restoring_files",
                                        "summary",
                                        Some(split.summarizable.len()),
                                        Some(recovery.recovered_files.len()),
                                        None,
                                    );
                                }
                                if let Some(manifest) = compact_result.manifest.as_mut() {
                                    manifest.files_recovered = recovery.recovered_files.len();
                                    for skipped in &recovery.skipped_files {
                                        manifest.warnings.push(format!(
                                            "recovery_skipped:{}:{}",
                                            skipped.path, skipped.reason
                                        ));
                                    }
                                    if summary_chars >= injection_budget_chars {
                                        manifest.warnings.push(
                                            "post_compaction_injection_budget_exhausted"
                                                .to_string(),
                                        );
                                    }
                                }
                                let mut insert_at = context_compact::POST_SUMMARY_INSERT_INDEX
                                    .min(summary_candidate.len());
                                if let Some(ledger_msg) = ledger_msg {
                                    summary_candidate.insert(insert_at, ledger_msg);
                                    insert_at += 1;
                                }
                                if let Some(recovery_msg) = recovery.message {
                                    // Insert after summary and optional runtime ledger.
                                    let insert_at = insert_at.min(summary_candidate.len());
                                    summary_candidate.insert(insert_at, recovery_msg);
                                    app_info!(
                                "context",
                                "compact",
                                "Post-compaction recovery: injected file contents after summary"
                            );
                                }
                                if options.emit_start_event {
                                    self.emit_compaction_progress(
                                        on_delta,
                                        "finalizing",
                                        "summary",
                                        Some(split.summarizable.len()),
                                        None,
                                        None,
                                    );
                                }
                                *canonical_history = summary_candidate.clone();
                                *request_projection = summary_candidate;
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
                                self.emit_compaction_progress(
                                    on_delta,
                                    "failed",
                                    "summary",
                                    Some(split.summarizable.len()),
                                    None,
                                    None,
                                );
                            }
                            Err(_) => {
                                run_outcome.summary_timed_out = true;
                                if let Some(logger) = crate::get_logger() {
                                    logger.log(
                                        "warn",
                                        "context",
                                        "compact",
                                        &format!(
                                            "Tier 3 summarization timed out after {}s",
                                            compact_config.summarization_timeout_secs
                                        ),
                                        None,
                                        None,
                                        None,
                                    );
                                }
                                self.emit_compaction_progress(
                                    on_delta,
                                    "failed",
                                    "summary",
                                    Some(split.summarizable.len()),
                                    None,
                                    None,
                                );
                            }
                        }
                    }
                }
            }
        }

        if compact_result.description == "summarization_needed" && !run_outcome.summary_applied {
            let sync_tier = sync_tier_from_compact_result(&compact_result);
            compact_result.tier_applied = sync_tier;
            let summary_skipped_by_policy = !options.allow_summarization;
            compact_result.description = if summary_skipped_by_policy {
                if sync_tier > 0 {
                    "summarization_skipped_by_policy_sync_compaction_only".to_string()
                } else {
                    "summarization_skipped_by_policy".to_string()
                }
            } else if run_outcome.summary_timed_out {
                if sync_tier > 0 {
                    "summarization_timed_out_sync_compaction_only".to_string()
                } else {
                    "summarization_timed_out".to_string()
                }
            } else if sync_tier > 0 {
                "summarization_not_applied_sync_compaction_only".to_string()
            } else {
                "summarization_not_applied".to_string()
            };
            if let Some(manifest) = compact_result.manifest.as_mut() {
                manifest.tier = sync_tier;
                if !summary_skipped_by_policy {
                    manifest.warnings.push(
                        if run_outcome.summary_timed_out {
                            "tier3_summary_timed_out"
                        } else {
                            "tier3_summary_not_applied"
                        }
                        .to_string(),
                    );
                }
            }
            run_outcome.tier_applied = sync_tier;
            run_outcome.changed_history = sync_tier > 0 && compact_result.messages_affected > 0;
            // Even when no synchronous tier changed history, continue through
            // the final event path.  A Tier 3 timeout/provider failure is an
            // actionable terminal outcome: persisting it keeps the manual
            // retry affordance available after a reload instead of leaving a
            // transient progress notice as the only evidence.
        }

        // Emit compaction event to frontend
        let tokens_after = estimate_request(request_projection);
        if let Some(manifest) = compact_result.manifest.as_mut() {
            manifest.tokens_after = tokens_after;
        }
        compact_result.tokens_after = tokens_after;

        // PostCompact + SessionStart(compact) hooks (observation): fire after a
        // real compaction (tier ≥ 2; tier 0 returned early above). Queues any
        // additionalContext for the next round's reminder suffix.
        if compact_result.tier_applied >= 2 {
            self.fire_compaction_hooks(
                compact_result.tier_applied,
                tokens_after,
                model,
                options.trigger.hook_trigger(),
            )
            .await;
        }

        if let Ok(event) = serde_json::to_string(&json!({
            "type": "context_compacted",
                "data": {
                    "tier_applied": compact_result.tier_applied,
                    "tokens_before": compact_result.tokens_before,
                    "tokens_after": tokens_after,
                    "messages_affected": compact_result.messages_affected,
                    "description": compact_result.description,
                    "summary_reason": run_outcome.summary_reason.map(|reason| reason.as_str()),
                    "manifest": compact_result.manifest.clone(),
                }
        })) {
            on_delta(&event);
        }
        run_outcome.tokens_after = tokens_after;
        run_outcome.tier_applied = compact_result.tier_applied;
        run_outcome.changed_history |= compact_result.messages_affected > 0;
        run_outcome.compact_result = Some(compact_result);
        run_outcome
    }

    /// Append hook-injected context to the pending queue, drained into the next
    /// round's reminder suffix. ArcSwap `rcu` so no `&mut self` is needed.
    #[doc(hidden)]
    pub fn push_pending_hook_context(&self, ctx: String) {
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
    #[doc(hidden)]
    pub fn drain_pending_hook_context(&self) -> Option<String> {
        let taken = self
            .pending_hook_context
            .swap(std::sync::Arc::new(Vec::new()));
        if taken.is_empty() {
            None
        } else {
            Some(taken.join("\n\n"))
        }
    }

    fn emit_compaction_progress(
        &self,
        on_delta: &(impl Fn(&str) + Send),
        phase: &str,
        kind: &str,
        messages_to_summarize: Option<usize>,
        files_recovered: Option<usize>,
        warning_count: Option<usize>,
    ) {
        let mut data = serde_json::Map::new();
        data.insert("phase".to_string(), json!(phase));
        data.insert("kind".to_string(), json!(kind));
        if let Some(count) = messages_to_summarize {
            data.insert("messages_to_summarize".to_string(), json!(count));
        }
        if let Some(count) = files_recovered {
            data.insert("files_recovered".to_string(), json!(count));
        }
        if let Some(count) = warning_count {
            data.insert("warning_count".to_string(), json!(count));
        }
        if let Ok(event) = serde_json::to_string(&json!({
            "type": "context_compaction_progress",
            "data": data,
        })) {
            on_delta(&event);
        }
    }

    #[doc(hidden)]
    pub async fn maybe_compact_between_tool_rounds(
        &self,
        request_projection: &mut Vec<serde_json::Value>,
        canonical_history: &mut Vec<serde_json::Value>,
        system_prompt_for_budget: &str,
        request_tool_schemas: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
        cancel: Arc<AtomicBool>,
        mid_loop_state: &mut MidLoopCompactionState,
        round: u32,
        on_delta: &(impl Fn(&str) + Send),
        hard_protected_start: Option<usize>,
    ) -> Result<CompactionRunOutcome> {
        if hard_protected_start.is_some() {
            // The just-settled current group is still awaiting exact Tier-1
            // admission. Persist its C0 canonical skeleton now, but defer all
            // request-only reclamation to the next round's dedicated
            // P→T0→P→T2→P→T3→P state machine, which has the real dynamic/tool
            // lanes. The legacy combined compactor cannot safely touch it.
            self.persist_round_context(canonical_history).await?;
            return Ok(CompactionRunOutcome::default());
        }

        // Current-group Tier 1 admission already ran at PostToolUse. Routine
        // checkpoints must not re-truncate settled history or publish Tier 0/2
        // just because a low compatibility watermark was crossed: doing so
        // rewrites the Provider prefix and churns prompt-cache entries. Count
        // the unchanged request view and proceed directly to the high-watermark
        // Tier 3 decision. Exact capacity recovery owns deterministic Tier 0/2.
        let tokens_before_summary = crate::context_compact::estimate_request_tokens_with_tools(
            system_prompt_for_budget,
            request_projection,
            request_tool_schemas,
            max_tokens,
        );
        let usage_before_summary = if self.context_window > 0 {
            tokens_before_summary as f64 / self.context_window as f64
        } else {
            0.0
        };
        if !self.compact_config.enabled
            || usage_before_summary < self.compact_config.summarization_threshold
        {
            self.persist_round_context(canonical_history).await?;
            return Ok(CompactionRunOutcome {
                changed_history: false,
                tokens_after: tokens_before_summary,
                ..CompactionRunOutcome::default()
            });
        }

        let tier3_summarization_throttled = mid_loop_state.suppress_tier3_for_turn
            || mid_loop_state.summary_attempt_throttled(round);
        let allow_summarization = !tier3_summarization_throttled;
        // Only spend the per-turn attempt budget when a Tier 3 summary will
        // actually be attempted.
        if allow_summarization {
            mid_loop_state.record_summary_attempt(round);
        }
        let outcome = self
            .run_compaction_with_options(
                request_projection,
                canonical_history,
                system_prompt_for_budget,
                model,
                max_tokens,
                on_delta,
                CompactionRunOptions {
                    trigger: CompactionRunTrigger::ToolLoopCheckpoint,
                    bypass_cache_ttl: true,
                    emit_start_event: true,
                    allow_memory_flush: false,
                    allow_summarization,
                    force_summary: false,
                    summary_reason: crate::context_compact::CompactionSummaryReason::HighWatermark,
                    summary_hard_protected_start: None,
                    cancel: Some(cancel),
                    tool_schemas: request_tool_schemas.to_vec(),
                },
            )
            .await;

        self.persist_round_context(canonical_history).await?;

        if outcome.cancelled {
            return Ok(outcome);
        }

        if outcome.summary_applied {
            let threshold_floor = (self.compact_config.summarization_threshold
                - MID_LOOP_SUMMARY_HYSTERESIS_DELTA)
                .max(0.0);
            let usage_after = if self.context_window > 0 {
                outcome.tokens_after as f64 / self.context_window as f64
            } else {
                0.0
            };
            if usage_after >= threshold_floor {
                mid_loop_state.suppress_tier3_for_turn = true;
                app_warn!(
                    "context",
                    "compact",
                    "mid-loop summary applied but reduction was insufficient: usage={:.2}, floor={:.2}; suppressing further mid-loop Tier 3 this turn",
                    usage_after,
                    threshold_floor
                );
            }
        }

        Ok(outcome)
    }

    /// Run the single Tier-3 step after exact-counted Tier 0 and Tier 2 could
    /// not make the current result group's C0 request fit.
    ///
    /// It deliberately does not checkpoint internally. The caller validates
    /// that the protected current user/tool suffix survived byte-for-byte and
    /// then publishes the summary immediately, before any fallible rebuild or
    /// replan step. This keeps a generated summary from becoming an
    /// uncommitted in-memory side effect.
    #[doc(hidden)]
    pub async fn summarize_old_history_for_current_tool_group(
        &self,
        request_projection: &mut Vec<serde_json::Value>,
        canonical_history: &mut Vec<serde_json::Value>,
        provider_shaped_prompt: &str,
        provider_tool_schemas: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
        hard_protected_start: usize,
        cancel: Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send),
    ) -> Result<CompactionRunOutcome> {
        if !self.compact_config.enabled {
            anyhow::bail!("context compaction is disabled");
        }
        let outcome = self
            .run_compaction_with_options(
                request_projection,
                canonical_history,
                provider_shaped_prompt,
                model,
                max_tokens,
                on_delta,
                CompactionRunOptions {
                    trigger: CompactionRunTrigger::ToolLoopCheckpoint,
                    bypass_cache_ttl: true,
                    emit_start_event: true,
                    allow_memory_flush: false,
                    allow_summarization: true,
                    force_summary: false,
                    summary_reason:
                        crate::context_compact::CompactionSummaryReason::RequiredAfterRecovery,
                    summary_hard_protected_start: Some(hard_protected_start),
                    cancel: Some(cancel),
                    tool_schemas: provider_tool_schemas.to_vec(),
                },
            )
            .await;
        if let Some(error) = outcome.fatal_error.as_deref() {
            anyhow::bail!("required history recovery failed closed: {error}");
        }
        Ok(outcome)
    }

    /// Build common hook-input fields from agent-level state, for hooks that
    /// fire outside a tool context (compaction, etc.). `cwd` is the session
    /// working dir (falling back to home); `permission_mode` defaults.
    #[doc(hidden)]
    pub fn hook_common_input(&self, event: &str) -> crate::hooks::CommonHookInput {
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
            prompt_id: crate::hooks::resolve_prompt_id(&session_id),
            session_id,
            transcript_path,
            cwd,
            permission_mode: crate::hooks::PermissionMode::Default,
            effort: crate::hooks::resolve_effort(),
            hook_event_name: event.to_string(),
            agent_id: Some(self.agent_id.clone()),
            agent_type: None,
        }
    }

    /// Fire `PostCompact` + `SessionStart(source=compact)` after a real
    /// compaction. Both observation events; any `additionalContext` they return
    /// is queued for the next round's reminder suffix.
    async fn fire_compaction_hooks(
        &self,
        tier: u8,
        tokens_after: u32,
        model: &str,
        trigger: crate::hooks::CompactTrigger,
    ) {
        use crate::hooks::{HookDispatcher, HookEvent, HookInput};

        // Failover rebuilds the agent and re-runs compaction per retry from the
        // same history (identical tier + tokens_after → identical key, deduped);
        // a genuinely distinct compaction differs in tier or tokens_after and
        // fires even within the window.
        let sid = self.session_id.clone().unwrap_or_default();
        let dedup_key = format!("{}:{sid}:{tier}:{tokens_after}", trigger.as_str());
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
            trigger,
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
            session_title: None,
        };
        let out = HookDispatcher::dispatch(HookEvent::SessionStart, start).await;
        if let Some(extra) = out.merged_additional_context() {
            self.push_pending_hook_context(extra);
        }
    }

    /// Non-streaming LLM call for context summarization.
    /// If a CompactionProvider is configured, tries it first; on failure falls
    /// back to the conversation model using the same independent summarizer
    /// request shape and bounded retry policy.
    async fn summarize_with_model(&self, prompt: &str) -> Result<String> {
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
                Ok(summary) => {
                    match crate::context_compact::validate_summarization_output(&summary) {
                        Ok(()) => return Ok(summary),
                        Err(error) => {
                            app_warn!(
                                "agent",
                                "summarize",
                                "CompactionProvider '{}' returned an invalid summary: {}; falling back to conversation model",
                                provider.name(),
                                error
                            );
                        }
                    }
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

        // Do not reuse the main chat snapshot here. A cached side query would
        // send the old history once as the cached prefix and a second time in
        // `prompt`, while also demoting the summarizer system contract to a
        // user message. Build an independent one-shot request instead. When
        // the conversation provider has normal failover context, reuse the
        // dedicated summarizer wrapper so transient API failures get its
        // bounded retry policy without changing request semantics.
        if let (Some(provider_config), Some(session_id)) =
            (self.provider_config.clone(), self.session_id.clone())
        {
            let provider = DedicatedModelProvider::new(
                provider_config,
                self.provider.model().to_string(),
                session_id,
                self.user_agent.clone(),
                format!("conversation model {}", self.provider.model()),
                self.context_window,
            );
            return crate::context_compact::CompactionProvider::summarize(
                &provider,
                prompt,
                self.compact_config.summary_max_tokens,
            )
            .await;
        }

        // Legacy/test constructors without provider metadata cannot safely
        // rotate credentials; keep the independent single-attempt path.
        let mut usage_event =
            crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_SUMMARIZE);
        usage_event.operation = Some("context.summarize_direct".to_string());
        usage_event.source = Some("context_compact".to_string());
        usage_event.provider_id = self.provider_config.as_ref().map(|p| p.id.clone());
        usage_event.provider_name = self.provider_config.as_ref().map(|p| p.name.clone());
        usage_event.model_id = Some(self.provider.model().to_string());
        usage_event.session_id = self.session_id.clone();
        usage_event.agent_id = Some(self.agent_id.clone());
        summarize_direct(
            &self.provider,
            &self.user_agent,
            prompt,
            self.compact_config.summary_max_tokens,
            Some(usage_event),
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
    pub fn normalize_history_for_anthropic(
        history: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        for item in history {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                // Skip OpenAI Responses reasoning items (encrypted, Anthropic can't use them)
                "reasoning" => continue,
                // Skip Responses API tool items (Anthropic uses tool_use/tool_result)
                "function_call"
                | "function_call_output"
                | "tool_search_call"
                | "tool_search_output"
                | "additional_tools" => continue,
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
                            Self::push_anthropic_normalized_message(
                                &mut result,
                                json!({ "role": role, "content": text }),
                            );
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
                    Self::push_anthropic_normalized_message(&mut result, msg);
                }
            }
        }
        result
    }

    fn push_anthropic_normalized_message(
        messages: &mut Vec<serde_json::Value>,
        msg: serde_json::Value,
    ) {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            messages.push(msg);
            return;
        }

        if let Some(last) = messages.last_mut() {
            if last.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                let Some(new_content) = msg.get("content").cloned() else {
                    messages.push(msg);
                    return;
                };
                let merged = match (last.get("content").cloned(), new_content) {
                    (Some(serde_json::Value::String(old)), serde_json::Value::String(new)) => {
                        serde_json::Value::String(format!("{}\n\n{}", old, new))
                    }
                    (
                        Some(serde_json::Value::Array(mut old_arr)),
                        serde_json::Value::Array(new_arr),
                    ) => {
                        old_arr.extend(new_arr);
                        serde_json::Value::Array(old_arr)
                    }
                    (Some(serde_json::Value::Array(mut old_arr)), serde_json::Value::String(s)) => {
                        old_arr.push(json!({"type": "text", "text": s}));
                        serde_json::Value::Array(old_arr)
                    }
                    (Some(serde_json::Value::String(old)), serde_json::Value::Array(new_arr)) => {
                        let mut arr = vec![json!({"type": "text", "text": old})];
                        arr.extend(new_arr);
                        serde_json::Value::Array(arr)
                    }
                    (_, other) => other,
                };
                last["content"] = merged;
                return;
            }
        }

        messages.push(msg);
    }

    /// Normalize conversation history for OpenAI Chat Completions API.
    /// Converts foreign format items (Responses API / Anthropic) to Chat format.
    pub fn normalize_history_for_chat(history: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        for item in history {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                // Skip OpenAI Responses reasoning items
                "reasoning" => continue,
                // Skip Responses API tool items (Chat uses tool_calls array)
                "function_call"
                | "function_call_output"
                | "tool_search_call"
                | "tool_search_output"
                | "additional_tools" => continue,
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
    pub fn normalize_history_for_responses(
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
                "message"
                | "function_call"
                | "function_call_output"
                | "tool_search_call"
                | "tool_search_output"
                | "additional_tools" => {
                    result.push(item.clone());
                }
                _ => {
                    // Role-based messages (from Anthropic/Chat)
                    if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                        // A Responses request may itself use a role message
                        // whose content is an array of native input blocks.
                        // This is the exact shape retained in the cache-safe
                        // main-request snapshot, so do not run it through the
                        // Anthropic text-only extractor (which would discard
                        // both `input_text` and `input_image`).
                        let is_native_responses_role_message =
                            item.get("role").and_then(|role| role.as_str()).is_some()
                                && !content_arr.is_empty()
                                && content_arr.iter().all(|block| {
                                    matches!(
                                        block.get("type").and_then(|kind| kind.as_str()),
                                        Some("input_text" | "input_image" | "input_file")
                                    )
                                });
                        if is_native_responses_role_message {
                            result.push(item.clone());
                            continue;
                        }
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
    pub fn push_user_message(
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
    usage_event: Option<crate::model_usage::ModelUsageEvent>,
) -> Result<String> {
    use crate::context_compact::SUMMARIZATION_SYSTEM_PROMPT;

    let started = std::time::Instant::now();
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
                user_content: None,
            },
        )
        .await;

    let result = match result {
        Ok(result) => result,
        Err(e) => {
            if let Some(mut event) = usage_event {
                event.duration_ms = Some(started.elapsed().as_millis() as u64);
                event.success = false;
                event.error = Some(e.to_string());
                event.metadata = Some(serde_json::json!({ "max_tokens": max_tokens }));
                crate::model_usage::record_model_usage_best_effort(event);
            }
            return Err(e);
        }
    };

    if let Some(mut event) = usage_event {
        if result.usage.input_coverage.is_present() {
            event.input_tokens = Some(result.usage.input_tokens);
            event.cache_creation_input_tokens = Some(result.usage.cache_creation_input_tokens);
            event.cache_read_input_tokens = Some(result.usage.cache_read_input_tokens);
        }
        if result.usage.output_coverage.is_present() {
            event.output_tokens = Some(result.usage.output_tokens);
        }
        event.duration_ms = Some(started.elapsed().as_millis() as u64);
        event.metadata = Some(serde_json::json!({ "max_tokens": max_tokens }));
        crate::model_usage::record_model_usage_best_effort(event);
    }

    crate::context_compact::validate_summarization_output(&result.text)
        .map_err(|error| anyhow::anyhow!("Invalid summarization response: {error}"))?;
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
/// [`FailoverPolicy::summarize_default`] — Tier 3 gets bounded transient
/// retries without changing credential affinity or request semantics.
pub struct DedicatedModelProvider {
    provider_config: std::sync::Arc<crate::provider::ProviderConfig>,
    model_id: String,
    session_id: String,
    user_agent: String,
    display_name: String,
    context_window: u32,
}

impl DedicatedModelProvider {
    pub(crate) fn new(
        provider_config: std::sync::Arc<crate::provider::ProviderConfig>,
        model_id: String,
        session_id: String,
        user_agent: String,
        display_name: String,
        context_window: u32,
    ) -> Self {
        Self {
            provider_config,
            model_id,
            session_id,
            user_agent,
            display_name,
            context_window,
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
        let session_id_for_usage = self.session_id.clone();

        execute_with_failover(
            provider_config,
            &self.session_id,
            FailoverPolicy::summarize_default(),
            None,
            |profile| {
                // profile is `Option<&AuthProfile>`; clone to own it across
                // the `.await` inside build_llm_provider (Codex branch).
                let profile_owned = profile.cloned();
                let session_id_for_usage = session_id_for_usage.clone();
                async move {
                    let provider = AssistantAgent::build_llm_provider(
                        provider_config,
                        model_id,
                        profile_owned.as_ref(),
                    )
                    .await?;
                    let mut usage_event = crate::model_usage::ModelUsageEvent::new(
                        crate::model_usage::KIND_SUMMARIZE,
                    );
                    usage_event.operation = Some("context.dedicated_summarize".to_string());
                    usage_event.source = Some("context_compact".to_string());
                    usage_event.provider_id = Some(provider_config.id.clone());
                    usage_event.provider_name = Some(provider_config.name.clone());
                    usage_event.model_id = Some(model_id.to_string());
                    usage_event.session_id = Some(session_id_for_usage);
                    summarize_direct(&provider, user_agent, prompt, max_tokens, Some(usage_event))
                        .await
                }
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("dedicated summarize: {}", e))
    }

    fn name(&self) -> &str {
        &self.display_name
    }

    fn context_window(&self) -> Option<u32> {
        (self.context_window > 0).then_some(self.context_window)
    }
}

/// Parse `"providerId:modelId"` and construct a `DedicatedModelProvider`.
/// Returns `None` (with a warning log) if the format is invalid or the provider is not found/disabled.
///
/// `session_id` is used as the failover sticky/cooldown key so summarize
/// cooldowns are scoped to one session (and inherit cross-call sticky
/// affinity within that session).
// pub：ha-acp 特征 crate 的 ACP stdio server（acp/agent.rs）复用同一
// compaction provider 构建路径，不 fork。
pub fn build_compaction_provider(
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
    let model_config = prov_config.model_config(model_id)?;
    let display_name = format!("{}:{}", prov_config.name, model_id);

    Some(DedicatedModelProvider::new(
        std::sync::Arc::new(prov_config.clone()),
        model_id.to_string(),
        session_id.to_string(),
        prov_config.user_agent.clone(),
        display_name,
        model_config.context_window,
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
mod post_summary_budget_tests {
    use super::post_summary_ledger_reserve_chars;

    #[test]
    fn reserves_small_ledger_budget_for_file_only_touches() {
        assert_eq!(
            post_summary_ledger_reserve_chars(10_000, false, true),
            2_000
        );
    }

    #[test]
    fn reserves_larger_ledger_budget_for_live_runtime_state() {
        assert_eq!(post_summary_ledger_reserve_chars(10_000, true, true), 8_000);
    }

    #[test]
    fn reserves_nothing_without_live_state_or_file_touches() {
        assert_eq!(post_summary_ledger_reserve_chars(10_000, false, false), 0);
    }
}

#[cfg(test)]
mod mid_loop_compaction_tests {
    use super::{
        CompactionRunOptions, MidLoopCompactionState, MID_LOOP_MIN_ROUNDS_BETWEEN_SUMMARIES,
    };
    use crate::agent::AssistantAgent;
    use crate::context_compact::{
        CompactConfig, CompactResult, CompactionContext, ContextEngine, EmergencyCompactionContext,
    };
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };

    #[test]
    fn summary_attempt_cooldown_advances_on_attempt_not_success() {
        let mut state = MidLoopCompactionState::default();

        assert!(!state.summary_attempt_throttled(1));
        state.record_summary_attempt(1);

        assert!(state.summary_attempt_throttled(2));
        assert!(state.summary_attempt_throttled(1 + MID_LOOP_MIN_ROUNDS_BETWEEN_SUMMARIES - 1));
        assert!(!state.summary_attempt_throttled(1 + MID_LOOP_MIN_ROUNDS_BETWEEN_SUMMARIES));

        state.record_summary_attempt(1 + MID_LOOP_MIN_ROUNDS_BETWEEN_SUMMARIES);
        assert!(state.summary_attempt_throttled(99));
    }

    struct ForceTier3Engine;

    impl ContextEngine for ForceTier3Engine {
        fn compact_sync(
            &self,
            _messages: &mut Vec<Value>,
            _ctx: &CompactionContext<'_>,
        ) -> CompactResult {
            CompactResult {
                tier_applied: 3,
                tokens_before: 10_000,
                tokens_after: 10_000,
                messages_affected: 0,
                description: "summarization_needed".to_string(),
                details: None,
                manifest: None,
            }
        }

        fn emergency_compact(
            &self,
            _messages: &mut Vec<Value>,
            _ctx: &EmergencyCompactionContext<'_>,
        ) -> CompactResult {
            panic!("emergency compaction should not run in this test")
        }
    }

    struct CountingTier2Engine {
        calls: Arc<AtomicUsize>,
    }

    impl ContextEngine for CountingTier2Engine {
        fn compact_sync(
            &self,
            _messages: &mut Vec<Value>,
            _ctx: &CompactionContext<'_>,
        ) -> CompactResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            CompactResult {
                tier_applied: 2,
                tokens_before: 10_000,
                tokens_after: 5_000,
                messages_affected: 1,
                description: "tool_results_hard_cleared".to_string(),
                details: None,
                manifest: None,
            }
        }

        fn emergency_compact(
            &self,
            _messages: &mut Vec<Value>,
            _ctx: &EmergencyCompactionContext<'_>,
        ) -> CompactResult {
            panic!("emergency compaction should not run in this test")
        }
    }

    struct MutatingProjectionEngine {
        tier: u8,
        description: &'static str,
    }

    impl ContextEngine for MutatingProjectionEngine {
        fn compact_sync(
            &self,
            messages: &mut Vec<Value>,
            _ctx: &CompactionContext<'_>,
        ) -> CompactResult {
            let changed = messages.iter_mut().any(|message| {
                if message.get("role").and_then(Value::as_str) == Some("tool") {
                    message["content"] = json!("[request projection only]");
                    true
                } else {
                    false
                }
            });
            CompactResult {
                tier_applied: self.tier,
                tokens_before: 10_000,
                tokens_after: 5_000,
                messages_affected: usize::from(changed),
                description: self.description.to_string(),
                details: None,
                manifest: None,
            }
        }

        fn emergency_compact(
            &self,
            _messages: &mut Vec<Value>,
            _ctx: &EmergencyCompactionContext<'_>,
        ) -> CompactResult {
            panic!("emergency compaction should not run in this test")
        }
    }

    struct StaticSummaryProvider;

    #[async_trait::async_trait]
    impl crate::context_compact::CompactionProvider for StaticSummaryProvider {
        async fn summarize(&self, _prompt: &str, _max_tokens: u32) -> anyhow::Result<String> {
            Ok(concat!(
                "## Primary Request and Success Criteria\nsummary ok\n\n",
                "## Current Execution State\nsummary ok\n\n",
                "## Decisions and Rationale\nsummary ok\n\n",
                "## Files, Symbols, and Artifacts\nsummary ok\n\n",
                "## Tool Results Worth Preserving\nsummary ok\n\n",
                "## Errors, Failed Attempts, and Fixes\nsummary ok\n\n",
                "## User Feedback and Constraints\nsummary ok\n\n",
                "## Pending Work and Next Action\nsummary ok\n\n",
                "## Trust Boundaries and Security Notes\nsummary ok",
            )
            .to_string())
        }

        fn name(&self) -> &str {
            "static-test-summary"
        }
    }

    struct CapturingSummaryProvider {
        prompt: Arc<Mutex<String>>,
    }

    #[async_trait::async_trait]
    impl crate::context_compact::CompactionProvider for CapturingSummaryProvider {
        async fn summarize(&self, prompt: &str, _max_tokens: u32) -> anyhow::Result<String> {
            *self.prompt.lock().unwrap() = prompt.to_string();
            StaticSummaryProvider.summarize(prompt, _max_tokens).await
        }

        fn name(&self) -> &str {
            "capturing-test-summary"
        }
    }

    struct FailingSummaryProvider;

    #[async_trait::async_trait]
    impl crate::context_compact::CompactionProvider for FailingSummaryProvider {
        async fn summarize(&self, _prompt: &str, _max_tokens: u32) -> anyhow::Result<String> {
            anyhow::bail!("synthetic summary failure")
        }

        fn name(&self) -> &str {
            "failing-test-summary"
        }
    }

    #[tokio::test]
    async fn synchronous_compaction_changes_only_request_projection() {
        let mut agent = AssistantAgent::new_anthropic("test-key");
        agent.set_context_engine(Arc::new(MutatingProjectionEngine {
            tier: 2,
            description: "context_pruned",
        }));
        let mut canonical = vec![
            json!({"role":"user","content":"inspect"}),
            json!({"role":"tool","tool_call_id":"call_1","content":"FULL_EFFECTIVE_RESULT"}),
        ];
        let mut request_projection = canonical.clone();

        let outcome = agent
            .run_compaction_with_options(
                &mut request_projection,
                &mut canonical,
                "system",
                "test-model",
                1024,
                &|_| {},
                CompactionRunOptions::turn_start(None),
            )
            .await;

        assert_eq!(outcome.tier_applied, 2);
        assert_eq!(
            request_projection[1]["content"],
            "[request projection only]"
        );
        assert_eq!(canonical[1]["content"], "FULL_EFFECTIVE_RESULT");
    }

    #[tokio::test]
    async fn tier3_summarizes_canonical_not_trimmed_request_projection() {
        let captured = Arc::new(Mutex::new(String::new()));
        let mut agent = AssistantAgent::new_anthropic("test-key");
        agent.set_context_engine(Arc::new(MutatingProjectionEngine {
            // A custom engine may signal summarization by description while
            // reporting its highest synchronous tier. Successful async
            // installation must normalize the public tier to Tier 3.
            tier: 2,
            description: "summarization_needed",
        }));
        agent.set_compaction_provider(Some(Arc::new(CapturingSummaryProvider {
            prompt: captured.clone(),
        })));
        agent.set_compact_config(CompactConfig {
            preserve_recent_rounds: 1,
            ..Default::default()
        });
        let mut canonical = vec![
            json!({"role":"user","content":"inspect"}),
            json!({"role":"assistant","content":"calling","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]}),
            json!({"role":"tool","tool_call_id":"call_1","content":"FULL_EFFECTIVE_RESULT"}),
            json!({"role":"user","content":"continue"}),
            json!({"role":"assistant","content":"continuing"}),
        ];
        let mut request_projection = canonical.clone();

        let outcome = agent
            .run_compaction_with_options(
                &mut request_projection,
                &mut canonical,
                "system",
                "test-model",
                1024,
                &|_| {},
                CompactionRunOptions::turn_start(None),
            )
            .await;

        assert!(outcome.summary_applied);
        assert_eq!(outcome.tier_applied, 3);
        let prompt = captured.lock().unwrap().clone();
        assert!(prompt.contains("FULL_EFFECTIVE_RESULT"));
        assert!(!prompt.contains("[request projection only]"));
        assert_eq!(canonical, request_projection);
        assert!(serde_json::to_string(&canonical)
            .unwrap()
            .contains("summary ok"));
    }

    #[tokio::test]
    async fn failed_tier3_keeps_canonical_while_retaining_safe_request_projection() {
        let mut agent = AssistantAgent::new_anthropic("test-key");
        agent.set_context_engine(Arc::new(MutatingProjectionEngine {
            tier: 2,
            description: "summarization_needed",
        }));
        agent.set_compaction_provider(Some(Arc::new(FailingSummaryProvider)));
        // Force the independent summary-capacity preflight to fail before any
        // network fallback can run; this is a deterministic failure fixture.
        agent.context_window = 1;
        agent.set_compact_config(CompactConfig {
            preserve_recent_rounds: 1,
            ..Default::default()
        });
        let original = vec![
            json!({"role":"user","content":"inspect"}),
            json!({"role":"assistant","content":"calling","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{}"}}]}),
            json!({"role":"tool","tool_call_id":"call_1","content":"FULL_EFFECTIVE_RESULT"}),
            json!({"role":"user","content":"continue"}),
            json!({"role":"assistant","content":"continuing"}),
        ];
        let mut canonical = original.clone();
        let mut request_projection = original.clone();

        let outcome = agent
            .run_compaction_with_options(
                &mut request_projection,
                &mut canonical,
                "system",
                "test-model",
                1024,
                &|_| {},
                CompactionRunOptions::turn_start(None),
            )
            .await;

        assert!(!outcome.summary_applied);
        assert_eq!(canonical, original);
        assert_eq!(
            request_projection[2]["content"],
            "[request projection only]"
        );
    }

    #[tokio::test]
    async fn summary_throttle_still_allows_sync_compaction() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = AssistantAgent::new_anthropic("test-key");
        // Use a genuinely constrained request budget instead of an invalid
        // zero threshold to enter the mid-loop compaction path.
        agent.context_window = 1_024;
        agent.set_context_engine(Arc::new(CountingTier2Engine {
            calls: calls.clone(),
        }));
        agent.set_compact_config(CompactConfig {
            summarization_threshold: 0.71,
            ..Default::default()
        });

        let mut state = MidLoopCompactionState::default();
        state.record_summary_attempt(1);
        assert!(state.summary_attempt_throttled(2));

        let mut messages = vec![json!({"role": "user", "content": "continue"})];
        let mut canonical = messages.clone();
        let on_delta = |_event: &str| {};
        let outcome = agent
            .maybe_compact_between_tool_rounds(
                &mut messages,
                &mut canonical,
                "system",
                &[],
                "test-model",
                1024,
                Arc::new(AtomicBool::new(false)),
                &mut state,
                2,
                &on_delta,
                None,
            )
            .await
            .expect("mid-loop checkpoint");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.tier_applied, 2);
        assert!(outcome.changed_history);
        assert_eq!(state.summary_attempt_count, 1);
    }

    #[tokio::test]
    async fn dedicated_provider_is_allowed_for_mid_loop_summary() {
        let mut agent = AssistantAgent::new_anthropic("test-key");
        // Keep enough room for the summary request itself while putting the
        // conversation above the configured Tier 3 watermark.
        agent.context_window = 12_000;
        agent.set_context_engine(Arc::new(ForceTier3Engine));
        agent.set_compaction_provider(Some(Arc::new(StaticSummaryProvider)));
        agent.set_compact_config(CompactConfig {
            summarization_threshold: 0.71,
            summary_max_tokens: 256,
            preserve_recent_rounds: 1,
            ..Default::default()
        });

        let mut state = MidLoopCompactionState::default();
        let mut messages = vec![
            json!({"role": "user", "content": "inspect the project"}),
            json!({"role": "assistant", "content": format!(
                "found context worth preserving: {}",
                "a".repeat(22_000)
            )}),
            json!({"role": "user", "content": "continue"}),
            json!({"role": "assistant", "content": "continuing"}),
        ];
        let mut canonical = messages.clone();
        let on_delta = |_event: &str| {};
        let outcome = agent
            .maybe_compact_between_tool_rounds(
                &mut messages,
                &mut canonical,
                "system",
                &[],
                "test-model",
                1024,
                Arc::new(AtomicBool::new(false)),
                &mut state,
                1,
                &on_delta,
                None,
            )
            .await
            .expect("mid-loop checkpoint");

        assert!(outcome.summary_applied);
        assert!(serde_json::to_string(&messages)
            .expect("serialize messages")
            .contains("summary ok"));
        assert_eq!(canonical, messages);
        assert_eq!(state.summary_attempt_count, 1);
    }

    #[tokio::test]
    async fn incognito_tier3_skips_recovery_and_runtime_ledger_injection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("secret.txt");
        std::fs::write(&path, "SECRET_SNAPSHOT").expect("write temp file");

        let mut agent = AssistantAgent::new_anthropic("test-key");
        agent.set_context_engine(Arc::new(ForceTier3Engine));
        agent.set_compaction_provider(Some(Arc::new(StaticSummaryProvider)));
        agent.incognito_cached.store(true, Ordering::Relaxed);
        agent.set_compact_config(crate::context_compact::CompactConfig {
            preserve_recent_rounds: 1,
            ..Default::default()
        });

        let write_args = json!({
            "path": path.to_string_lossy(),
            "content": "SECRET_SNAPSHOT",
        })
        .to_string();
        let mut messages = vec![
            json!({"role": "user", "content": "write a file"}),
            json!({
                "role": "assistant",
                "content": "ok",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": crate::tool_defs::TOOL_WRITE,
                        "arguments": write_args,
                    }
                }]
            }),
            json!({"role": "tool", "tool_call_id": "call_1", "content": "wrote file"}),
            json!({"role": "user", "content": "continue"}),
            json!({"role": "assistant", "content": "continuing"}),
        ];
        let mut canonical = messages.clone();
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let events_sink = events.clone();
        let on_delta = move |event: &str| {
            events_sink.lock().unwrap().push(event.to_string());
        };

        let outcome = agent
            .run_compaction_with_options(
                &mut messages,
                &mut canonical,
                "system",
                "test-model",
                1024,
                &on_delta,
                CompactionRunOptions::turn_start(None),
            )
            .await;

        assert!(outcome.summary_applied);
        let serialized = serde_json::to_string(&messages).expect("serialize messages");
        assert_eq!(canonical, messages);
        assert!(serialized.contains("summary ok"));
        assert!(!serialized.contains("SECRET_SNAPSHOT"));
        assert!(!serialized.contains("untrusted_file_snapshot"));
        assert!(!serialized.contains("Runtime Ledger"));
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

    #[test]
    fn native_tool_search_items_only_survive_responses_normalization() {
        let history = vec![
            json!({ "type": "tool_search_call", "execution": "server", "status": "completed" }),
            json!({ "type": "tool_search_output", "execution": "server", "status": "completed", "tools": [] }),
        ];
        assert_eq!(
            AssistantAgent::normalize_history_for_responses(&history).len(),
            2
        );
        assert!(AssistantAgent::normalize_history_for_chat(&history).is_empty());
        assert!(AssistantAgent::normalize_history_for_anthropic(&history).is_empty());
    }

    #[test]
    fn anthropic_normalization_merges_responses_interim_assistant_text() {
        let history = vec![
            json!({"role": "user", "content": "check this"}),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "I am checking it." }]
            }),
            json!({
                "type": "function_call",
                "call_id": "call_1",
                "name": "read",
                "arguments": "{}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "file contents"
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "It is fixed." }]
            }),
            json!({"role": "user", "content": "continue"}),
        ];

        let normalized = AssistantAgent::normalize_history_for_anthropic(&history);

        assert_eq!(
            normalized
                .iter()
                .map(|v| v.get("role").and_then(|r| r.as_str()).unwrap_or(""))
                .collect::<Vec<_>>(),
            vec!["user", "assistant", "user"],
            "normalized history must preserve Anthropic role alternation: {normalized:?}"
        );
        assert_eq!(
            normalized[1].get("content").and_then(|c| c.as_str()),
            Some("I am checking it.\n\nIt is fixed.")
        );
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
