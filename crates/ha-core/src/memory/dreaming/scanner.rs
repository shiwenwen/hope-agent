#![cfg_attr(test, allow(clippy::needless_return))]

//! Kernel contract and feature port for Dreaming candidate scans.

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::Result;

use super::types::EvidenceRef;
use crate::memory::MemoryEntry;

#[derive(Clone, Copy)]
pub struct DreamingScannerRuntime {
    pub collect_candidates: fn(u32, usize) -> Result<Vec<MemoryEntry>>,
    pub evidence_for_candidate: fn(&MemoryEntry) -> Vec<EvidenceRef>,
    pub render_candidates_for_prompt: fn(&[MemoryEntry]) -> String,
}

static RUNTIME: OnceLock<DreamingScannerRuntime> = OnceLock::new();
#[cfg(not(test))]
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_dreaming_scanner_runtime(
    runtime: DreamingScannerRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("dreaming scanner runtime"))
}

#[cfg(test)]
#[path = "../../../../ha-memory/src/dreaming_scanner.rs"]
mod test_scanner;

pub fn collect_candidates(scope_days: u32, limit: usize) -> Result<Vec<MemoryEntry>> {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.collect_candidates)(scope_days, limit);
    }
    #[cfg(test)]
    {
        return test_scanner::collect_candidates(scope_days, limit);
    }
    #[cfg(not(test))]
    {
        warn_unavailable();
        Ok(Vec::new())
    }
}

pub fn evidence_for_candidate(entry: &MemoryEntry) -> Vec<EvidenceRef> {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.evidence_for_candidate)(entry);
    }
    #[cfg(test)]
    {
        return test_scanner::evidence_for_candidate(entry);
    }
    #[cfg(not(test))]
    {
        warn_unavailable();
        vec![EvidenceRef::memory(entry.id)]
    }
}

pub fn render_candidates_for_prompt(candidates: &[MemoryEntry]) -> String {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.render_candidates_for_prompt)(candidates);
    }
    #[cfg(test)]
    {
        return test_scanner::render_candidates_for_prompt(candidates);
    }
    #[cfg(not(test))]
    {
        warn_unavailable();
        "(no candidates)".to_string()
    }
}

#[cfg(not(test))]
fn warn_unavailable() {
    if !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "dreaming_scanner_runtime_unavailable",
            "Dreaming scanner runtime is not wired; candidate scan is disabled"
        );
    }
}
