//! Recall summarization layer (Phase B'3).
//!
//! When `recall_memory` / `session_search` return many hits, the raw snippet
//! list is noisy and expensive to reason over. Opt-in behaviour: if
//! `AppConfig.recall_summary.enabled` is true AND we have at least
//! `min_hits` results, collapse them into a single concise paragraph via a
//! bounded `side_query` on a fresh analysis agent.
//!
//! Failures (timeout, no provider, LLM error) degrade to the raw output so
//! the caller never has to handle this layer specially.

use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::truncate_utf8;
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

/// Decide whether to summarize and execute the side_query. When the config
/// is disabled or too few hits, returns `None` and the caller should use the
/// raw output as-is. On LLM error / timeout, also returns `None` (degrade
/// silently).
///
/// `context` is the already-rendered snippet text (the raw tool result). We
/// just ask the model to compress it; we don't re-fetch memories here.
pub async fn maybe_summarize_recall(
    query: &str,
    hits: usize,
    context: &str,
    cfg: &RecallSummaryConfig,
) -> Option<String> {
    if !cfg.enabled || hits < cfg.min_hits || context.trim().is_empty() {
        return None;
    }
    // Bound the context size up front so the side_query prompt stays within
    // the cache-safe prefix size.
    let truncated = truncate_utf8(context, cfg.context_char_budget);
    match run_summary(query, truncated, cfg).await {
        Ok(text) if !text.trim().is_empty() => Some(text),
        Ok(_) => None,
        Err(e) => {
            app_warn!(
                "memory",
                "recall_summary",
                "Summarization failed, returning raw hits: {}",
                e
            );
            None
        }
    }
}

async fn run_summary(query: &str, context: &str, cfg: &RecallSummaryConfig) -> Result<String> {
    let prompt = format!(
        "User's current question: {query}\n\n\
         Past memory/history fragments ({n_chars} chars):\n\n{context}\n\n\
         Integrate into ONE concise paragraph (≤400 chars). Focus on \
         actionable insights, user preferences, key decisions, and unresolved \
         points. Skip low-signal details. No bullets, no headings — just \
         prose. If nothing is relevant to the question, reply exactly with \
         the single word NONE.",
        query = query,
        n_chars = context.len(),
        context = context,
    );
    let config = crate::config::cached_config();
    let (agent, _model_id) = crate::recap::report::build_analysis_agent(&config)?;
    let fut = agent.side_query(&prompt, cfg.max_tokens);
    let result = tokio::time::timeout(Duration::from_secs(cfg.timeout_secs), fut)
        .await
        .map_err(|_| anyhow::anyhow!("recall_summary side_query timed out"))??;
    let text = result.text.trim();
    if text.eq_ignore_ascii_case("NONE") {
        return Ok(String::new());
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_is_noop() {
        let cfg = RecallSummaryConfig {
            enabled: false,
            ..Default::default()
        };
        let result = maybe_summarize_recall("q", 100, "context", &cfg).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn below_min_hits_is_noop() {
        let cfg = RecallSummaryConfig {
            enabled: true,
            min_hits: 3,
            ..Default::default()
        };
        let result = maybe_summarize_recall("q", 2, "context", &cfg).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn empty_context_is_noop() {
        let cfg = RecallSummaryConfig {
            enabled: true,
            min_hits: 1,
            ..Default::default()
        };
        let result = maybe_summarize_recall("q", 5, "   ", &cfg).await;
        assert!(result.is_none());
    }
}
