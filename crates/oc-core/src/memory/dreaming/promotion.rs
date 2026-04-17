//! Promotion — flip promoted memories to `pinned=true` in the backend.

use anyhow::Result;

use super::types::PromotionRecord;

/// Apply promotions by toggling `pinned=true` on each memory id.
/// Returns the list of IDs successfully pinned (may be shorter than input
/// if some IDs no longer exist). Synchronous — caller runs in
/// `spawn_blocking`.
pub fn apply_promotions(records: &[PromotionRecord]) -> Result<Vec<i64>> {
    let Some(backend) = crate::get_memory_backend() else {
        return Ok(Vec::new());
    };

    let mut pinned = Vec::new();
    for record in records {
        // Verify the memory still exists before pinning.
        match backend.get(record.memory_id) {
            Ok(Some(entry)) if !entry.pinned => {
                if backend.toggle_pin(record.memory_id, true).is_ok() {
                    pinned.push(record.memory_id);
                }
            }
            Ok(Some(_)) => {
                // Already pinned — leave the record for the diary but skip
                // the toggle. Still counts as "promoted" so the UI reflects
                // the LLM's nomination.
                pinned.push(record.memory_id);
            }
            _ => {
                // Missing entry — skip silently; diary will still list
                // the title/rationale the LLM returned.
            }
        }
    }
    Ok(pinned)
}
