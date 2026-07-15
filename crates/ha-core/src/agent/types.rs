use std::sync::atomic::AtomicBool;

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};

use crate::agent_config::{AsyncToolPolicy, FilterConfig};
use crate::permission::SandboxMode;
use crate::provider::ThinkingStyle;

use super::active_memory::AgentConfigFingerprint;

/// Snapshot of the fields read from `agent.json` on every chat / tool-loop
/// iteration. Cached on `AssistantAgent` so we stop re-reading and re-parsing
/// agent.json 10+ times per chat turn.
#[derive(Debug, Clone)]
pub(super) struct AgentCapsCache {
    /// Fingerprint of the `agent.json` that produced this snapshot. When it
    /// changes, hot-path callers reload the snapshot so tool visibility and
    /// permission defaults follow Settings edits on the next turn.
    pub fingerprint: Option<AgentConfigFingerprint>,
    /// Per-agent non-Core tool switch overrides.
    pub agent_tool_filter: FilterConfig,
    pub sandbox_mode: SandboxMode,
    pub async_tool_policy: AsyncToolPolicy,
    /// Per-agent MCP master switch (mirrors `agent.json` `capabilities.mcpEnabled`).
    /// When false, all MCP tools are excluded from schema + system prompt.
    pub mcp_enabled: bool,
    /// Whether memory is enabled for this agent (mirrors `agent.json`
    /// `memory.enabled`). Drives Tier::Memory tool injection gate.
    pub memory_enabled: bool,
    /// Mirrors `agent.json` `capabilities.enableCustomToolApproval`. When false,
    /// `custom_approval_tools` is ignored.
    pub enable_custom_tool_approval: bool,
    /// Mirrors `agent.json` `capabilities.customApprovalTools`. Only consumed
    /// in Default permission mode.
    pub custom_approval_tools: Vec<String>,
}

impl Default for AgentCapsCache {
    fn default() -> Self {
        Self {
            fingerprint: None,
            sandbox_mode: SandboxMode::Off,
            async_tool_policy: AsyncToolPolicy::default(),
            mcp_enabled: true,
            agent_tool_filter: FilterConfig::default(),
            memory_enabled: true,
            enable_custom_tool_approval: false,
            custom_approval_tools: Vec::new(),
        }
    }
}

/// File/image attachment sent alongside a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime_type: String,
    /// Optional frontend provenance marker. Uploaded files are persisted into
    /// Hope Agent's attachment store; mention-derived references may point at
    /// user-selected local files and are intentionally not exposed through
    /// chat history attachment metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Base64-encoded file data (used for images — passed directly through IPC)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Absolute path to the file on disk (used for non-image files)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// For `source = "quote"` (file-browser "quote to chat"): the 1-based line
    /// range of the quoted snippet (e.g. `"12-20"`). Combined with `file_path`
    /// (the path) and `data` (the snippet text) to emit a `<file_reference>`
    /// block to the model. Not persisted as a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_lines: Option<String>,
    /// For `source = "message_quote"`: role of the selected conversation
    /// message. The inline body remains in `data`; no file is read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_role: Option<String>,
}

impl Attachment {
    /// Get base64-encoded data: use `data` field if present, otherwise read from `file_path`.
    pub(super) fn get_base64_data(&self) -> anyhow::Result<String> {
        if let Some(ref data) = self.data {
            return Ok(data.clone());
        }
        if let Some(ref path) = self.file_path {
            return read_and_encode_base64(path);
        }
        Err(anyhow::anyhow!(
            "Attachment '{}' has neither data nor file_path",
            self.name
        ))
    }
}

/// Read a file from disk and return its contents as a base64-encoded string.
pub(super) fn read_and_encode_base64(path: &str) -> anyhow::Result<String> {
    let data = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("Failed to read attachment '{}': {}", path, e))?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&data))
}

