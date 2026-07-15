//! Existing-memory backfill (Phase 2.5): bring legacy `memories` rows into the
//! claim world WITHOUT changing current prompt injection.
//!
//! Deterministic + rule-only (no LLM): `memory_type` maps to claim_type /
//! subject / predicate, the memory `content` is carried verbatim into the claim
//! `content`, and `source` / `pinned` derive `evidence_class` / `salience`.
//! Every backfilled claim links to its source memory with a `detached` link, so
//! the claim's lifecycle can NEVER hide the pre-existing memory (the injection
//! hidden-set only considers `managed` links). That's what keeps the Phase 2.5
//! acceptance — "backfill doesn't change current injection" — true.
//!
//! Low-risk auto-active policy: ONLY a pinned profile/preference memory
//! (`user` / `feedback`) is written `active` — a pin is the user's explicit
//! keep signal. Everything else lands in `needs_review` (the review queue), so
//! the user confirms before it can ever inject.

use anyhow::{anyhow, Result};
use serde::Serialize;

use super::store;
use super::write;
use crate::memory::{MemoryEntry, MemoryScope, MemoryType};

/// Page size for scanning the `memories` table during plan / apply.
const SCAN_PAGE: usize = 500;
/// Max preview rows returned by the dry-run plan (summary counts stay exact).
const PREVIEW_LIMIT: usize = 200;

/// One memory's proposed backfill claim — a dry-run preview row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillCandidate {
    pub memory_id: i64,
    pub scope_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub claim_type: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub content: String,
    pub tags: Vec<String>,
    pub evidence_class: String,
    pub confidence: f32,
    pub salience: f32,
    pub pinned: bool,
    /// "active" (low-risk auto) | "needs_review".
    pub proposed_status: String,
}

/// Exact counts over the whole `memories` table, independent of the capped
/// preview list.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillSummary {
    pub total_memories: usize,
    pub already_linked: usize,
    pub candidates: usize,
    pub auto_active: usize,
    pub needs_review: usize,
}

/// Dry-run plan: exact summary + a capped candidate preview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillPlan {
    pub summary: BackfillSummary,
    pub candidates: Vec<BackfillCandidate>,
    pub preview_truncated: bool,
}

/// Result of applying the backfill.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillApplyResult {
    pub created: usize,
    pub auto_active: usize,
    pub needs_review: usize,
    /// Memories that vanished or got linked after the scan (raced) — no claim
    /// created, but not an error either.
    pub skipped: usize,
    pub failed: usize,
}

// ── Deterministic rule mapping (pure, unit-tested) ──────────────

fn claim_type_for(mt: &MemoryType) -> &'static str {
    match mt {
        MemoryType::User => "user_profile",
        MemoryType::Feedback => "preference",
        MemoryType::Project => "project_fact",
        MemoryType::Reference => "reference",
    }
}

fn subject_for(mt: &MemoryType) -> &'static str {
    match mt {
        MemoryType::User | MemoryType::Feedback => "user",
        MemoryType::Project => "project",
        MemoryType::Reference => "reference",
    }
}

/// Coarse placeholder predicates — Deep consolidation (Phase 3, LLM) refines
/// the structure later; the human-readable `content` carries the real meaning.
fn predicate_for(mt: &MemoryType) -> &'static str {
    match mt {
        MemoryType::User => "is",
        MemoryType::Feedback => "prefers",
        MemoryType::Project => "about",
        MemoryType::Reference => "references",
    }
}

/// Manual memories (`source="user"`) are explicit user statements; everything
/// else (auto-extracted / imported) is treated as assistant-inferred — a
/// conservative baseline the user can upgrade during review.
fn evidence_class_for(source: &str) -> &'static str {
    if source == "user" {
        "explicit_user_statement"
    } else {
        "assistant_inferred"
    }
}

fn salience_for(pinned: bool) -> f32 {
    if pinned {
        0.9
    } else {
        0.5
    }
}

