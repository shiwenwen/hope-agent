//! Shared types for the structured claim layer (next-gen Dreaming).
//!
//! These mirror the `memory_claims` / `memory_evidence` / `memory_claim_links`
//! columns (design §3.2 / §3.3 / §3.3.1), returned by the read API. Free-form
//! JSON columns (`tags_json`, `freshness_policy_json`, `access_scope_json`) are
//! parsed into values for the frontend; everything is camelCase over the wire.

use serde::{Deserialize, Serialize};

/// Owner-facing schema metadata for the current structured memory claim layer.
/// This is read-only UI/configuration metadata, not a security boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimSchemaMetadata {
    pub claim_types: Vec<String>,
    pub profile_claim_types: Vec<String>,
    pub project_claim_type: String,
    pub evidence_classes: Vec<String>,
    pub evidence_source_types: Vec<String>,
    pub confidence_sources: Vec<String>,
    pub statuses: Vec<String>,
}

/// A structured long-term memory claim — one row of `memory_claims`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRecord {
    pub id: String,
    /// "global" | "agent" | "project".
    pub scope_type: String,
    /// Agent / project id for scoped claims; `None` for global.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// user_profile | preference | project_fact | standing_rule | reference |
    /// task_pattern.
    pub claim_type: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub content: String,
    pub tags: Vec<String>,
    pub confidence: f32,
    /// derived | llm_adjusted | user_confirmed.
    pub confidence_source: String,
    pub salience: f32,
    /// Decay parameters; freshness is computed at read time, not stored.
    pub freshness_policy: serde_json::Value,
    /// active | superseded | expired | archived | needs_review.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_claim_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Search-time evidence only; absent from ordinary owner list/get rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_evidence: Option<super::super::MemoryRetrievalEvidence>,
}

/// Page response for owner-plane structured memory listing. `total` is the
/// exact count for the same filters; `total_truncated` is reserved for future
/// bounded scans and is currently `false` for SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimListPage {
    pub items: Vec<ClaimRecord>,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
}

/// One row of `memory_evidence` — the provenance of a claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRecord {
    pub id: String,
    pub claim_id: String,
    /// session_message | memory | file | tool_result | url | recap_facet |
    /// manual.
    pub source_type: String,
    /// manual_correction | user_confirmed | explicit_user_statement |
    /// project_artifact_fact | assistant_inferred | behavioral_pattern.
    pub evidence_class: String,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Short, redacted excerpt — may be absent (anchor-only evidence). The
    /// read API does not re-redact; redaction happens at write time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// redacted | raw_allowed | anchor_only.
    pub redaction_status: String,
    pub access_scope: serde_json::Value,
    pub weight: f32,
    pub created_at: String,
}

/// One row of `memory_claim_links` — the sync relationship between a claim and
/// a legacy `memories` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimLink {
    pub claim_id: String,
    pub memory_id: i64,
    /// managed | user_pinned | detached.
    pub sync_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_claim_status: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A claim plus its evidence and legacy-memory links — returned by
/// `claim_get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimDetail {
    pub claim: ClaimRecord,
    pub evidence: Vec<EvidenceRecord>,
    pub links: Vec<ClaimLink>,
}

/// Owner-facing conflict counts for Review Inbox list grouping. This is a
/// read-only UI summary, not a resolver decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimConflictSummary {
    pub claim_id: String,
    pub conflict_count: usize,
    pub active_count: usize,
    pub needs_review_count: usize,
    pub examples: Vec<ClaimConflictExample>,
}

/// Bounded, list-safe preview of a conflicting claim. Full evidence stays
/// behind `claim_conflict_details`; this exists only to help the Review Inbox
/// list explain why an item was grouped as a conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimConflictExample {
    pub claim_id: String,
    pub status: String,
    pub object: String,
    pub content: String,
    pub confidence: f32,
    pub salience: f32,
}

/// Owner-facing evidence trust counts for claim list rows. This intentionally
/// omits quotes, file paths and URLs; full provenance stays behind `claim_get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimEvidenceSummary {
    pub claim_id: String,
    pub evidence_count: usize,
    pub confirmed_count: usize,
    pub source_backed_count: usize,
    pub inferred_count: usize,
    pub trust: String,
}

/// Owner-facing Review Inbox risk projection for claim list rows. This is a
/// deterministic, list-safe explanation of why a `needs_review` claim should
/// be reviewed; it contains only reason keys and counts, never evidence
/// payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimReviewSummary {
    pub claim_id: String,
    pub primary: String,
    pub risks: Vec<String>,
    pub conflict_count: usize,
}

/// Read-only entity/relation projection built from existing claims. This is an
/// owner-plane diagnostic/context view, not a second source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimGraphProjection {
    pub center_claim_id: String,
    pub nodes: Vec<ClaimGraphNode>,
    pub edges: Vec<ClaimGraphEdge>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimGraphNode {
    pub id: String,
    pub label: String,
    pub entity_type: String,
    pub scope_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub claim_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimGraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub predicate: String,
    pub claim_id: String,
    pub content: String,
    pub status: String,
    pub confidence: f32,
    pub salience: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
}

/// LLM-extracted claim candidate (design §4.3), parsed from the `claims`
/// array of the combined extraction response. Validated + canonicalized by
/// the write path; `confidence` is NOT taken from the model — it is derived
/// from `evidence_class` baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimCandidate {
    pub claim_type: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub content: String,
    #[serde(default)]
    pub scope: Option<ClaimScopeHint>,
    /// One of the 6 closed `evidence_class` labels; invalid / missing falls
    /// back to `assistant_inferred` at write time.
    #[serde(default)]
    pub evidence_class: Option<String>,
    #[serde(default)]
    pub salience: Option<f32>,
    #[serde(default)]
    pub temporal: Option<ClaimTemporal>,
    /// Source anchors the model cited, e.g. `"message:..."` / `"memory:42"`.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A live `active` claim loaded for the Deep resolver (expire / merge /
/// conflict). Carries the columns the resolver groups + reasons over; not
/// serialized over the wire (resolver is an internal pipeline).
#[derive(Debug, Clone)]
pub struct ResolveClaim {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub claim_type: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub content: String,
    pub confidence: f32,
    pub confidence_source: String,
    pub salience: f32,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub evidence_count: usize,
    pub manual_evidence_count: usize,
    pub max_evidence_weight: f32,
    pub created_at: String,
    pub updated_at: String,
}

/// Scope hint from the model: `{type: "global"|"agent"|"project", id?}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimScopeHint {
    #[serde(rename = "type")]
    pub scope_type: String,
    #[serde(default)]
    pub id: Option<String>,
}

/// Temporal validity hints from the model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimTemporal {
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_until: Option<String>,
}
