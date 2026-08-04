//! Recall summarization config（`AppConfig.recall_summary`）。
//!
//! 执行逻辑（`maybe_summarize_recall` / side_query 调用）留在 ha-core
//! `memory/recall_summary.rs`；此处只有 wire 类型。

use serde::{Deserialize, Serialize};

use crate::util::default_true;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallSummaryConfig {
    /// Master switch. Default: false (opt-in, costs one side_query per call).
    #[serde(default)]
    pub enabled: bool,
    /// Minimum hits before the summarizer fires. Below this the caller gets
    /// raw snippets unchanged. Default 3.
    #[serde(default = "default_min_hits")]
    pub min_hits: usize,
    /// Upper bound (chars) on the raw context fed into the summarizer.
    /// Default 20000.
    #[serde(default = "default_context_budget")]
    pub context_char_budget: usize,
    /// Hard timeout on the side_query roundtrip. Default 30s.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Upper bound on summary output tokens. Default 1024.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Whether to also run the summarizer when the caller requested history
    /// (hits count combines memories + messages). Default true — turn off if
    /// you only want persistent memories summarized.
    #[serde(default = "default_true")]
    pub include_history: bool,
    /// Model chain override for the summarization call. `None` = fall
    /// through to `function_models.automation` → chat default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,
}

impl Default for RecallSummaryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_hits: default_min_hits(),
            context_char_budget: default_context_budget(),
            timeout_secs: default_timeout_secs(),
            max_tokens: default_max_tokens(),
            include_history: true,
            model_override: None,
        }
    }
}

fn default_min_hits() -> usize {
    3
}
fn default_context_budget() -> usize {
    20_000
}
fn default_timeout_secs() -> u64 {
    30
}
fn default_max_tokens() -> u32 {
    1024
}