/// Supported LLM providers
pub enum LlmProvider {
    /// Anthropic Messages API
    Anthropic {
        api_key: String,
        base_url: String,
        model: String,
    },
    /// OpenAI Chat Completions API (/v1/chat/completions)
    OpenAIChat {
        api_key: String,
        base_url: String,
        model: String,
    },
    /// OpenAI Responses API (/v1/responses)
    OpenAIResponses {
        api_key: String,
        base_url: String,
        model: String,
    },
    /// Built-in Codex OAuth (ChatGPT subscription)
    Codex {
        access_token: String,
        account_id: String,
        model: String,
    },
}

impl LlmProvider {
    /// Model id this provider was constructed with — used by failover
    /// closures to rebuild a sibling `LlmProvider` for a different
    /// `AuthProfile` while keeping the same model.
    pub(super) fn model(&self) -> &str {
        match self {
            Self::Anthropic { model, .. }
            | Self::OpenAIChat { model, .. }
            | Self::OpenAIResponses { model, .. }
            | Self::Codex { model, .. } => model,
        }
    }
}

/// Dual-agent plan mode: Plan Agent (read-only + planning tools) vs Executing Agent (full tools + execution tracking).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PlanAgentMode {
    /// Normal mode, no plan restrictions
    #[default]
    Off,
    /// Plan Agent: allow-list based tool access + path-restricted write/edit
    PlanAgent {
        allowed_tools: Vec<String>,
        ask_tools: Vec<String>,
    },
    /// Executing Agent: full tool access + extra plan execution tools
    ExecutingAgent,
}

