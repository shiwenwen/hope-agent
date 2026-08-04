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

use crate::truncate_utf8;

// 类型已下沉 ha-config-schema，此处原地再导出保持
// `crate::memory::recall_summary::RecallSummaryConfig` 路径不变。
pub use ha_config_schema::memory::recall_summary::RecallSummaryConfig;

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
    let chain = crate::automation::effective_chain(&config, cfg.model_override.clone());
    let fut = crate::automation::run(crate::automation::ModelTaskSpec {
        purpose: "recall_summary",
        chain,
        session_key: "automation:recall_summary",
        instruction: &prompt,
        max_tokens: cfg.max_tokens,
    });
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
