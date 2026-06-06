//! Indexer: turn `.md` files into index rows, and reconcile a KB against disk.
//!
//! Internal KBs are reindexed synchronously at each write (we own the only
//! writer). External (bound) KBs are reconciled on bind / startup / open and
//! kept fresh by the [`super::watcher`] (D6). All file IO is blocking — call
//! from a blocking context or `spawn_blocking`.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use super::db::{IndexDb, NoteIndexInput};
use super::{chunker, parser};

static INDEX_DB: OnceLock<Arc<IndexDb>> = OnceLock::new();

/// Directory names skipped while scanning a KB root (built-in defaults; the
/// configurable ignore-rule UI is Phase 2). Dotfiles/dirs (`.git`, `.obsidian`,
/// `.trash`) are skipped separately via the hidden filter.
pub(crate) const IGNORE_DIRS: &[&str] = &["node_modules", "logseq", ".obsidian", ".trash", ".git"];

pub fn get_index_db() -> Option<Arc<IndexDb>> {
    INDEX_DB.get().cloned()
}

pub fn set_index_db(db: Arc<IndexDb>) {
    let _ = INDEX_DB.set(db);
}

/// Open `index.db`, install the embedding provider (if knowledge embedding is
/// configured), and register the global. Called once at startup.
pub fn init_index_db() -> Result<()> {
    let path = crate::paths::knowledge_index_db_path()?;
    let db = Arc::new(IndexDb::open(&path)?);
    apply_embedding_to_index(&db);
    set_index_db(db);
    Ok(())
}

/// Resolve the active **knowledge** embedding model (`knowledge_embedding`,
/// independent of memory — D7) and install it on the index DB so note chunks
/// embed under that model/signature. No-op (clears) when knowledge embedding is
/// disabled or unresolved; note vector search then degrades to FTS-only.
pub fn apply_embedding_to_index(db: &IndexDb) {
    let store = crate::config::cached_config();
    if !store.knowledge_embedding.enabled {
        db.clear_embedder();
        return;
    }
    match crate::memory::resolve_memory_embedding_config(
        &store.knowledge_embedding,
        &store.embedding_models,
    ) {
        Ok(Some((_, config, _sig))) => match crate::memory::create_embedding_provider(&config) {
            Ok(p) => db.set_embedder(p),
            Err(e) => {
                crate::app_warn!(
                    "knowledge",
                    "embedding",
                    "failed to init note embedding provider: {}",
                    e
                );
                db.clear_embedder();
            }
        },
        _ => db.clear_embedder(),
    }
}

/// Summary of a KB reconcile pass.
#[derive(Debug, Clone, Default)]
pub struct ReindexReport {
    pub changed: usize,
    pub removed: usize,
    pub total: usize,
}

/// (Re)index a single note file and re-resolve KB links. Used by write tools
/// and the watcher for one-file changes.
pub fn reindex_note(kb_id: &str, root: &Path, rel_path: &str) -> Result<()> {
    let db = get_index_db().ok_or_else(|| anyhow::anyhow!("knowledge index not initialized"))?;
    reindex_one(&db, kb_id, root, rel_path)?;
    db.reresolve_kb_links(kb_id)?;
    Ok(())
}

/// (Re)index a single note file **without** re-resolving KB links. The caller
/// must invoke `IndexDb::reresolve_kb_links` once after a batch of these. Used by
/// the rename link-rewriter, which touches many source notes and only needs one
/// resolve pass at the end (avoids O(files × links) re-resolves).
pub fn reindex_note_no_resolve(kb_id: &str, root: &Path, rel_path: &str) -> Result<()> {
    let db = get_index_db().ok_or_else(|| anyhow::anyhow!("knowledge index not initialized"))?;
    reindex_one(&db, kb_id, root, rel_path)
}

/// Remove a single note from the index and re-resolve KB links.
pub fn remove_note(kb_id: &str, rel_path: &str) -> Result<()> {
    let db = get_index_db().ok_or_else(|| anyhow::anyhow!("knowledge index not initialized"))?;
    db.delete_note(kb_id, rel_path)?;
    db.reresolve_kb_links(kb_id)?;
    Ok(())
}