pub struct AssistantAgent {
    pub(super) provider: LlmProvider,
    /// Custom User-Agent header for API requests
    pub(super) user_agent: String,
    /// Thinking/reasoning parameter format
    pub(super) thinking_style: ThinkingStyle,
    /// Conversation history persisted across chat() calls
    pub(super) conversation_history: std::sync::Mutex<Vec<serde_json::Value>>,
    /// Current agent ID (for memory context loading)
    pub(super) agent_id: String,
    /// Extra context appended to the system prompt (e.g. cron execution context)
    pub(super) extra_system_context: Option<String>,
    /// Model context window size in tokens
    pub(super) context_window: u32,
    /// Context compaction configuration
    pub(super) compact_config: crate::context_compact::CompactConfig,
    /// Pluggable context compression engine (default: DefaultContextEngine)
    pub(super) context_engine: std::sync::Arc<dyn crate::context_compact::ContextEngine>,
    /// Optional dedicated summarization provider (Tier 3).
    /// When Some, tried first for summarization; falls back to side_query on failure.
    pub(super) compaction_provider:
        Option<std::sync::Arc<dyn crate::context_compact::CompactionProvider>>,
    /// Token estimate calibrator (updated with actual API usage)
    pub(super) token_calibrator: std::sync::Mutex<crate::context_compact::TokenEstimateCalibrators>,
    /// Session-scoped deferred tools already discovered by `tool_search`.
    /// Persisted for regular sessions and kept memory-only for incognito.
    pub(super) activated_tool_names: std::sync::Mutex<Vec<String>>,
    /// Current session ID (for sub-agent context)
    pub(super) session_id: Option<String>,
    /// Session database backing the current chat-engine turn. Most runtime
    /// paths use the global DB, but deterministic/eval runners can provide an
    /// isolated DB; agent-side session lookups must honor that source first.
    pub(crate) session_db: Option<std::sync::Arc<crate::session::SessionDB>>,
    /// Cached `sessions.incognito` flag for the current session. Refreshed at
    /// each turn boundary (`reset_chat_flags`) and on `set_session_id`; allows
    /// hot-path guards to avoid a SQLite round-trip per call.
    pub(crate) incognito_cached: std::sync::atomic::AtomicBool,
    /// Sub-agent nesting depth (0 = top-level)
    pub(super) subagent_depth: u32,
    /// Turn source for knowledge-base access scoping (design D10). Set per-turn
    /// by `configure_agent`; flows into `ToolExecContext.chat_source`.
    pub(super) chat_source: Option<crate::knowledge::KbAccessSource>,
    /// Origin of the whole call chain for KB access (design D10). Set per-turn
    /// by `configure_agent`; flows into `ToolExecContext.origin_chat_source`.
    /// Equals `chat_source` for top-level turns; a subagent carries its parent
    /// turn's origin so IM-origin chains can't launder access via `Subagent`.
    pub(super) origin_chat_source: Option<crate::knowledge::KbAccessSource>,
    /// IM identity of the lineage origin for the WS8 KB-access opt-in gate. Set
    /// per-turn by `configure_agent`; flows into `ToolExecContext.channel_kb_context`.
    /// `Some` only for IM-origin lineages (top-level IM turn or IM-origin subagent).
    pub(super) channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
    /// Run ID for steer mailbox (set only when running as a sub-agent)
    pub(super) steer_run_id: Option<String>,
    /// Tools denied for this agent (used for depth-based tool policy)
    pub(super) denied_tools: Vec<String>,
    /// Optional tool-visibility scope for this turn (see [`crate::tools::ToolScope`]).
    /// `Some(Knowledge)` trims the schema + system-prompt tool hints to the
    /// knowledge-space white-list. Orthogonal to `denied_tools` and chat source;
    /// purely narrows visibility, never widens KB access.
    pub(super) tool_scope: Option<crate::tools::ToolScope>,
    /// Active skill's allowed tools: when non-empty, only these tools are sent to the LLM.
    /// Set when a skill with `allowed-tools` frontmatter is activated.
    pub(super) skill_allowed_tools: Vec<String>,
    /// Cached `PlanModeState` this agent's resolved plan slots were
    /// derived from. Used by the streaming loop's mid-turn probe as the
    /// dirty bit: `Planning ↔ Review` and `Completed ↔ Off` both produce
    /// identical `PlanAgentMode` values, so a mode-only diff would miss
    /// these transitions even though their `extra_system_context` differs
    /// materially (Review embeds the just-submitted plan; Completed
    /// embeds the executed plan).
    pub(super) plan_state_cached: ArcSwap<crate::plan::PlanModeState>,
    /// Plan Agent / Executing Agent mode (dual-agent architecture).
    ///
    /// `ArcSwap` provides lock-free internal mutability so the streaming
    /// loop's mid-turn probe (which holds `&self`) can update the mode in
    /// place after `enter_plan_mode` flips backend state. Schema rebuild
    /// (`build_tool_schemas`) and permission ctx (`tool_context_with_usage`)
    /// both read the current snapshot so they stay in sync without manual
    /// threading.
    pub(super) plan_agent_mode: ArcSwap<PlanAgentMode>,
    /// Plan mode path-based allow rules — paired with `plan_agent_mode` and
    /// updated in lockstep so write/edit path-aware allow stays accurate
    /// after a mid-turn mode change.
    pub(super) plan_mode_allow_paths: ArcSwap<Vec<String>>,
    /// Plan-derived system-prompt segment, kept SEPARATE from the
    /// caller-supplied `extra_system_context` so a mid-turn plan-state
    /// flip can swap just this slice without losing the caller's context
    /// (cron task description, sub-agent role, etc.). Read by
    /// `build_full_system_prompt` which appends both. ArcSwap so the
    /// streaming loop's mid-turn probe can install fresh plan guidance
    /// without `&mut self`.
    pub(super) plan_extra_context: ArcSwap<Option<String>>,
    /// Pending hook-injected context: `additionalContext` from observation
    /// events that fire *outside* a round (PostCompact, SessionStart(compact),
    /// Notification). Drained into the next round's reminder suffix. ArcSwap so
    /// compaction / EventBus call sites can push without `&mut self`. Held on
    /// the live agent — a single `run_streaming_chat` never rebuilds it (only
    /// failover retries do, where losing best-effort observation context is
    /// acceptable).
    pub(super) pending_hook_context: ArcSwap<Vec<String>>,
    /// True when the mode was supplied externally by the spawn caller (e.g.
    /// `spawn_plan_subagent` with explicit `PlanAgent`) rather than read
    /// from this session's backend plan state. The streaming loop's
    /// mid-turn probe must NOT overwrite a locked mode — the spawn caller
    /// is the source of truth, not the (typically `Off`) child session
    /// backend state.
    pub(super) plan_agent_mode_externally_locked: AtomicBool,
    /// Temperature for LLM API calls (0.0–2.0). None = use API default.
    pub(super) temperature: Option<f64>,
    /// Cache-safe params from the last main chat request, used for side_query().
    /// Wrapped in Arc to avoid expensive deep clones on every chat turn.
    pub(super) cache_safe_params: std::sync::Mutex<Option<std::sync::Arc<CacheSafeParams>>>,
    /// Timestamp of the last successful memory extraction (or session start).
    pub(crate) last_extraction_at: std::sync::Mutex<std::time::Instant>,
    /// Accumulated token count since last extraction.
    pub(crate) tokens_since_extraction: std::sync::atomic::AtomicU32,
    /// Accumulated message count since last extraction.
    pub(crate) messages_since_extraction: std::sync::atomic::AtomicU32,
    /// Whether save_memory/update_core_memory was called in the current chat() round.
    /// Used for mutual exclusion with auto-extraction.
    pub(crate) manual_memory_saved: std::sync::atomic::AtomicBool,
    /// When true, automatically approve all tool calls (IM channel auto-approve mode).
    pub(super) auto_approve_tools: bool,
    /// When true, every tool-loop round re-reads the live reasoning effort from
    /// `AppState` so UI toggles / `/thinking` slash commands apply to the next API
    /// request without waiting for the next user turn. Main-chat agents opt in
    /// via `configure_agent`; subagents / side_query / memory_extract / cron
    /// leave it `false` so their caller-specified effort isn't silently
    /// overridden by the UI picker.
    pub(super) follow_global_reasoning_effort: bool,
    /// Timestamp of last Tier 2+ compaction (cache-TTL throttle, session-scoped).
    pub(crate) last_tier2_compaction_at: std::sync::Mutex<Option<std::time::Instant>>,
    /// Lazily-populated cache for fields read from `agent.json` on every
    /// chat/tool-loop iteration. Cleared by `set_agent_id`.
    pub(super) agent_caps_cache: std::sync::Mutex<Option<std::sync::Arc<AgentCapsCache>>>,
    /// Behavior awareness holder. Lazily created on first `chat()`
    /// call once we have a session id and the feature is enabled.
    pub(crate) awareness:
        std::sync::Mutex<Option<std::sync::Arc<crate::awareness::SessionAwareness>>>,
    /// Latest dynamic awareness suffix to append to the system prompt as
    /// a separate cache breakpoint. Rebuilt on each chat() turn by
    /// `prepare_dynamic_suffix`.
    pub(crate) awareness_suffix: std::sync::Mutex<Option<std::sync::Arc<String>>>,
    /// Active Memory per-agent runtime state (cache + inflight flags).
    /// Initialized once on construction and reused across all chat() turns.
    pub(crate) active_memory_state: std::sync::Arc<super::active_memory::ActiveMemoryState>,
    /// Latest Active Memory recall suffix to append to the system prompt as
    /// yet another independent cache breakpoint. Rebuilt every user turn by
    /// `refresh_active_memory_suffix` when the side_query completes in time.
    /// `None` means: nothing to inject this turn (empty shortlist, LLM said
    /// NONE, timeout, or feature disabled).
    pub(crate) active_memory_suffix: std::sync::Mutex<Option<std::sync::Arc<String>>>,
    /// Structured trace for the latest Active Memory suffix. This is not sent
    /// to the model; it powers UI explainability ("which memory was recalled")
    /// and future used-memory chips. Kept separate from the suffix so provider
    /// injection remains byte-for-byte compatible except for the recall text.
    pub(crate) active_memory_trace:
        std::sync::Mutex<Option<std::sync::Arc<super::active_memory::ActiveMemoryRecall>>>,
    /// Static memory refs injected through the cache-stable system prompt
    /// prefix for this turn (currently Pinned claims from Context Pack). These
    /// are persisted with the assistant row as `used_memory_refs` so the UI can
    /// explain long-term context even when Active Memory is disabled or empty.
    pub(crate) static_memory_refs: std::sync::Mutex<Vec<super::active_memory::UsedMemoryRef>>,
    /// Content-free metrics for the exact static memory snapshot used to build
    /// this turn's base prompt. Dynamic recall metrics are joined per round.
    pub(crate) static_memory_manifest:
        std::sync::Mutex<crate::memory::context_manifest::StaticMemoryContextManifest>,
    /// Session-stable Core Memory contents. File changes do not replace this
    /// snapshot until explicit reload/compaction or context identity changes.
    pub(crate) core_memory_snapshot: std::sync::Mutex<
        Option<std::sync::Arc<crate::memory::core_repository::CoreMemorySnapshot>>,
    >,
    /// Episode / Procedure candidates considered for this turn. These are not
    /// all injected into the model; high-confidence procedures may additionally
    /// enter `procedure_memory_suffix` as bounded soft workflow guidance.
    pub(crate) experience_memory_refs: std::sync::Mutex<Vec<super::active_memory::UsedMemoryRef>>,
    /// Temporal graph claim neighbors considered for this turn. These are
    /// trace-only candidate refs; they do not enter the prompt until the graph
    /// layer gets an explicit budgeted injection path.
    pub(crate) graph_memory_refs: std::sync::Mutex<Vec<super::active_memory::UsedMemoryRef>>,
    /// Latest Procedure Memory soft guidance block. Rebuilt every user turn by
    /// `refresh_experience_memory_context`. Only user-saved/promoted active
    /// procedures can enter this suffix; episodes remain trace-only.
    pub(crate) procedure_memory_suffix: std::sync::Mutex<Option<std::sync::Arc<String>>>,
    /// Per-turn retrieval-layer status ledger. Each retrieval bridge upserts
    /// its own layer while `run_streaming_chat` refreshes dynamic context; the
    /// final assistant row persists the merged trace as diagnostics only.
    pub(crate) retrieval_planner_layers:
        std::sync::Mutex<Vec<super::retrieval_planner::RetrievalPlannerLayerTrace>>,
    /// Query-derived, privacy-safe ranking context for the current turn. The
    /// raw query is never persisted in Retrieval Planner diagnostics.
    pub(crate) retrieval_planner_context:
        std::sync::Mutex<super::retrieval_planner::RetrievalPlannerDecisionContext>,
    /// Read bridge ③ per-agent runtime state (passive related-notes cache).
    pub(crate) related_notes_state: std::sync::Arc<super::related_notes::RelatedNotesState>,
    /// Latest passive related-notes suffix (note titles from the accessible KBs),
    /// injected as another independent block. Rebuilt every user turn by
    /// `refresh_related_notes_suffix`. `None` = nothing to inject (disabled,
    /// incognito, no accessible KB, or no hits).
    pub(crate) related_notes_suffix: std::sync::Mutex<Option<std::sync::Arc<String>>>,
    /// Per-turn coding policy profile (Phase 2.2). Deterministic, cheap, and
    /// injected outside the static prompt prefix so task-kind churn does not
    /// invalidate prompt-cache hits.
    pub(crate) coding_profile_suffix: std::sync::Mutex<Option<std::sync::Arc<String>>>,
    /// Structured trace for the latest passive related-notes suffix. Not sent
    /// to the model; persisted as `used_memory_refs` so users can see which
    /// knowledge notes were surfaced for this answer.
    pub(crate) related_notes_trace:
        std::sync::Mutex<Option<std::sync::Arc<super::related_notes::RelatedNotesRecall>>>,
    /// Per-turn memo of the resolved effective KB access map. `resolve_kb_access`
    /// is hit up to ~5× per turn (passive recall + the no-KB tool-schema gate +
    /// the `# Knowledge Bases` system-prompt section, the last built twice and
    /// again on plan-mode resync); its inputs (session / source / origin /
    /// channel / incognito / project / attach rows) only change at a turn
    /// boundary, so the resolution (a couple of SQLite round-trips) is memoized
    /// for the turn. Cleared in `reset_chat_flags` (turn start) and
    /// `set_session_id` (cached-agent rebind). `Arc` so consumers share one
    /// allocation; `KbAccess` is `Copy`. **Schema/prompt/recall only — never
    /// gate tool EXECUTION off this** (the execution boundary `note.rs::access_map`
    /// stays live so a mid-turn revoke still blocks real reads/writes).
    pub(crate) kb_access_cache: std::sync::Mutex<
        Option<std::sync::Arc<std::collections::HashMap<String, crate::knowledge::KbAccess>>>,
    >,
    /// Per-turn memo of the system-prompt inputs that hit SessionDB / spawn a
    /// `git` subprocess inside the otherwise-synchronous prompt builders (the
    /// full config/goal/memory base prompt and the LSP diagnostics suffix).
    /// Refreshed on the blocking pool by `refresh_turn_prompt_cache` at turn
    /// start and on plan-mode resync, then read synchronously by
    /// `build_full_system_prompt` / `build_merged_system_prompt`. A miss
    /// (different model/provider, or a caller outside the streaming turn)
    /// falls back to the original synchronous compute. Cleared in
    /// `reset_chat_flags`, `set_agent_id`, `set_session_id`, `set_session_db`.
    pub(crate) turn_prompt_cache: std::sync::Mutex<Option<TurnPromptCache>>,
    /// Optional `ProviderConfig` reference, injected via
    /// [`AssistantAgent::with_failover_context`]. When present **and**
    /// `session_id` is set, side_query / DedicatedModelProvider routes
    /// through `failover::execute_with_failover` for profile rotation +
    /// retry. When `None`, those paths fall back to direct one-shot calls
    /// (legacy behavior, used by `new_anthropic` / `new_openai` test paths).
    pub(crate) provider_config: Option<std::sync::Arc<crate::provider::ProviderConfig>>,
}

