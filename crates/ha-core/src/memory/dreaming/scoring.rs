#![cfg_attr(test, allow(clippy::needless_return))]

//! Kernel contract and feature port for Dreaming nomination scoring.

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use super::types::PromotionRecord;

#[derive(Clone, Copy)]
pub struct DreamingScoringRuntime {
    pub parse_nominations: fn(&str) -> Vec<PromotionRecord>,
    pub filter_and_rank: fn(Vec<PromotionRecord>, f32, usize) -> Vec<PromotionRecord>,
}

static RUNTIME: OnceLock<DreamingScoringRuntime> = OnceLock::new();
#[cfg(not(test))]
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_dreaming_scoring_runtime(
    runtime: DreamingScoringRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("dreaming scoring runtime"))
}

#[cfg(test)]
#[path = "../../../../ha-memory/src/dreaming_scoring.rs"]
mod test_scoring;

pub fn parse_nominations(text: &str) -> Vec<PromotionRecord> {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.parse_nominations)(text);
    }
    #[cfg(test)]
    {
        return test_scoring::parse_nominations(text);
    }
    #[cfg(not(test))]
    {
        warn_unavailable();
        Vec::new()
    }
}

pub fn filter_and_rank(
    records: Vec<PromotionRecord>,
    min_score: f32,
    max_promote: usize,
) -> Vec<PromotionRecord> {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.filter_and_rank)(records, min_score, max_promote);
    }
    #[cfg(test)]
    {
        return test_scoring::filter_and_rank(records, min_score, max_promote);
    }
    #[cfg(not(test))]
    {
        warn_unavailable();
        Vec::new()
    }
}

#[cfg(not(test))]
fn warn_unavailable() {
    if !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "dreaming_scoring_runtime_unavailable",
            "Dreaming scoring runtime is not wired; promotion is disabled"
        );
    }
}
