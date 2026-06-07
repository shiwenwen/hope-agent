//! Structured claim layer (next-gen Dreaming).
//!
//! Claims are the dual-track successor to flat `memories`: each is a
//! scoped, typed assertion (`subject predicate object`) with confidence,
//! salience, freshness policy, per-claim evidence, and links back to the
//! legacy `memories` rows it manages (design §2 / §3). They live in the same
//! `memory.db` (tables created in
//! [`crate::memory::sqlite::SqliteMemoryBackend::open`]).
//!
//! Surface: schema + read API (`claim_list` / `claim_get`), claim extraction
//! dual-write + rule-only canonicalize ([`write`]), the prompt-injection
//! hidden-set, and existing-memory backfill ([`backfill`]). Deep consolidation
//! (merge / supersede / expire) and Memory Profile land in later PRs.

mod backfill;
mod store;
mod types;
mod write;

pub use backfill::{
    apply_backfill, plan_backfill, BackfillApplyResult, BackfillCandidate, BackfillPlan,
    BackfillSummary,
};
pub use store::{
    expire_claim, get_claim, init_claim_store, link_claim_memory, list_active_claims_for_resolve,
    list_claims, mark_claim_needs_review, merge_claims, parse_claim_scope, search_claims,
    write_claim_candidate, ClaimListFilter, ClaimWriteOutcome,
};
pub use types::{
    ClaimCandidate, ClaimDetail, ClaimLink, ClaimRecord, ClaimScopeHint, ClaimTemporal,
    EvidenceRecord, ResolveClaim,
};
pub use write::{
    confidence_baseline, effective_status, is_injectable_status, normalize_evidence_class,
    normalize_object, EVIDENCE_CLASSES,
};
