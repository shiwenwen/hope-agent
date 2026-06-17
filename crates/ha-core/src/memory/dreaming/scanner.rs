//! Scanner — pull recent memories from the backend for consolidation.
//!
//! Conservative by design: we don't invent new schema fields just to
//! drive ranking (recall_count / last_accessed_at aren't currently
//! persisted). Instead we lean on `created_at` and the LLM's judgement.

use anyhow::Result;
use chrono::{Duration, Utc};

use super::types::EvidenceRef;
use crate::memory::{MemoryEntry, MemoryScope};

/// Fetch up to `limit` memory entries created in the last `scope_days`,
/// across Global + all Agent + all Project scopes. Pinned entries are
/// excluded so dreaming doesn't re-promote what's already pinned.
///
/// Synchronous SQLite / vector call — caller wraps it in
/// `tokio::task::spawn_blocking`.
pub fn collect_candidates(scope_days: u32, limit: usize) -> Result<Vec<MemoryEntry>> {
    let Some(backend) = crate::get_memory_backend() else {
        return Ok(Vec::new());
    };

    // `list` returns entries in created_at DESC order so "recent" already
    // gets priority. We over-fetch (limit * 3) and then time-filter +
    // pinned-filter client-side to keep the query simple.
    let raw = backend.list(None, None, limit.saturating_mul(3), 0)?;

    let cutoff = Utc::now() - Duration::days(scope_days.max(1) as i64);
    let cutoff_str = cutoff.to_rfc3339();

    let filtered: Vec<MemoryEntry> = raw
        .into_iter()
        .filter(|e| !e.pinned)
        .filter(|e| e.created_at.as_str() >= cutoff_str.as_str())
        .take(limit)
        .collect();

    Ok(filtered)
}

/// Build the provenance pointers for one scan candidate (Evidence Layer).
///
/// Every candidate traces to its own memory id. It additionally traces to
/// the originating session **only when that session provably exists and is
/// not incognito** — incognito sources must never enter evidence (design
/// §8.1; acceptance: "incognito source 不进入 evidence"). The check is
/// fail-closed: a missing / deleted / errored session (we can't prove it
/// wasn't incognito — incognito sessions are burn-on-close) is treated as
/// not visible and contributes no session ref. The DB lookup lives here;
/// `build_evidence` holds the pure decision logic for unit tests.
pub fn evidence_for_candidate(entry: &MemoryEntry) -> Vec<EvidenceRef> {
    let session_visible = entry
        .source_session_id
        .as_deref()
        .map(session_visible_for_evidence)
        .unwrap_or(false);
    build_evidence(
        entry.id,
        entry.source_session_id.as_deref(),
        session_visible,
    )
}

/// Fail-closed visibility check: a source session may appear in evidence
/// only when its metadata is present **and** `incognito = false`. Missing
/// metadata (deleted / DB unavailable / lookup error) → not visible.
fn session_visible_for_evidence(session_id: &str) -> bool {
    crate::session::lookup_session_meta(Some(session_id))
        .map(|meta| !meta.incognito)
        .unwrap_or(false)
}

/// Pure provenance decision: memory ref always, session ref only when the
/// source session is known and visible (`session_visible`). Separated from
/// the DB lookup so the fail-closed rule is deterministically testable.
pub(crate) fn build_evidence(
    memory_id: i64,
    source_session_id: Option<&str>,
    session_visible: bool,
) -> Vec<EvidenceRef> {
    let mut refs = vec![EvidenceRef::memory(memory_id)];
    if let Some(sid) = source_session_id {
        if session_visible && !sid.is_empty() {
            refs.push(EvidenceRef::session(sid));
        }
    }
    refs
}

/// Format the candidate list into a compact block suitable for inclusion
/// in the narrative prompt. Each line: `[id] (type/scope) content`.
pub fn render_candidates_for_prompt(candidates: &[MemoryEntry]) -> String {
    if candidates.is_empty() {
        return "(no candidates)".to_string();
    }
    candidates
        .iter()
        .map(|m| {
            let content = crate::truncate_utf8(&m.content, 400);
            let scope = match &m.scope {
                MemoryScope::Global => "global".to_string(),
                MemoryScope::Agent { id } => format!("agent:{}", id),
                MemoryScope::Project { id } => format!("project:{}", id),
            };
            format!(
                "[{id}] ({ty}/{scope}) {content}",
                id = m.id,
                ty = m.memory_type.as_str(),
                scope = scope,
                content = content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_always_anchors_to_memory_id() {
        // No source session → memory ref only (session_visible irrelevant).
        let refs = build_evidence(42, None, true);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].source_type, "memory");
        assert_eq!(refs[0].memory_id, Some(42));
        assert_eq!(refs[0].session_id, None);
    }

    #[test]
    fn evidence_adds_session_when_visible() {
        let refs = build_evidence(7, Some("sess-abc"), true);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[1].source_type, "session_message");
        assert_eq!(refs[1].session_id.as_deref(), Some("sess-abc"));
    }

    #[test]
    fn evidence_excludes_session_when_not_visible() {
        // Fail-closed: incognito OR missing/deleted/errored session meta.
        let refs = build_evidence(7, Some("sess-gone"), false);
        assert_eq!(
            refs.len(),
            1,
            "non-visible (incognito/missing) source must not enter evidence"
        );
        assert_eq!(refs[0].source_type, "memory");
    }

    #[test]
    fn evidence_ignores_empty_session_id() {
        let refs = build_evidence(9, Some(""), true);
        assert_eq!(refs.len(), 1);
    }
}