/// See [`AssistantAgent::turn_prompt_cache`]: blocking prompt inputs
/// precomputed off-worker once per turn, keyed by the turn's model/provider
/// so a mismatched reader falls back to live compute.
#[derive(Debug)]
pub(crate) struct TurnPromptCache {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) base_prompt: std::sync::Arc<String>,
    pub(crate) lsp_suffix: Option<String>,
}

/// Cached parameters from the last main chat request.
/// Used by `side_query()` to construct cache-friendly API requests that share the
/// same prompt prefix as the main conversation, enabling prompt cache hits.
#[derive(Debug)]
pub(super) struct CacheSafeParams {
    pub system_prompt: String,
    pub tool_schemas: Vec<serde_json::Value>,
    pub conversation_history: Vec<serde_json::Value>,
    pub provider_format: ProviderFormat,
}

/// Provider format tag for CacheSafeParams, derived from LlmProvider variant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ProviderFormat {
    Anthropic,
    OpenAIChat,
    OpenAIResponses,
    Codex,
}

impl From<&LlmProvider> for ProviderFormat {
    fn from(provider: &LlmProvider) -> Self {
        match provider {
            LlmProvider::Anthropic { .. } => ProviderFormat::Anthropic,
            LlmProvider::OpenAIChat { .. } => ProviderFormat::OpenAIChat,
            LlmProvider::OpenAIResponses { .. } => ProviderFormat::OpenAIResponses,
            LlmProvider::Codex { .. } => ProviderFormat::Codex,
        }
    }
}

