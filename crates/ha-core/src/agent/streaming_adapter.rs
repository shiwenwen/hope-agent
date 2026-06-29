//! StreamingChatAdapter trait: provider-agnostic streaming chat abstraction.
//!
//! Each provider (Anthropic / OpenAIChat / OpenAIResponses / Codex) implements
//! this trait, encapsulating body construction, HTTP send, SSE decoding, and
//! history persistence in a provider-specific shape. The public tool loop
//! ([`super::streaming_loop::AssistantAgent::run_streaming_chat`]) orchestrates
//! compaction, cache snapshot, tool dispatch, microcompact, and event emission
//! in a provider-agnostic way.
//!
//! Phase 2 of the LLM call unification — Phase 1 was [`super::llm_adapter`]
//! for one-shot side-query / summarization calls. See
//! `docs/architecture/side-query.md` for the architecture overview.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::api_types::FunctionCallItem;
use super::types::{ChatUsage, ProviderFormat};
use crate::tools::ToolProvider;

/// Provider-agnostic request payload for one tool-loop round.
///
/// All provider-specific concerns (cache_control, system block ordering,
/// reasoning config shape) are constructed inside the adapter from these
/// inputs. The public orchestrator stays oblivious to body shape differences.
pub(crate) struct RoundRequest<'a> {
    /// Static system prompt (cache-friendly prefix). Cached by Anthropic /
    /// auto-cached by OpenAI as the prompt prefix.
    pub system_prompt: &'a str,
    /// Dynamic awareness suffix (Phase B) — provider injects as a separate
    /// cache breakpoint so its churn doesn't invalidate the static prefix.
    pub awareness_suffix: Option<&'a str>,
    /// Active Memory recall sentence (Phase B1) — third independent cache
    /// breakpoint. Same rationale as `awareness_suffix`.
    pub active_memory_suffix: Option<&'a str>,
    /// Passive related-notes block (read bridge ③, Phase 3). Note titles from the
    /// accessible KBs. Appended as a plain system block WITHOUT `cache_control` on
    /// Anthropic (the 4 breakpoints are already taken — prefix, awareness,
    /// active_memory, last tool); it changes per user message so caching would
    /// rarely hit anyway. Untrusted (never instructions).
    pub related_notes_suffix: Option<&'a str>,
    /// Per-round task tracker reminder. Appended as the last system block
    /// (without `cache_control` on Anthropic to stay under the 4-breakpoint
    /// cap). Lifecycle differs from awareness/active_memory: cheap pure-DB
    /// derivation each round, no side_query, no TTL — the goal is to keep
    /// in_progress / pending tasks visible to the model so it doesn't drop
    /// them on the floor before final reply.
    pub task_reminder_suffix: Option<&'a str>,
    /// Tool schemas for this round (already filtered for plan mode / denied
    /// tools / skill allowlist by `build_tool_schemas`).
    pub tool_schemas: &'a [Value],
    /// Conversation history prepared for API: `_oc_round` metadata stripped.
    pub history_for_api: &'a [Value],
    /// Resolved reasoning effort for this round (live or fallback).
    pub reasoning_effort: Option<&'a str>,
    /// Sampling temperature override (None = API default).
    pub temperature: Option<f64>,
    /// Max output tokens for this round.
    pub max_tokens: u32,
    /// On the final allowed round we omit `tools` from the request to force
    /// a text response — otherwise the model may pick a tool, the loop
    /// executes it and exits before the result is sent back to the model.
    pub is_final_round: bool,
    /// Round index (0-based) — used for logging and `_oc_round` stamping.
    pub round: u32,
}

/// Provider-agnostic outcome of one round (after SSE decoding completes).
pub(crate) struct RoundOutcome {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<FunctionCallItem>,
    pub usage: ChatUsage,
    /// Time-to-first-token (ms from request start).
    pub ttft_ms: Option<u64>,
    /// Anthropic-only: stop_reason ("tool_use" / "end_turn" / "max_tokens" / ...).
    /// Other providers leave this `None` and rely on `tool_calls.is_empty()`.
    pub stop_reason: Option<String>,
}

/// One executed tool call, ready to be appended to history by the adapter.
///
/// `media_items` and `is_error` are intentionally not surfaced here — the
/// orchestrator already used them to fire `emit_tool_result` events before
/// constructing this struct. Adapters store `clean_result` verbatim in history;
/// normal tool execution has already materialized inline image markers where
/// appropriate, and any remaining `__IMAGE_BASE64__` / `__IMAGE_FILE__`
/// expansion happens only on the outgoing API request so persisted history
/// never holds provider-specific image blocks.
pub(crate) struct ExecutedTool {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    /// Tool result with the `__MEDIA_ITEMS__` prefix already stripped.
    pub clean_result: String,
}

