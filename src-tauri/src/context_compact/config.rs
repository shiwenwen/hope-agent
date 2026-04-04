// ── Configuration (user-configurable, stored in config.json) ──

use serde::{Deserialize, Serialize};

fn default_soft_trim_ratio() -> f64 {
    0.50
}
fn default_hard_clear_ratio() -> f64 {
    0.70
}
fn default_keep_last_assistants() -> usize {
    4
}
fn default_min_prunable_tool_chars() -> usize {
    20_000
}
fn default_soft_trim_max_chars() -> usize {
    6_000
}
fn default_soft_trim_head_chars() -> usize {
    2_000
}
fn default_soft_trim_tail_chars() -> usize {
    2_000
}
fn default_hard_clear_placeholder() -> String {
    "[Old tool result content cleared]".into()
}
fn default_summarization_threshold() -> f64 {
    0.85
}
fn default_preserve_recent_turns() -> usize {
    4
}
fn default_identifier_policy() -> String {
    "strict".into()
}
fn default_summarization_timeout() -> u64 {
    60
}
fn default_summary_max_tokens() -> u32 {
    4096
}
fn default_max_history_share() -> f64 {
    0.5
}
fn default_recovery_max_files() -> usize {
    5
}
fn default_recovery_max_file_bytes() -> usize {
    16_384
}

/// Context compaction configuration, stored in config.json `compact` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactConfig {
    // ── Global ──
    /// Enable context compaction (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,

    // ── Tier 0: Microcompaction ──
    /// Enable microcompaction of ephemeral tool results (default: true).
    /// Clears old results from tools like ls/grep/find that become stale quickly.
    #[serde(default = "crate::default_true")]
    pub microcompact_enabled: bool,
    /// Tool names eligible for Tier 0 microcompaction.
    /// Results from these tools are cleared when older than `keep_last_assistants` boundary.
    #[serde(default = "default_microcompact_tools")]
    pub microcompact_tools: Vec<String>,

    // ── Tier 2: Context Pruning ──
    /// Soft trim trigger ratio (default: 0.50)
    #[serde(default = "default_soft_trim_ratio")]
    pub soft_trim_ratio: f64,
    /// Hard clear trigger ratio (default: 0.70)
    #[serde(default = "default_hard_clear_ratio")]
    pub hard_clear_ratio: f64,
    /// Protect last N assistant messages from pruning (default: 4)
    #[serde(default = "default_keep_last_assistants")]
    pub keep_last_assistants: usize,
    /// Skip hard clear if total prunable chars below this (default: 20_000)
    #[serde(default = "default_min_prunable_tool_chars")]
    pub min_prunable_tool_chars: usize,
    /// Only soft-trim tool results larger than this (default: 6_000)
    #[serde(default = "default_soft_trim_max_chars")]
    pub soft_trim_max_chars: usize,
    /// Head chars to keep during soft trim (default: 2_000)
    #[serde(default = "default_soft_trim_head_chars")]
    pub soft_trim_head_chars: usize,
    /// Tail chars to keep during soft trim (default: 2_000)
    #[serde(default = "default_soft_trim_tail_chars")]
    pub soft_trim_tail_chars: usize,
    /// Enable hard clear phase (default: true)
    #[serde(default = "crate::default_true")]
    pub hard_clear_enabled: bool,
    /// Placeholder text for hard-cleared tool results
    #[serde(default = "default_hard_clear_placeholder")]
    pub hard_clear_placeholder: String,
    /// Tool names exempt from pruning
    #[serde(default = "default_tools_deny_prune")]
    pub tools_deny_prune: Vec<String>,

    // ── Tier 3: LLM Summarization ──
    /// Summarization trigger ratio (default: 0.85)
    #[serde(default = "default_summarization_threshold")]
    pub summarization_threshold: f64,
    /// Preserve last N user turns during summarization (default: 4, max: 12)
    #[serde(default = "default_preserve_recent_turns")]
    pub preserve_recent_turns: usize,
    /// Identifier preservation policy: "strict" | "off" | "custom" (default: "strict")
    #[serde(default = "default_identifier_policy")]
    pub identifier_policy: String,
    /// Custom identifier instructions (when policy is "custom")
    #[serde(default)]
    pub identifier_instructions: Option<String>,
    /// Custom summarization instructions (appended to default prompt)
    #[serde(default)]
    pub custom_instructions: Option<String>,
    /// Summarization timeout in seconds (default: 60)
    #[serde(default = "default_summarization_timeout")]
    pub summarization_timeout_secs: u64,
    /// Max output tokens for summarization call (default: 4096)
    #[serde(default = "default_summary_max_tokens")]
    pub summary_max_tokens: u32,
    /// Max share of context window for history during pruning (default: 0.5)
    #[serde(default = "default_max_history_share")]
    pub max_history_share: f64,

    // ── Post-Compaction Recovery ──
    /// Enable post-compaction file recovery after Tier 3 summarization (default: true).
    /// Re-reads recently written/edited files from disk and injects their current
    /// contents so the model doesn't need an extra read tool call.
    #[serde(default = "crate::default_true")]
    pub recovery_enabled: bool,
    /// Max files to recover after compaction (default: 5)
    #[serde(default = "default_recovery_max_files")]
    pub recovery_max_files: usize,
    /// Max bytes per recovered file (default: 16384 = 16KB)
    #[serde(default = "default_recovery_max_file_bytes")]
    pub recovery_max_file_bytes: usize,
}

