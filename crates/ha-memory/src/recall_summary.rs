//! Optional recall summarization execution machine.

use std::time::Duration;

use anyhow::Result;

use ha_core::memory::recall_summary::{RecallSummaryConfig, RecallSummaryFuture};

async fn summarize(query: &str, context: &str, cfg: &RecallSummaryConfig) -> Result<String> {
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
    let config = ha_core::config::cached_config();
    let chain = ha_core::automation::effective_chain(&config, cfg.model_override.clone());
    let future = ha_core::automation::run(ha_core::automation::ModelTaskSpec {
        purpose: "recall_summary",
        chain,
        session_key: "automation:recall_summary",
        instruction: &prompt,
        max_tokens: cfg.max_tokens,
    });
    let result = tokio::time::timeout(Duration::from_secs(cfg.timeout_secs), future)
        .await
        .map_err(|_| anyhow::anyhow!("recall_summary side_query timed out"))??;
    let text = result.text.trim();
    if text.eq_ignore_ascii_case("NONE") {
        return Ok(String::new());
    }
    Ok(text.to_string())
}

pub fn summarize_boxed<'a>(
    query: &'a str,
    context: &'a str,
    cfg: &'a RecallSummaryConfig,
) -> RecallSummaryFuture<'a> {
    Box::pin(summarize(query, context, cfg))
}
