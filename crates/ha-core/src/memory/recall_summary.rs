//! Kernel contract for optional recall summarization.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::Result;

use crate::truncate_utf8;

pub use ha_config_schema::memory::recall_summary::RecallSummaryConfig;

pub type RecallSummaryFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

#[derive(Clone, Copy)]
pub struct RecallSummaryRuntime {
    pub summarize: for<'a> fn(&'a str, &'a str, &'a RecallSummaryConfig) -> RecallSummaryFuture<'a>,
}

static RUNTIME: OnceLock<RecallSummaryRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_recall_summary_runtime(
    runtime: RecallSummaryRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("recall summary runtime"))
}

/// Apply the kernel-owned enablement/budget gates, then delegate the actual
/// one-shot model execution to `ha-memory`. Failures preserve the historical
/// fallback to raw recall output.
pub async fn maybe_summarize_recall(
    query: &str,
    hits: usize,
    context: &str,
    cfg: &RecallSummaryConfig,
) -> Option<String> {
    if !cfg.enabled || hits < cfg.min_hits || context.trim().is_empty() {
        return None;
    }
    let Some(runtime) = RUNTIME.get() else {
        if !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
            app_warn!(
                "memory",
                "recall_summary_runtime_unavailable",
                "Recall summary runtime is not wired; returning raw hits"
            );
        }
        return None;
    };
    let truncated = truncate_utf8(context, cfg.context_char_budget);
    match (runtime.summarize)(query, truncated, cfg).await {
        Ok(text) if !text.trim().is_empty() => Some(text),
        Ok(_) => None,
        Err(error) => {
            app_warn!(
                "memory",
                "recall_summary",
                "Summarization failed, returning raw hits: {}",
                error
            );
            None
        }
    }
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
        assert!(maybe_summarize_recall("q", 100, "context", &cfg)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn below_min_hits_is_noop() {
        let cfg = RecallSummaryConfig {
            enabled: true,
            min_hits: 3,
            ..Default::default()
        };
        assert!(maybe_summarize_recall("q", 2, "context", &cfg)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn empty_context_is_noop() {
        let cfg = RecallSummaryConfig {
            enabled: true,
            min_hits: 1,
            ..Default::default()
        };
        assert!(maybe_summarize_recall("q", 5, "   ", &cfg).await.is_none());
    }
}
