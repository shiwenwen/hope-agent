#![cfg_attr(test, allow(clippy::needless_return))]

//! Kernel contract and feature port for Dreaming narrative execution.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::OnceLock;

#[cfg(not(test))]
use anyhow::anyhow;
use anyhow::Result;

use super::config::DreamingConfig;
use super::types::PromotionRecord;
use crate::memory::MemoryEntry;
use crate::provider::ActiveModel;

pub struct NarrativeOutput {
    pub promotions: Vec<PromotionRecord>,
    pub promotions_nominated: usize,
    pub diary_markdown: String,
}

pub type NarrativeFuture<'a> = Pin<Box<dyn Future<Output = Result<NarrativeOutput>> + Send + 'a>>;

#[derive(Clone, Copy)]
pub struct DreamingNarrativeRuntime {
    pub build_prompt: fn(&[MemoryEntry], &DreamingConfig) -> String,
    pub run_side_query:
        for<'a> fn(Vec<ActiveModel>, &'a [MemoryEntry], &'a DreamingConfig) -> NarrativeFuture<'a>,
    pub render_diary_markdown: fn(&NarrativeOutput) -> String,
    pub write_diary: fn(&str) -> Result<PathBuf>,
}

static RUNTIME: OnceLock<DreamingNarrativeRuntime> = OnceLock::new();

pub fn register_dreaming_narrative_runtime(
    runtime: DreamingNarrativeRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("dreaming narrative runtime"))
}

#[cfg(test)]
#[path = "../../../../ha-memory/src/dreaming_narrative.rs"]
#[allow(dead_code)]
mod test_narrative;

pub fn build_prompt(candidates: &[MemoryEntry], cfg: &DreamingConfig) -> String {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.build_prompt)(candidates, cfg);
    }
    #[cfg(test)]
    {
        return test_narrative::build_prompt(candidates, cfg);
    }
    #[cfg(not(test))]
    String::new()
}

pub async fn run_side_query(
    chain: Vec<ActiveModel>,
    candidates: &[MemoryEntry],
    cfg: &DreamingConfig,
) -> Result<NarrativeOutput> {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.run_side_query)(chain, candidates, cfg).await;
    }
    #[cfg(test)]
    {
        return test_narrative::run_side_query(chain, candidates, cfg).await;
    }
    #[cfg(not(test))]
    Err(anyhow!("Dreaming narrative runtime is not wired"))
}

pub fn render_diary_markdown(output: &NarrativeOutput) -> String {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.render_diary_markdown)(output);
    }
    #[cfg(test)]
    {
        return test_narrative::render_diary_markdown(output);
    }
    #[cfg(not(test))]
    String::new()
}

pub fn write_diary(markdown: &str) -> Result<PathBuf> {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.write_diary)(markdown);
    }
    #[cfg(test)]
    {
        return test_narrative::write_diary(markdown);
    }
    #[cfg(not(test))]
    Err(anyhow!("Dreaming narrative runtime is not wired"))
}