impl ProviderFormat {
    /// Human-readable label used in `build_full_system_prompt(model, provider_label)`,
    /// log lines, and error messages. Stable string — providers / models
    /// reference this name in prompts.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAIChat => "OpenAIChat",
            Self::OpenAIResponses => "OpenAIResponses",
            Self::Codex => "Codex",
        }
    }
}

/// Result of a side query call.
#[derive(Debug)]
pub struct SideQueryResult {
    pub text: String,
    pub usage: ChatUsage,
}

/// Stateful filter that strips `<think>...</think>` tags from streaming content.
/// Content inside tags is redirected to thinking output; content outside goes to text output.
pub(super) struct ThinkTagFilter {
    in_thinking: bool,
    /// Buffer for potential partial tag at the end of a chunk (e.g. "<", "<th", "</thi")
    tag_buffer: String,
}

impl ThinkTagFilter {
    pub(super) fn new() -> Self {
        Self {
            in_thinking: false,
            tag_buffer: String::new(),
        }
    }

    /// Process a chunk of content text. Returns (text_outside_tags, thinking_inside_tags).
    pub(super) fn process(&mut self, input: &str) -> (String, String) {
        let mut text_out = String::new();
        let mut think_out = String::new();

        // Prepend any buffered partial tag
        let full_input = if self.tag_buffer.is_empty() {
            input.to_string()
        } else {
            let mut s = std::mem::take(&mut self.tag_buffer);
            s.push_str(input);
            s
        };

        let mut chars = full_input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '<' {
                // Collect potential tag
                let mut tag = String::from('<');
                while let Some(&next) = chars.peek() {
                    tag.push(next);
                    chars.next();
                    if next == '>' {
                        break;
                    }
                }

                if !tag.ends_with('>') {
                    // Incomplete tag at end of chunk — buffer it
                    self.tag_buffer = tag;
                    continue;
                }

                let tag_lower = tag.to_lowercase();
                let tag_trimmed =
                    tag_lower.trim_matches(|c: char| c == '<' || c == '>' || c.is_whitespace());
                if tag_trimmed == "think" || tag_trimmed == "thinking" {
                    self.in_thinking = true;
                } else if tag_trimmed == "/think" || tag_trimmed == "/thinking" {
                    self.in_thinking = false;
                } else {
                    // Not a think tag — emit as content
                    if self.in_thinking {
                        think_out.push_str(&tag);
                    } else {
                        text_out.push_str(&tag);
                    }
                }
            } else if self.in_thinking {
                think_out.push(ch);
            } else {
                text_out.push(ch);
            }
        }

