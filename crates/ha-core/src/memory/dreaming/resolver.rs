#![cfg_attr(test, allow(clippy::needless_return, dead_code))]

//! Deep resolver — temporal expire + duplicate merge + conflict detection over
//! `active` claims (design §4.5). The next-gen difference: not just "remember",
//! but "know when to stop trusting".
//!
//! MVP policy:
//! - **expire** is DETERMINISTIC: any active claim whose `valid_until` has
//!   passed becomes `expired` (no LLM).
//! - **merge / conflict** are LLM-judged per group (same scope + claim_type +
//!   subject + predicate, ≥2 distinct objects). The LLM only classifies the
//!   group relationship; the landing is conservative:
//!   - `duplicates` → fold evidence into one survivor, archive the rest (merge).
//!   - `conflict`   → mark every member `needs_review` (NEVER auto-supersede —
//!     deterministic rules can't tell a real conflict from coexisting facts
//!     like `uses:rust` vs `uses:typescript`; the user decides).
//!   - `independent` → no-op.
//! - Nothing is hard-deleted (design N1); only status changes + an audit row.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::memory::claims::ResolveClaim;

/// Cap on conflict groups analyzed per resolver run. Each group is one LLM
/// side_query, so this bounds per-run LLM calls, cost, and lock-hold time (a
/// huge memory base can't turn one Dashboard click into unbounded calls / a
/// lease-overrunning run). Overflow is left for the next run — expire/merge
/// shrink the active set each pass, so it converges.
const MAX_RESOLVER_GROUPS: usize = 50;

/// Kind of resolver outcome for one claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverDecisionType {
    Expire,
    Merge,
    NeedsReview,
}

impl ResolverDecisionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolverDecisionType::Expire => "expire",
            ResolverDecisionType::Merge => "merge",
            ResolverDecisionType::NeedsReview => "needs_review",
        }
    }
}

/// One planned resolver mutation + its audit rationale.
#[derive(Debug, Clone)]
pub struct ResolverDecision {
    pub decision_type: ResolverDecisionType,
    pub claim_id: String,
    pub rationale: String,
    /// For `Merge`: the surviving claim the evidence folds into.
    pub merge_into: Option<String>,
}

/// Deterministic graph-planning projection consumed by the offline evaluator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoResolverPlanning {
    pub llm_group_ids: Vec<Vec<String>>,
    pub graph_noop_group_ids: Vec<Vec<String>>,
    pub truncated: bool,
}

/// Terminal summary of a resolver cycle.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub scanned: usize,
    pub expired: usize,
    pub merged: usize,
    pub needs_review: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ResolverReport {
    #[doc(hidden)]
    pub fn skipped(note: &str, started: Instant) -> Self {
        ResolverReport {
            run_id: None,
            scanned: 0,
            expired: 0,
            merged: 0,
            needs_review: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some(note.to_string()),
        }
    }
}

/// Stable owner-facing reasons why a Deep Resolver run cannot be started.
/// "No work" is not a blocker: a manual click may still return a skipped
/// ResolverReport, but preflight should distinguish safety/config blockers from
/// an empty queue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolverPreflightBlockReason {
    DreamingDisabled,
    LongTermMemoryDisabled,
    ManualDisabled,
    ClaimLoadFailed,
}

impl ResolverPreflightBlockReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DreamingDisabled => "dreaming_disabled",
            Self::LongTermMemoryDisabled => "long_term_memory_disabled",
            Self::ManualDisabled => "manual_disabled",
            Self::ClaimLoadFailed => "claim_load_failed",
        }
    }
}

/// Owner-only preflight for a manual Deep Resolver run. It never calls the LLM
/// and never writes claim state; it only reports how much deterministic expiry
/// and conflict-group work a run would see at this moment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolverPreflightReport {
    pub generated_at: String,
    pub dreaming_enabled: bool,
    pub long_term_memory_enabled: bool,
    pub manual_enabled: bool,
    pub auto_expire_on_light_cycle: bool,
    pub auto_resolve_on_light_cycle: bool,
    pub auto_resolve_max_groups: usize,
    pub auto_resolve_min_confidence: f32,
    pub auto_merge_near_duplicates: bool,
    pub auto_merge_similarity: f32,
    pub auto_supersede: bool,
    pub can_run_manual: bool,
    pub active_claim_count: usize,
    pub expired_candidate_count: usize,
    pub conflict_group_count: usize,
    pub groups_to_analyze: usize,
    pub group_cap: usize,
    pub truncated: bool,
    pub would_call_llm: bool,
    pub would_write_expirations: bool,
    pub blocking_reasons: Vec<ResolverPreflightBlockReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_error: Option<String>,
}

/// Feature-owned Deep Resolver execution port.
pub type ResolverRuntimeFuture = Pin<Box<dyn Future<Output = ResolverReport> + Send + 'static>>;

