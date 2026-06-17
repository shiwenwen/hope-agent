//! Shared types for the structured claim layer (next-gen Dreaming).
//!
//! These mirror the `memory_claims` / `memory_evidence` / `memory_claim_links`
//! columns (design §3.2 / §3.3 / §3.3.1), returned by the read API. Free-form
//! JSON columns (`tags_json`, `freshness_policy_json`, `access_scope_json`) are
//! parsed into values for the frontend; everything is camelCase over the wire.

use serde::{Deserialize, Serialize};

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

/// LLM-extracted claim candidate (design §4.3), parsed from the `claims`
/// array of the combined extraction response. Validated + canonicalized by
/// the write path; `confidence` is NOT taken from the model — it is derived
/// from `evidence_class` baseline.
#[derive(Debug, Clone, Deserialize)]
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
    pub valid_until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Scope hint from the model: `{type: "global"|"agent"|"project", id?}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimScopeHint {
    #[serde(rename = "type")]
    pub scope_type: String,
    #[serde(default)]
    pub id: Option<String>,
}

/// Temporal validity hints from the model.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimTemporal {
    #[serde(default)]
    pub valid_from: Option<String>,
    #[serde(default)]
    pub valid_until: Option<String>,
}
