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
mod review;
mod store;
mod types;
mod write;

pub use backfill::{
    apply_backfill, plan_backfill, BackfillApplyResult, BackfillCandidate, BackfillPlan,
    BackfillSummary,
};
pub use review::{forget_claim, update_claim, ClaimActionOutcome, ClaimUpdate};
pub use store::{
    claim_graph, claim_review_reason_snapshot, delete_claims_for_scope, expire_claim, get_claim,
    init_claim_store, link_claim_memory, list_active_claims_for_resolve,
    list_claim_conflict_details, list_claim_conflict_summaries, list_claim_conflicts,
    list_claim_evidence_summaries, list_claim_review_summaries, list_claims, list_claims_page,
    list_pinned_claims, mark_claim_needs_review, merge_claims, parse_claim_scope,
    restore_claim_detail, search_claims, write_claim_candidate, write_claim_candidate_with_status,
    ClaimListFilter, ClaimRestoreImportOutcome, ClaimReviewReasonSnapshot, ClaimWriteOutcome,
};
pub use types::{
    ClaimCandidate, ClaimConflictSummary, ClaimDetail, ClaimEvidenceSummary, ClaimGraphEdge,
    ClaimGraphNode, ClaimGraphProjection, ClaimLink, ClaimListPage, ClaimRecord,
    ClaimReviewSummary, ClaimSchemaMetadata, ClaimScopeHint, ClaimTemporal, EvidenceRecord,
    ResolveClaim,
};
pub use write::{
    confidence_baseline, effective_status, is_injectable_status, normalize_evidence_class,
    normalize_object, EVIDENCE_CLASSES,
};

pub const CLAIM_TYPES: [&str; 6] = [
    "user_profile",
    "preference",
    "project_fact",
    "standing_rule",
    "reference",
    "task_pattern",
];
pub const PROFILE_CLAIM_TYPES: [&str; 2] = ["user_profile", "preference"];
pub const PROJECT_CLAIM_TYPE: &str = "project_fact";
pub const EVIDENCE_SOURCE_TYPES: [&str; 7] = [
    "session_message",
    "memory",
    "file",
    "tool_result",
    "url",
    "recap_facet",
    "manual",
];
pub const CONFIDENCE_SOURCES: [&str; 3] = ["derived", "llm_adjusted", "user_confirmed"];
pub const CLAIM_STATUSES: [&str; 5] = [
    "active",
    "superseded",
    "expired",
    "archived",
    "needs_review",
];

pub fn claim_schema_metadata() -> ClaimSchemaMetadata {
    ClaimSchemaMetadata {
        claim_types: CLAIM_TYPES.iter().map(|s| (*s).to_string()).collect(),
        profile_claim_types: PROFILE_CLAIM_TYPES
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        project_claim_type: PROJECT_CLAIM_TYPE.to_string(),
        evidence_classes: EVIDENCE_CLASSES.iter().map(|s| (*s).to_string()).collect(),
        evidence_source_types: EVIDENCE_SOURCE_TYPES
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        confidence_sources: CONFIDENCE_SOURCES
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        statuses: CLAIM_STATUSES.iter().map(|s| (*s).to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_schema_metadata_is_self_consistent() {
        let schema = claim_schema_metadata();
        assert!(schema.claim_types.contains(&PROJECT_CLAIM_TYPE.to_string()));
        for claim_type in PROFILE_CLAIM_TYPES {
            assert!(schema.claim_types.contains(&claim_type.to_string()));
            assert!(schema.profile_claim_types.contains(&claim_type.to_string()));
        }
        for evidence_class in EVIDENCE_CLASSES {
            assert!(schema
                .evidence_classes
                .contains(&evidence_class.to_string()));
        }
        for source_type in EVIDENCE_SOURCE_TYPES {
            assert!(schema
                .evidence_source_types
                .contains(&source_type.to_string()));
        }
        for source in CONFIDENCE_SOURCES {
            assert!(schema.confidence_sources.contains(&source.to_string()));
        }
        for status in CLAIM_STATUSES {
            assert!(schema.statuses.contains(&status.to_string()));
        }
    }
}