fn default_microcompact_tools() -> Vec<String> {
    use crate::tools::{
        TOOL_AGENTS_LIST, TOOL_FIND, TOOL_GREP, TOOL_LS, TOOL_PROCESS, TOOL_SESSIONS_LIST,
    };
    vec![
        TOOL_LS.into(),
        TOOL_GREP.into(),
        TOOL_FIND.into(),
        TOOL_PROCESS.into(),
        TOOL_SESSIONS_LIST.into(),
        TOOL_AGENTS_LIST.into(),
    ]
}

fn default_tools_deny_prune() -> Vec<String> {
    use crate::tools::{
        TOOL_DELETE_MEMORY, TOOL_MEMORY_GET, TOOL_RECALL_MEMORY, TOOL_SAVE_MEMORY,
        TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY, TOOL_WEB_FETCH, TOOL_WEB_SEARCH,
    };
    vec![
        TOOL_WEB_SEARCH.into(),
        TOOL_WEB_FETCH.into(),
        TOOL_SAVE_MEMORY.into(),
        TOOL_RECALL_MEMORY.into(),
        TOOL_UPDATE_MEMORY.into(),
        TOOL_DELETE_MEMORY.into(),
        TOOL_MEMORY_GET.into(),
        TOOL_UPDATE_CORE_MEMORY.into(),
    ]
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            enabled: crate::default_true(),
            microcompact_enabled: crate::default_true(),
            microcompact_tools: default_microcompact_tools(),
            soft_trim_ratio: default_soft_trim_ratio(),
            hard_clear_ratio: default_hard_clear_ratio(),
            keep_last_assistants: default_keep_last_assistants(),
            min_prunable_tool_chars: default_min_prunable_tool_chars(),
            soft_trim_max_chars: default_soft_trim_max_chars(),
            soft_trim_head_chars: default_soft_trim_head_chars(),
            soft_trim_tail_chars: default_soft_trim_tail_chars(),
            hard_clear_enabled: crate::default_true(),
            hard_clear_placeholder: default_hard_clear_placeholder(),
            tools_deny_prune: default_tools_deny_prune(),
            summarization_threshold: default_summarization_threshold(),
            preserve_recent_turns: default_preserve_recent_turns(),
            identifier_policy: default_identifier_policy(),
            identifier_instructions: None,
            custom_instructions: None,
            summarization_timeout_secs: default_summarization_timeout(),
            summary_max_tokens: default_summary_max_tokens(),
            max_history_share: default_max_history_share(),
            recovery_enabled: crate::default_true(),
            recovery_max_files: default_recovery_max_files(),
            recovery_max_file_bytes: default_recovery_max_file_bytes(),
        }
    }
}