/// Low-risk auto-active policy: ONLY a pinned profile/preference memory
/// (`user` / `feedback`) is auto-activated — a pin is the user's explicit keep
/// signal. Everything else waits in `needs_review`.
fn proposed_status_for(mt: &MemoryType, pinned: bool) -> &'static str {
    if pinned && matches!(mt, MemoryType::User | MemoryType::Feedback) {
        "active"
    } else {
        "needs_review"
    }
}

/// The claim's coarse object slot: normalized content, capped so a long memory
/// doesn't bloat the (subject, predicate, object) key.
fn object_for(content: &str) -> String {
    let normalized = write::normalize_object(content);
    crate::truncate_utf8(&normalized, 200).to_string()
}

fn scope_columns(scope: &MemoryScope) -> (String, Option<String>) {
    match scope {
        MemoryScope::Global => ("global".to_string(), None),
        MemoryScope::Agent { id } => ("agent".to_string(), Some(id.clone())),
        MemoryScope::Project { id } => ("project".to_string(), Some(id.clone())),
    }
}

/// Build the deterministic backfill candidate for one memory. Pure.
pub fn candidate_from_memory(m: &MemoryEntry) -> BackfillCandidate {
    let (scope_type, scope_id) = scope_columns(&m.scope);
    let evidence_class = evidence_class_for(&m.source);
    let confidence = write::confidence_baseline(evidence_class);
    BackfillCandidate {
        memory_id: m.id,
        scope_type,
        scope_id,
        claim_type: claim_type_for(&m.memory_type).to_string(),
        subject: subject_for(&m.memory_type).to_string(),
        predicate: predicate_for(&m.memory_type).to_string(),
        object: object_for(&m.content),
        content: m.content.clone(),
        tags: m.tags.clone(),
        evidence_class: evidence_class.to_string(),
        confidence,
        salience: salience_for(m.pinned),
        pinned: m.pinned,
        proposed_status: proposed_status_for(&m.memory_type, m.pinned).to_string(),
    }
}

// ── Orchestration ───────────────────────────────────────────────

/// Dry-run: scan every memory, skip those already represented in the claim
/// world (idempotent re-runs + memories the live dual-write already linked),
/// and return exact counts + a capped candidate preview. Writes nothing.
pub fn plan_backfill() -> Result<BackfillPlan> {
    let backend =
        crate::get_memory_backend().ok_or_else(|| anyhow!("memory backend not initialised"))?;
    let linked = store::all_linked_memory_ids()?;

    let mut summary = BackfillSummary::default();
    let mut candidates: Vec<BackfillCandidate> = Vec::new();
    let mut offset = 0usize;
    loop {
        let batch = backend.list(None, None, SCAN_PAGE, offset)?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for m in &batch {
            summary.total_memories += 1;
            if linked.contains(&m.id) {
                summary.already_linked += 1;
                continue;
            }
            let c = candidate_from_memory(m);
            summary.candidates += 1;
            if c.proposed_status == "active" {
                summary.auto_active += 1;
            } else {
                summary.needs_review += 1;
            }
            if candidates.len() < PREVIEW_LIMIT {
                candidates.push(c);
            }
        }
        offset += batch_len;
        if batch_len < SCAN_PAGE {
            break;
        }
    }

    let preview_truncated = summary.candidates > candidates.len();
    Ok(BackfillPlan {
        summary,
        candidates,
        preview_truncated,
    })
}