/// Side-output captured from a single tool dispatch (metadata, plus any
/// trailing fields we add later). Travels alongside the result + duration to
/// the streaming loop and the persister so the diff panel sees the same shape
/// from both the live event channel and the SQLite history.
#[derive(Debug, Clone, Default)]
pub(crate) struct ToolDispatchSideOutput {
    pub metadata: Option<serde_json::Value>,
    /// Effective tool arguments after a `PreToolUse` hook rewrote them via
    /// `updatedInput`. `None` when no rewrite happened — the caller keeps the
    /// model's original arguments. When `Some`, the orchestrator MUST use this
    /// value for the live UI tool-call display, the persisted history row,
    /// and the `PostToolUse` hook input so the rewrite isn't audited away.
    /// Serialized JSON string (matches `tc.arguments` shape).
    pub effective_arguments: Option<String>,
}

#[async_trait]
pub(crate) trait StreamingChatAdapter: Send + Sync {
    /// Provider format tag — drives `build_full_system_prompt(model, label)`,
    /// log line source identifiers, and error messages. Stable string keys
    /// (used by external prompts), so encoded as enum variants here.
    fn provider_format(&self) -> ProviderFormat;

    /// Tool schema variant to request from the tool registry. Anthropic uses
    /// the native Anthropic shape; the three OpenAI flavors share the OpenAI
    /// schema variant.
    fn tool_provider(&self) -> ToolProvider;

    /// Normalize history that may have been persisted from a different
    /// provider (failover / model switch / first turn after switching agent).
    /// Encapsulates the `normalize_history_for_*` helpers so the orchestrator
    /// stays unaware of cross-provider format quirks.
    fn normalize_history(&self, history: &mut Vec<Value>);

    /// One API round: construct body → POST → decode SSE → return structured
    /// result. All cancel polling and `on_delta` token forwarding happens
    /// inside this method (provider-specific SSE event types).
    ///
    /// `on_delta` uses `&dyn Fn` (not `&impl Fn`) because trait methods
    /// cannot be generic over closure types while remaining object-safe.
    /// `Send + Sync` is required because `async_trait` desugars to a
    /// `BoxFuture<'_, Send>` and the closure may be captured across awaits.
    async fn chat_round(
        &self,
        client: &reqwest::Client,
        req: RoundRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<RoundOutcome>;

    /// Append this round's assistant output + executed tool results to
    /// history in this provider's native shape:
    ///  - Anthropic: `{role:assistant, content:[thinking,text,tool_use...]}`
    ///    + `{role:user, content:[tool_result...]}`
    ///  - OpenAI Chat: assistant message with `tool_calls` + role=tool messages
    ///  - Responses/Codex: optional assistant `message` text, followed by
    ///    `function_call` + `function_call_output` items (reasoning items are
    ///    intentionally not replayed; both providers run with `store: false`,
    ///    where stale `rs_*` ids 404 the next request)
    ///
    /// Implementations must use `crate::context_compact::push_and_stamp` to
    /// stamp the `_oc_round` metadata for compaction round-boundary alignment.
    fn append_round_to_history(
        &self,
        history: &mut Vec<Value>,
        round: u32,
        outcome: &RoundOutcome,
        executed: &[ExecutedTool],
    );

    /// Append the terminal assistant message (the no-tool exit round, not the
    /// full accumulated turn text) when the loop exits naturally or hits
    /// `max_rounds`. Earlier tool-round narration belongs to
    /// `append_round_to_history`; duplicating it here makes the next turn see
    /// the same user-facing update twice. Anthropic packs thinking + text into
    /// a content-block array; OpenAI Chat puts thinking in `reasoning_content`;
    /// Responses/Codex emits a `{type:message, role:assistant,
    /// content:[{type:output_text, text}]}` item.
    fn append_final_assistant(
        &self,
        history: &mut Vec<Value>,
        final_text: &str,
        last_thinking: &str,
    );

    /// Decide whether the tool loop should exit after this round's outcome.
    ///  - Anthropic: `stop_reason != Some("tool_use")` (model decided to stop)
    ///  - Others: `tool_calls.is_empty()` (model emitted text, no tools requested)
    fn loop_should_exit(&self, outcome: &RoundOutcome) -> bool;
}