/// (Re)index a single note by its KB-relative path, resolving the KB root for
/// the caller. Used by the per-note "rebuild index" context-menu action.
pub fn reindex_note_by_path(kb_id: &str, rel_path: &str) -> Result<()> {
    let root = super::resolve_kb_dir(kb_id)?.dir;
    let root = root.canonicalize().unwrap_or(root);
    reindex_note(kb_id, &root, rel_path)
}

/// Re-index every `.md` under a folder (KB-relative `rel_dir`, `""` = root),
/// then re-resolve KB links once. Used by the per-folder "rebuild index"
/// context-menu action. Unlike [`reindex_kb`] this does not prune deleted files
/// — it just rebuilds the current contents of that subtree (FTS + vectors if
/// embedding is enabled).
pub fn reindex_dir(kb_id: &str, rel_dir: &str) -> Result<ReindexReport> {
    let db = get_index_db().ok_or_else(|| anyhow::anyhow!("knowledge index not initialized"))?;
    let root = super::resolve_kb_dir(kb_id)?.dir;
    let root = root.canonicalize().unwrap_or(root);

    let prefix = {
        let trimmed = rel_dir.trim_matches('/');
        if trimmed.is_empty() {
            String::new()
        } else {
            format!("{trimmed}/")
        }
    };

    let mut report = ReindexReport::default();
    for rel in scan_markdown_files(&root) {
        if !prefix.is_empty() && !rel.starts_with(&prefix) {
            continue;
        }
        report.total += 1;
        if let Err(e) = reindex_one(&db, kb_id, &root, &rel) {
            crate::app_warn!("knowledge", "index", "reindex {} failed: {}", rel, e);
            continue;
        }
        report.changed += 1;
    }
    db.reresolve_kb_links(kb_id)?;
    Ok(report)
}

/// Reconcile a whole KB against disk: upsert changed files (by mtime unless
/// `full`), prune deleted, re-resolve links once. The expensive full scan +
/// embedding path; run off the request thread.
pub fn reindex_kb(kb_id: &str, full: bool) -> Result<ReindexReport> {
    let db = get_index_db().ok_or_else(|| anyhow::anyhow!("knowledge index not initialized"))?;
    let root = super::resolve_kb_dir(kb_id)?.dir;
    let root = root.canonicalize().unwrap_or(root);

    let disk = scan_markdown_files(&root);
    let disk_set: HashSet<&str> = disk.iter().map(|s| s.as_str()).collect();

    let existing = db.note_index_state(kb_id)?;
    let existing_map: HashMap<String, i64> = existing
        .iter()
        .map(|(rel, mtime, _)| (rel.clone(), *mtime))
        .collect();

    let mut report = ReindexReport {
        total: disk.len(),
        ..Default::default()
    };

    // Prune notes whose files are gone.
    for (rel, _, _) in &existing {
        if !disk_set.contains(rel.as_str()) {
            if db.delete_note(kb_id, rel).unwrap_or(false) {
                report.removed += 1;
            }
        }
    }

    // Upsert changed / new files.
    for rel in &disk {
        let abs = root.join(rel);
        let mtime = std::fs::metadata(&abs)
            .ok()
            .map(file_mtime_millis)
            .unwrap_or(0);
        if !full {
            if let Some(prev) = existing_map.get(rel) {
                if *prev == mtime && mtime != 0 {
                    continue;
                }
            }
        }
        if let Err(e) = reindex_one(&db, kb_id, &root, rel) {
            crate::app_warn!("knowledge", "index", "reindex {} failed: {}", rel, e);
            continue;
        }
        report.changed += 1;
    }

    db.reresolve_kb_links(kb_id)?;
    Ok(report)
}