#[derive(Clone, Copy)]
pub struct DreamingResolverRuntime {
    pub preflight: fn() -> ResolverPreflightReport,
    pub preflight_from_claims: fn(
        &super::config::DreamingConfig,
        bool,
        Result<Vec<ResolveClaim>, String>,
        &str,
    ) -> ResolverPreflightReport,
    pub run_cycle: fn(super::triggers::DreamTrigger) -> ResolverRuntimeFuture,
    pub plan_auto_expiration: fn(&[ResolveClaim], &str) -> Option<Vec<ResolverDecision>>,
    pub plan_auto_groups: fn(&[ResolveClaim], &HashSet<String>, usize) -> AutoResolverPlanning,
}

static RESOLVER_RUNTIME: OnceLock<DreamingResolverRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_dreaming_resolver_runtime(
    runtime: DreamingResolverRuntime,
) -> Result<(), &'static str> {
    RESOLVER_RUNTIME
        .set(runtime)
        .map_err(|_| "Dreaming resolver runtime already registered")
}

fn runtime() -> Option<&'static DreamingResolverRuntime> {
    let runtime = RESOLVER_RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "dreaming_resolver_runtime_unavailable",
            "Deep Resolver runtime is not wired; resolver execution is unavailable"
        );
    }
    runtime
}

pub fn resolver_preflight() -> ResolverPreflightReport {
    if let Some(runtime) = runtime() {
        return (runtime.preflight)();
    }
    let cfg = crate::config::cached_config();
    resolver_preflight_from_claims(
        &cfg.dreaming,
        cfg.memory.effective_enabled(cfg.memory_extract.enabled),
        Err("Deep Resolver runtime is not wired".to_string()),
        &crate::util::now_rfc3339(),
    )
}

pub(crate) fn resolver_preflight_from_claims(
    cfg: &super::config::DreamingConfig,
    long_term_memory_enabled: bool,
    claims_result: Result<Vec<ResolveClaim>, String>,
    now: &str,
) -> ResolverPreflightReport {
    if let Some(runtime) = RESOLVER_RUNTIME.get() {
        return (runtime.preflight_from_claims)(cfg, long_term_memory_enabled, claims_result, now);
    }
    #[cfg(test)]
    {
        return test_resolver::resolver_preflight_from_claims(
            cfg,
            long_term_memory_enabled,
            claims_result,
            now,
        );
    }
    #[cfg(not(test))]
    {
        let load_error = claims_result
            .err()
            .or_else(|| Some("Deep Resolver runtime is not wired".to_string()));
        ResolverPreflightReport {
            generated_at: now.to_string(),
            dreaming_enabled: cfg.enabled,
            long_term_memory_enabled,
            manual_enabled: cfg.manual_enabled,
            auto_expire_on_light_cycle: cfg.deep_resolver.auto_expire_on_light_cycle,
            auto_resolve_on_light_cycle: cfg.deep_resolver.auto_resolve_on_light_cycle,
            auto_resolve_max_groups: cfg.deep_resolver.auto_group_cap(),
            auto_resolve_min_confidence: cfg.deep_resolver.auto_min_confidence(),
            auto_merge_near_duplicates: cfg.deep_resolver.auto_merge_near_duplicates,
            auto_merge_similarity: cfg.deep_resolver.auto_merge_similarity_threshold(),
            auto_supersede: false,
            can_run_manual: false,
            active_claim_count: 0,
            expired_candidate_count: 0,
            conflict_group_count: 0,
            groups_to_analyze: 0,
            group_cap: MAX_RESOLVER_GROUPS,
            truncated: false,
            would_call_llm: false,
            would_write_expirations: false,
            blocking_reasons: vec![ResolverPreflightBlockReason::ClaimLoadFailed],
            load_error,
        }
    }
}

pub async fn run_resolver_cycle(trigger: super::triggers::DreamTrigger) -> ResolverReport {
    let started = Instant::now();
    let Some(runtime) = runtime() else {
        return ResolverReport::skipped("Deep Resolver runtime is not wired", started);
    };
    (runtime.run_cycle)(trigger).await
}

pub(in crate::memory::dreaming) fn plan_auto_expiration_sweep(
    claims: &[ResolveClaim],
    now: &str,
) -> Option<Vec<ResolverDecision>> {
    if let Some(runtime) = RESOLVER_RUNTIME.get() {
        return (runtime.plan_auto_expiration)(claims, now);
    }
    #[cfg(test)]
    {
        return test_resolver::plan_auto_expiration_sweep(claims, now);
    }
    #[cfg(not(test))]
    None
}

pub(in crate::memory::dreaming) fn plan_auto_resolution_groups(
    claims: &[ResolveClaim],
    expiring: &HashSet<String>,
    group_cap: usize,
) -> AutoResolverPlanning {
    if let Some(runtime) = RESOLVER_RUNTIME.get() {
        return (runtime.plan_auto_groups)(claims, expiring, group_cap);
    }
    #[cfg(test)]
    {
        return test_resolver::plan_auto_resolution_groups(claims, expiring, group_cap);
    }
    #[cfg(not(test))]
    AutoResolverPlanning::default()
}

#[cfg(test)]
#[path = "../../../../ha-memory/src/dreaming_resolver.rs"]
mod test_resolver;