/// Apply the backfill: re-scan deterministically (NOT trusting any client-sent
/// candidate list — owner plane, and the rule mapping makes apply == plan) and
/// write a claim + `memory` evidence + `detached` link for every not-yet-linked
/// memory. Best-effort per item: a failure logs and continues.
pub fn apply_backfill() -> Result<BackfillApplyResult> {
    let backend =
        crate::get_memory_backend().ok_or_else(|| anyhow!("memory backend not initialised"))?;
    let linked = store::all_linked_memory_ids()?;

    let mut result = BackfillApplyResult::default();
    let mut offset = 0usize;
    loop {
        let batch = backend.list(None, None, SCAN_PAGE, offset)?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for m in &batch {
            if linked.contains(&m.id) {
                continue;
            }
            let c = candidate_from_memory(m);
            match store::write_backfill_claim(&c) {
                Ok(Some(_)) => {
                    result.created += 1;
                    if c.proposed_status == "active" {
                        result.auto_active += 1;
                    } else {
                        result.needs_review += 1;
                    }
                }
                // Raced: memory vanished or got linked between scan and write.
                Ok(None) => {
                    result.skipped += 1;
                }
                Err(e) => {
                    crate::app_warn!(
                        "memory",
                        "claim_backfill",
                        "backfill write failed for memory {}: {}",
                        m.id,
                        e
                    );
                    result.failed += 1;
                }
            }
        }
        offset += batch_len;
        if batch_len < SCAN_PAGE {
            break;
        }
    }
    crate::app_info!(
        "memory",
        "claim_backfill",
        "backfill applied: created={} active={} needs_review={} skipped={} failed={}",
        result.created,
        result.auto_active,
        result.needs_review,
        result.skipped,
        result.failed
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: i64, mt: MemoryType, source: &str, pinned: bool, content: &str) -> MemoryEntry {
        MemoryEntry {
            id,
            memory_type: mt,
            scope: MemoryScope::Global,
            content: content.to_string(),
            tags: vec![],
            source: source.to_string(),
            source_session_id: None,
            pinned,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            relevance_score: None,
            retrieval_evidence: None,
            attachment_path: None,
            attachment_mime: None,
        }
    }

    #[test]
    fn pinned_preference_is_auto_active() {
        let c = candidate_from_memory(&mem(
            1,
            MemoryType::Feedback,
            "user",
            true,
            "Prefers terse replies",
        ));
        assert_eq!(c.proposed_status, "active");
        assert_eq!(c.claim_type, "preference");
        assert_eq!(c.subject, "user");
        assert_eq!(c.evidence_class, "explicit_user_statement");
        assert!((c.salience - 0.9).abs() < 1e-6);
        // content carried verbatim; object is the normalized form.
        assert_eq!(c.content, "Prefers terse replies");
    }

    #[test]
    fn pinned_user_fact_is_auto_active() {
        let c = candidate_from_memory(&mem(2, MemoryType::User, "user", true, "Name is Wen"));
        assert_eq!(c.proposed_status, "active");
        assert_eq!(c.claim_type, "user_profile");
    }

    #[test]
    fn unpinned_memory_needs_review() {
        let c = candidate_from_memory(&mem(3, MemoryType::User, "user", false, "Lives in Berlin"));
        assert_eq!(c.proposed_status, "needs_review");
    }

    #[test]
    fn pinned_non_preference_needs_review() {
        // A pinned project/reference fact is NOT auto-active — only user /
        // feedback preferences are low-risk enough to skip review.
        assert_eq!(
            candidate_from_memory(&mem(4, MemoryType::Project, "user", true, "Uses Tauri"))
                .proposed_status,
            "needs_review"
        );
        assert_eq!(
            candidate_from_memory(&mem(5, MemoryType::Reference, "user", true, "https://x"))
                .proposed_status,
            "needs_review"
        );
    }

    #[test]
    fn auto_source_is_assistant_inferred() {
        let c = candidate_from_memory(&mem(6, MemoryType::User, "auto", false, "x"));
        assert_eq!(c.evidence_class, "assistant_inferred");
        assert!((c.confidence - 0.45).abs() < 1e-6);
    }

    #[test]
    fn manual_source_confidence_is_explicit_baseline() {
        let c = candidate_from_memory(&mem(7, MemoryType::Feedback, "user", false, "x"));
        assert_eq!(c.evidence_class, "explicit_user_statement");
        assert!((c.confidence - 0.85).abs() < 1e-6);
        // Unpinned still needs review even with high-confidence evidence class.
        assert_eq!(c.proposed_status, "needs_review");
    }
}
