//! Kernel policy/types and feature port for dynamic Memory recall.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::agent::active_memory::{ActiveMemoryCandidateRef, ActiveMemoryRecall};
use crate::agent::retrieval_planner::{classify_intent, RetrievalIntent};

use super::{
    claims::ClaimRecord, MemoryEntry, MemoryRecallRuntimeConfig, MemoryRetrievalEvidence,
    MemoryScope,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecallSkipReason {
    EmptyQuery,
    Incognito,
    MemoryOff,
    RecallOff,
    NoCandidates,
    BudgetEmpty,
    RuntimeUnavailable,
}

impl RecallSkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmptyQuery => "empty_query",
            Self::Incognito => "incognito",
            Self::MemoryOff => "memory_off",
            Self::RecallOff => "recall_off",
            Self::NoCandidates => "no_candidates",
            Self::BudgetEmpty => "budget_empty",
            Self::RuntimeUnavailable => "runtime_unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallGate {
    Search { intent: RetrievalIntent },
    Skip(RecallSkipReason),
}

/// Consent/incognito gate remains kernel-owned; the optional feature runtime
/// receives only already-authorized candidates.
pub fn recall_gate(
    query: &str,
    incognito: bool,
    memory_enabled: bool,
    recall_enabled: bool,
) -> RecallGate {
    if incognito {
        return RecallGate::Skip(RecallSkipReason::Incognito);
    }
    if !memory_enabled {
        return RecallGate::Skip(RecallSkipReason::MemoryOff);
    }
    if !recall_enabled {
        return RecallGate::Skip(RecallSkipReason::RecallOff);
    }
    let normalized = query
        .trim()
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase();
    if normalized.is_empty() {
        return RecallGate::Skip(RecallSkipReason::EmptyQuery);
    }
    RecallGate::Search {
        intent: classify_intent(query),
    }
}

#[derive(Debug, Clone)]
pub struct ProfileRecallCandidate {
    pub id: String,
    pub scope: MemoryScope,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct AuxiliaryRecallCandidate {
    pub kind: String,
    pub id: String,
    pub source_type: String,
    pub scope: MemoryScope,
    pub content: String,
    pub retrieval_score: Option<f32>,
    pub confidence: Option<f32>,
    pub salience: Option<f32>,
    pub intent_score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDeepRecall {
    pub selected_indices: Vec<usize>,
    pub summary: Option<String>,
}

#[derive(Clone, Copy)]
pub struct MemoryRetrievalRuntime {
    pub plan_fast: fn(
        &str,
        Vec<MemoryEntry>,
        Vec<ClaimRecord>,
        Vec<ProfileRecallCandidate>,
        Vec<AuxiliaryRecallCandidate>,
        &MemoryRecallRuntimeConfig,
    ) -> Result<ActiveMemoryRecall, RecallSkipReason>,
    pub evidence_relevant: fn(&str, Option<&MemoryRetrievalEvidence>) -> bool,
    pub build_deep: fn(&str, &[ActiveMemoryCandidateRef], usize, usize) -> String,
    pub parse_deep: fn(&str, usize, usize, usize) -> Option<ParsedDeepRecall>,
    pub apply_deep: fn(ActiveMemoryRecall, ParsedDeepRecall, u32) -> Option<ActiveMemoryRecall>,
}

static RUNTIME: OnceLock<MemoryRetrievalRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_memory_retrieval_runtime(
    runtime: MemoryRetrievalRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("memory retrieval runtime"))
}

fn runtime() -> Option<&'static MemoryRetrievalRuntime> {
    let runtime = RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "retrieval_runtime_unavailable",
            "Memory retrieval runtime is not wired; automatic recall is disabled"
        );
    }
    runtime
}

pub fn plan_fast_recall(
    query: &str,
    memories: Vec<MemoryEntry>,
    claims: Vec<ClaimRecord>,
    profiles: Vec<ProfileRecallCandidate>,
    auxiliary: Vec<AuxiliaryRecallCandidate>,
    config: &MemoryRecallRuntimeConfig,
) -> Result<ActiveMemoryRecall, RecallSkipReason> {
    let runtime = runtime().ok_or(RecallSkipReason::RuntimeUnavailable)?;
    (runtime.plan_fast)(query, memories, claims, profiles, auxiliary, config)
}

pub fn retrieval_evidence_is_relevant(
    query: &str,
    evidence: Option<&MemoryRetrievalEvidence>,
) -> bool {
    runtime().is_some_and(|runtime| (runtime.evidence_relevant)(query, evidence))
}

pub fn build_deep_recall_prompt(
    query: &str,
    candidates: &[ActiveMemoryCandidateRef],
    max_selected: usize,
    max_chars: usize,
) -> String {
    runtime().map_or_else(String::new, |runtime| {
        (runtime.build_deep)(query, candidates, max_selected, max_chars)
    })
}

pub fn parse_deep_recall_response(
    raw: &str,
    candidate_count: usize,
    max_selected: usize,
    max_chars: usize,
) -> Option<ParsedDeepRecall> {
    runtime()
        .and_then(|runtime| (runtime.parse_deep)(raw, candidate_count, max_selected, max_chars))
}

pub fn apply_deep_recall(
    recall: ActiveMemoryRecall,
    parsed: ParsedDeepRecall,
    max_tokens: u32,
) -> Option<ActiveMemoryRecall> {
    runtime().and_then(|runtime| (runtime.apply_deep)(recall, parsed, max_tokens))
}
