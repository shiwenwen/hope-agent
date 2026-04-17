//! Scanner — pull recent memories from the backend for consolidation.
//!
//! Conservative by design: we don't invent new schema fields just to
//! drive ranking (recall_count / last_accessed_at aren't currently
//! persisted). Instead we lean on `created_at` and the LLM's judgement.

use anyhow::Result;
use chrono::{Duration, Utc};

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
