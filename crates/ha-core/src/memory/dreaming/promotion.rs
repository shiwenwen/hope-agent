//! Kernel contract for Dreaming promotions.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};

use super::types::PromotionRecord;

#[derive(Clone, Copy)]
pub struct DreamingPromotionRuntime {
    pub apply: fn(&[PromotionRecord]) -> Result<Vec<i64>>,
}

static RUNTIME: OnceLock<DreamingPromotionRuntime> = OnceLock::new();

pub fn register_dreaming_promotion_runtime(
    runtime: DreamingPromotionRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("dreaming promotion runtime"))
}

pub fn apply_promotions(records: &[PromotionRecord]) -> Result<Vec<i64>> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| anyhow!("Dreaming promotion runtime is not wired"))?;
    (runtime.apply)(records)
}