        (text_out, think_out)
    }
}

/// Token usage for one chat turn, aggregated across every round of the
/// tool loop. Counts are cumulative; the `last_*` fields track the most
/// recent API round for status UIs where cumulative sums are misleading.
#[derive(Debug, Clone, Default)]
pub struct ChatUsage {
    /// Provider-reported input tokens. For Anthropic this excludes cache
    /// creation/read tokens; for OpenAI-style providers it is already the
    /// complete input count. Keep this raw field for billing compatibility.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// Normalized number of input tokens that occupied the model context.
    /// Unlike `input_tokens`, this has identical semantics across providers.
    pub context_input_tokens: u64,
    /// Context input that was not served by a cache read. Cache writes are
    /// intentionally included: they still require fresh prompt processing.
    pub fresh_input_tokens: u64,
    pub last_input_tokens: u64,
    pub last_context_input_tokens: u64,
    pub last_fresh_input_tokens: u64,
    pub last_cache_creation_input_tokens: u64,
    pub last_cache_read_input_tokens: u64,
}

impl ChatUsage {
    /// Normalize a single Anthropic round. Anthropic reports uncached input,
    /// cache creation, and cache reads as disjoint counters.
    pub fn normalize_anthropic_round(&mut self) {
        self.context_input_tokens = self
            .input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens);
        self.fresh_input_tokens = self
            .context_input_tokens
            .saturating_sub(self.cache_read_input_tokens);
    }

    /// Normalize a single OpenAI-compatible round. OpenAI input tokens already
    /// include cached tokens; cached tokens are a subset of the total.
    pub fn normalize_openai_round(&mut self) {
        self.context_input_tokens = self.input_tokens;
        self.fresh_input_tokens = self
            .context_input_tokens
            .saturating_sub(self.cache_read_input_tokens);
    }

    /// Fold one round's usage into the running turn total. Cumulative
    /// fields accumulate; `last_*` fields are overwritten so callers can
    /// render the most recent round without summing over a tool loop.
    pub fn accumulate_round(&mut self, round: &ChatUsage) {
        self.input_tokens += round.input_tokens;
        self.output_tokens += round.output_tokens;
        self.cache_creation_input_tokens += round.cache_creation_input_tokens;
        self.cache_read_input_tokens += round.cache_read_input_tokens;
        let round_context = if round.context_input_tokens > 0 {
            round.context_input_tokens
        } else {
            round.input_tokens
        };
        let round_fresh = if round.fresh_input_tokens > 0 {
            round.fresh_input_tokens
        } else {
            round_context.saturating_sub(round.cache_read_input_tokens)
        };
        self.context_input_tokens += round_context;
        self.fresh_input_tokens += round_fresh;
        // Backward-compatible event/DB field: `last_input_tokens` has always
        // powered the context gauge, so make it the normalized context count.
        self.last_input_tokens = round_context;
        self.last_context_input_tokens = round_context;
        self.last_fresh_input_tokens = round_fresh;
        self.last_cache_creation_input_tokens = round.cache_creation_input_tokens;
        self.last_cache_read_input_tokens = round.cache_read_input_tokens;
    }
}

