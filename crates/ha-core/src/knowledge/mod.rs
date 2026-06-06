//! Knowledge Base subsystem ("Knowledge Space", design `docs/plans/knowledge-base.md`).
//!
//! Zero Tauri dependency (red line). Two storage classes (D9):
//! - **Registry** ([`KnowledgeRegistry`]) — `knowledge_bases` + access bindings in
//!   `sessions.db` (truth source).
//! - **Index cache** ([`IndexDb`]) — note/chunk/link/tag + FTS5 + vec0 in
//!   `~/.hope-agent/knowledge/index.db` (rebuildable from `.md` files).
//!
//! Note files (`.md`) are the single truth source for content; the index is a
//! cache. Internal KBs are app-managed + writable; external (bound) KBs are
//! browse-only in Phase 1 (D11).

pub mod access;
pub mod chunker;
pub mod db;
pub mod embedding;
pub mod graph;
pub mod index;
pub mod inject;
pub mod parser;
pub mod reembed;
pub mod registry;
pub mod rename;
pub mod resolver;
pub mod search;
pub mod service;
pub mod types;
pub mod watcher;

pub use access::{effective_kb_access, KbAccessSource, KnowledgeAccessContext};
pub use chunker::ChunkConfig;
pub use db::IndexDb;
pub use embedding::{
    apply_knowledge_embedding_from_config, disable_knowledge_embedding,
    get_knowledge_embedding_state, knowledge_active_embedding_signature,
    set_knowledge_embedding_default,
};
pub use reembed::{cancel_active_knowledge_reembed_jobs, start_knowledge_reembed_job};
pub use registry::{resolve_kb_dir, KnowledgeRegistry};
pub use rename::{rename_dir, rename_note};
pub use service::{get_chunk_config, set_chunk_config};
pub use types::*;

use anyhow::Result;

/// BLAKE3 over raw bytes → lowercase hex (design D14 hash contract). Used for
/// whole-file `note.content_hash` (no newline normalization, CRLF kept) and
/// per-chunk `note_chunk.content_hash`.
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Full teardown of a knowledge base: registry rows (sessions.db) + index cache
/// rows (index.db) + the internal notes directory on disk. External (bound)
/// roots are **never** deleted — only the index cache for them is pruned.
///
/// Mirrors `project::delete_project_cascade`: the in-`sessions.db` part is one
/// transaction; cross-store side effects (index.db, disk) run last and are
/// best-effort. Returns `false` if the KB was already gone.
pub fn delete_kb_cascade(kb_id: &str) -> Result<bool> {
    let registry =
        crate::get_knowledge_db().ok_or_else(|| anyhow::anyhow!("knowledge db not initialized"))?;
    let Some(kb) = registry.get(kb_id)? else {
        return Ok(false);
    };

    // Step 1: registry rows (KB + attach) in one transaction.
    registry.delete(kb_id)?;

    // Step 2: index cache rows (separate DB, best-effort).
    if let Some(index) = index::get_index_db() {
        if let Err(e) = index.prune_kb(kb_id) {
            crate::app_warn!(
                "knowledge",
                "delete",
                "prune index for kb {} failed: {}",
                kb_id,
                e
            );
        }
    }

    // Step 3: internal notes dir on disk (never touch an external bound root).
    if !kb.is_external() {
        if let Ok(dir) = crate::paths::knowledge_dir().map(|d| d.join(sanitize(kb_id))) {
            if dir.exists() {
                // Containment guard: refuse to rm a path that escapes the
                // knowledge root (defense in depth; kb_id is a server UUID).
                if let (Ok(canon), Ok(root)) = (dir.canonicalize(), crate::paths::knowledge_dir()) {
                    let canon_root = root.canonicalize().unwrap_or(root);
                    if canon.starts_with(&canon_root) {
                        let _ = std::fs::remove_dir_all(&canon);
                    }
                }
            }
        }
    }

    Ok(true)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
