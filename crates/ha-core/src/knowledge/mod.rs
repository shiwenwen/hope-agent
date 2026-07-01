//! Knowledge Base subsystem ("Knowledge Space", see `docs/architecture/knowledge-base.md`).
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
pub mod compile;
pub mod db;
pub mod embedding;
pub mod graph;
pub mod index;
pub mod inject;
pub mod maintenance;
pub mod parser;
pub mod reembed;
pub mod registry;
pub mod rename;
pub mod resolver;
pub mod schema;
pub mod search;
pub mod service;
pub mod source;
pub mod types;
pub mod watcher;

pub use access::{effective_kb_access, ChannelKbContext, KbAccessSource, KnowledgeAccessContext};
pub use chunker::ChunkConfig;
pub use db::IndexDb;
pub use embedding::{
    apply_knowledge_embedding_from_config, disable_knowledge_embedding,
    get_knowledge_embedding_state, knowledge_active_embedding_signature,
    set_knowledge_embedding_default,
};
pub use reembed::{cancel_active_knowledge_reembed_jobs, start_knowledge_reembed_job};
pub use registry::{resolve_kb_dir, KbRoot, KnowledgeRegistry};
pub use rename::{rename_dir, rename_note};
pub use search::KnowledgeSearchConfig;
pub use service::{get_chunk_config, get_search_config, set_chunk_config, set_search_config};
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

    // Collect the bound knowledge chat sessions BEFORE deleting the KB (which
    // cascade-removes their thread rows). These `kind=knowledge` sessions are
    // hidden from the main list / pickers / FTS, so without explicit teardown
    // they'd become unreachable zombies (+ leaked plan/attachment files).
    let thread_sessions = registry.chat_thread_session_ids(kb_id).unwrap_or_default();

    // Step 1: registry rows (KB + attach) in one transaction.
    registry.delete(kb_id)?;

    // Step 1b: tear down the bound knowledge sessions (messages via CASCADE +
    // plan/attachment files on disk). Best-effort, mirrors the rest of cascade.
    if !thread_sessions.is_empty() {
        if let Some(db) = crate::get_session_db() {
            for sid in &thread_sessions {
                if let Err(e) = db.delete_session(sid) {
                    crate::app_warn!(
                        "knowledge",
                        "delete",
                        "delete kb thread session {} failed: {}",
                        sid,
                        e
                    );
                }
            }
        }
    }

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

    // Step 3: managed on-disk directories. Internal KBs own the whole
    // `knowledge/{id}` tree (notes + sources). External vaults must never have
    // their bound root deleted, but their Hope-managed raw-source inbox still
    // lives under `knowledge/{id}/sources` and should be cleaned up.
    if let Ok(dir) = crate::paths::knowledge_dir().map(|d| d.join(sanitize(kb_id))) {
        let target = if kb.is_external() {
            dir.join("sources")
        } else {
            dir
        };
        if target.exists() {
            // Containment guard: refuse to rm a path that escapes the knowledge
            // root (defense in depth; kb_id is a server UUID).
            if let (Ok(canon), Ok(root)) = (target.canonicalize(), crate::paths::knowledge_dir()) {
                let canon_root = root.canonicalize().unwrap_or(root);
                if canon.starts_with(&canon_root) {
                    let _ = std::fs::remove_dir_all(&canon);
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