#[cfg(test)]
mod chat_usage_tests {
    use super::ChatUsage;

    #[test]
    fn anthropic_round_counts_disjoint_cache_counters_in_context() {
        let mut usage = ChatUsage {
            input_tokens: 10,
            cache_creation_input_tokens: 5,
            cache_read_input_tokens: 20,
            ..Default::default()
        };

        usage.normalize_anthropic_round();

        assert_eq!(usage.context_input_tokens, 35);
        assert_eq!(usage.fresh_input_tokens, 15);
    }

    #[test]
    fn openai_round_treats_cached_tokens_as_subset_of_total_input() {
        let mut usage = ChatUsage {
            input_tokens: 100,
            cache_creation_input_tokens: 5,
            cache_read_input_tokens: 80,
            ..Default::default()
        };

        usage.normalize_openai_round();

        assert_eq!(usage.context_input_tokens, 100);
        assert_eq!(usage.fresh_input_tokens, 20);
    }

    #[test]
    fn accumulate_round_keeps_cache_totals_and_last_round_cache() {
        let mut usage = ChatUsage::default();
        usage.accumulate_round(&ChatUsage {
            input_tokens: 10,
            output_tokens: 1,
            cache_creation_input_tokens: 5,
            cache_read_input_tokens: 20,
            ..Default::default()
        });
        usage.accumulate_round(&ChatUsage {
            input_tokens: 30,
            output_tokens: 2,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 7,
            ..Default::default()
        });

        assert_eq!(usage.input_tokens, 40);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.cache_creation_input_tokens, 5);
        assert_eq!(usage.cache_read_input_tokens, 27);
        assert_eq!(usage.last_input_tokens, 30);
        assert_eq!(usage.last_cache_creation_input_tokens, 0);
        assert_eq!(usage.last_cache_read_input_tokens, 7);
    }
}

// ── Codex model definitions ───────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct CodexModel {
    pub id: String,
    pub name: String,
}