/// Spawn a background reconcile of a KB, emitting `knowledge:changed` on
/// completion. Used for bind / startup of (potentially large) external vaults.
pub fn spawn_reindex_kb(kb_id: String, full: bool) {
    tokio::task::spawn_blocking(move || match reindex_kb(&kb_id, full) {
        Ok(report) => {
            crate::app_info!(
                "knowledge",
                "index",
                "reindexed kb {}: {} changed, {} removed, {} total",
                kb_id,
                report.changed,
                report.removed,
                report.total
            );
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "knowledge:changed",
                    serde_json::json!({ "kbId": kb_id, "op": "reindex" }),
                );
            }
        }
        Err(e) => crate::app_warn!("knowledge", "index", "reindex kb {} failed: {}", kb_id, e),
    });
}

/// Reconcile every registered KB at startup (best-effort, off-thread).
pub fn spawn_startup_reconcile() {
    let Some(registry) = crate::get_knowledge_db() else {
        return;
    };
    let ids = registry.list_all_ids().unwrap_or_default();
    for id in ids {
        spawn_reindex_kb(id, false);
    }
}

// ── internals ───────────────────────────────────────────────────

fn reindex_one(db: &IndexDb, kb_id: &str, root: &Path, rel_path: &str) -> Result<()> {
    let abs = root.join(rel_path);
    let bytes = std::fs::read(&abs)?;
    let content_hash = super::blake3_hex(&bytes);
    let content = String::from_utf8_lossy(&bytes).to_string();
    let meta = std::fs::metadata(&abs)?;
    let mtime = file_mtime_millis(meta.clone());
    let size = meta.len() as i64;

    let parsed = parser::parse_document(&content);
    let chunk_cfg = crate::config::cached_config().knowledge_chunk.clamped();
    let chunks = chunker::chunk(&content, &parsed, &chunk_cfg);
    let title = parsed.title.clone().unwrap_or_else(|| file_stem(rel_path));

    let (chunk_embeddings, embedding_signature) = embed_chunks(db, &chunks);

    let input = NoteIndexInput {
        kb_id: kb_id.to_string(),
        rel_path: rel_path.to_string(),
        title,
        frontmatter_json: parsed.frontmatter_json,
        mtime,
        size,
        content_hash,
        chunks,
        chunk_embeddings,
        embedding_signature,
        links: parsed.links,
        tags: parsed.tags,
    };
    db.replace_note_index(input)?;
    Ok(())
}

/// Embed chunk bodies with the index's active provider. Returns
/// `(Some(vectors), Some(signature))` or `(None, None)` when embedding is off /
/// fails (FTS-only degradation).
fn embed_chunks(
    db: &IndexDb,
    chunks: &[chunker::ParsedChunk],
) -> (Option<Vec<Vec<f32>>>, Option<String>) {
    if chunks.is_empty() {
        return (None, None);
    }
    let Some(embedder) = db.embedder() else {
        return (None, None);
    };
    let signature = super::embedding::knowledge_active_embedding_signature();
    let bodies: Vec<String> = chunks.iter().map(|c| c.body.clone()).collect();
    match embedder.embed_batch(&bodies) {
        Ok(vecs) if vecs.len() == chunks.len() => (Some(vecs), signature),
        Ok(_) => (None, None),
        Err(e) => {
            crate::app_warn!("knowledge", "embedding", "embed batch failed: {}", e);
            (None, None)
        }
    }
}

/// Recursively collect `*.md` / `*.markdown` rel-paths under `root`, skipping
/// hidden + ignored directories.
fn scan_markdown_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .filter_entry(|e| {
            // Skip ignored directories by name.
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = e.file_name().to_str() {
                    return !IGNORE_DIRS.contains(&name);
                }
            }
            true
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
            .unwrap_or(false);
        if !is_md {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    out
}

fn file_mtime_millis(meta: std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn file_stem(rel_path: &str) -> String {
    let p = rel_path.replace('\\', "/");
    let last = p.rsplit('/').next().unwrap_or(&p);
    let stem = last
        .strip_suffix(".markdown")
        .or_else(|| last.strip_suffix(".md"))
        .unwrap_or(last);
    stem.to_string()
}
