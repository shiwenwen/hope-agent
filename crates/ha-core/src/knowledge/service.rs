//! Owner-plane knowledge operations (design: two auth planes).
//!
//! These back the GUI "Knowledge Space" tab + HTTP `/api/knowledge/*` endpoints.
//! The operator **is** the owner (desktop = local machine; HTTP = API-key
//! holder), so there is no `effective_kb_access` here — the owner sees all their
//! KBs. The agent/session plane (`note_*` tools) is the one that goes through
//! `effective_kb_access`. Keep this split: owner plane = full; tool plane =
//! scoped.

use anyhow::{anyhow, bail, Result};

use super::index;
use super::types::{
    Backlink, KbAccess, KbAttachInput, KnowledgeBaseMeta, Note, NoteReadResult, NoteSearchHit,
    ReferenceableNote, RenameOutcome,
};
use crate::filesystem::{self, WorkspaceScope};

fn registry() -> Result<&'static std::sync::Arc<super::KnowledgeRegistry>> {
    crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))
}

fn index_db() -> Result<std::sync::Arc<super::IndexDb>> {
    index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))
}

/// List KBs with note counts enriched from the index cache.
pub fn list_kb_meta(include_archived: bool) -> Result<Vec<KnowledgeBaseMeta>> {
    let mut metas = registry()?.list(include_archived)?;
    if let Ok(idx) = index_db() {
        for m in &mut metas {
            m.note_count = idx.count_notes(&m.kb.id).unwrap_or(0);
        }
    }
    Ok(metas)
}

/// List a KB's indexed notes (metadata), ordered by path.
pub fn list_notes(kb_id: &str) -> Result<Vec<Note>> {
    index_db()?.list_notes(kb_id)
}

/// Owner read: raw content + outgoing links + backlinks + tags.
pub fn note_read(kb_id: &str, rel_path: &str) -> Result<NoteReadResult> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    let abs = scope
        .resolve_existing(rel_path)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    let bytes = std::fs::read(&abs)?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    let content_hash = super::blake3_hex(&bytes);

    let db = index_db()?;
    let note = db
        .get_note_by_rel_path(kb_id, rel_path)?
        .ok_or_else(|| anyhow!("note not indexed yet: {}", rel_path))?;
    Ok(NoteReadResult {
        kb_id: kb_id.to_string(),
        note_id: note.id,
        rel_path: rel_path.to_string(),
        title: note.title.clone(),
        content,
        content_hash,
        frontmatter_json: note.frontmatter_json.clone(),
        outgoing_links: db.outgoing_links(note.id)?,
        backlinks: db.backlinks(note.id)?,
        tags: db.tags_for_note(note.id)?,
    })
}

/// Owner write (full content). Rejects read-only (external) roots. Re-indexes +
/// emits `knowledge:changed`. Optional stale-write guard against the disk hash.
pub fn note_save(
    kb_id: &str,
    rel_path: &str,
    content: &str,
    expected_file_hash: Option<&str>,
    create_only: bool,
) -> Result<String> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!(
            "knowledge base '{}' root is read-only (external vaults are read-only in Phase 1)",
            kb_id
        );
    }
    if let Some(expected) = expected_file_hash {
        let current = scope
            .resolve_existing(rel_path)
            .ok()
            .and_then(|abs| std::fs::read(&abs).ok())
            .map(|b| super::blake3_hex(&b));
        match current {
            Some(cur) if cur != expected => {
                bail!(
                    "stale write: '{}' changed on disk (expected_file_hash mismatch, current {})",
                    rel_path,
                    cur
                );
            }
            // An expected hash means the caller is editing a file it believes
            // exists. If it's gone (deleted/moved externally), don't silently
            // recreate it — that resurrects a removed note at a stale path.
            None => {
                bail!(
                    "stale write: '{}' was removed on disk (cannot save edits to a deleted note)",
                    rel_path
                );
            }
            _ => {}
        }
    }
    filesystem::project_write_text(&scope, rel_path, content, create_only)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    if let Err(e) = index::reindex_note(kb_id, scope.root(), rel_path) {
        crate::app_warn!("knowledge", "service", "reindex {} failed: {}", rel_path, e);
    }
    emit(kb_id, "upsert");
    Ok(super::blake3_hex(content.as_bytes()))
}

/// Owner delete. Rejects read-only roots.
pub fn note_delete(kb_id: &str, rel_path: &str) -> Result<()> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    filesystem::project_delete(&scope, rel_path, false)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    if let Err(e) = index::remove_note(kb_id, rel_path) {
        crate::app_warn!(
            "knowledge",
            "service",
            "remove index {} failed: {}",
            rel_path,
            e
        );
    }
    emit(kb_id, "delete");
    Ok(())
}

/// Owner rename/move. Rejects read-only roots. Moves the `.md` file, rebuilds
/// the index entry, and **rewrites inbound `[[ ]]` links** in other notes so
/// path-form / filename-derived references stay intact (#9). Returns the new rel
/// path + how many references were rewritten (for the "updated N references" UI).
///
/// Runs on a blocking worker (like `rename_dir`): the rewrite reads/reindexes —
/// and, with knowledge embedding on, re-embeds — every linking note, which must
/// not block a tokio runtime thread.
pub async fn note_rename(kb_id: &str, from_rel: &str, to_rel: &str) -> Result<RenameOutcome> {
    let kb = kb_id.to_string();
    let from = from_rel.to_string();
    let to = to_rel.to_string();
    tokio::task::spawn_blocking(move || super::rename::rename_note(&kb, &from, &to, None))
        .await
        .map_err(|e| anyhow!("note_rename task failed: {e}"))?
}

/// Owner: list every directory under the KB root (relative, "/"-joined). Lets the
/// GUI show (possibly empty) folders that the index — which only tracks `.md` —
/// does not record. Walks disk off the async worker; skips `IGNORE_DIRS` to stay
/// consistent with the indexer (no node_modules/logseq blowup on big vaults).
pub async fn list_dirs(kb_id: &str) -> Result<Vec<String>> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    let root = scope.root().to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let root = root.canonicalize().unwrap_or(root);
        let mut out = Vec::new();
        for entry in ignore::WalkBuilder::new(&root)
            .hidden(true)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .parents(false)
            .filter_entry(|e| {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if let Some(name) = e.file_name().to_str() {
                        return !index::IGNORE_DIRS.contains(&name);
                    }
                }
                true
            })
            .build()
            .flatten()
        {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if path == root {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(&root) {
                let s = rel.to_string_lossy().replace('\\', "/");
                if !s.is_empty() {
                    out.push(s);
                }
            }
        }
        out.sort();
        Ok(out)
    })
    .await
    .map_err(|e| anyhow!("list_dirs task failed: {e}"))?
}

/// Owner: create an (empty) folder under the KB root. Rejects read-only roots.
pub fn mkdir(kb_id: &str, rel: &str) -> Result<String> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    let res =
        filesystem::project_mkdir(&scope, rel).map_err(|e| anyhow!(e.message().to_string()))?;
    emit(kb_id, "mkdir");
    Ok(res.rel_path)
}

/// Owner: rename/move a folder and everything inside it (one fs rename), then
/// reconcile the index (prune old paths, index new) + **rewrite inbound path-form
/// `[[ ]]` links** across the KB so they follow the moved notes (#9). Runs off
/// the async worker but completes before returning, so the caller can immediately
/// reopen the moved notes.
pub async fn rename_dir(kb_id: &str, from_rel: &str, to_rel: &str) -> Result<RenameOutcome> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    drop(scope);
    let kb = kb_id.to_string();
    let from = from_rel.to_string();
    let to = to_rel.to_string();
    let outcome = tokio::task::spawn_blocking(move || super::rename::rename_dir(&kb, &from, &to))
        .await
        .map_err(|e| anyhow!("rename_dir task failed: {e}"))??;
    Ok(outcome)
}

/// Owner: every broken (dangling) link in a KB — the "maintenance" panel feed.
pub fn broken_links(kb_id: &str) -> Result<Vec<super::types::BrokenLink>> {
    index_db()?.list_broken_links(kb_id)
}

/// Owner: orphan notes (no resolved inbound or outbound link) in a KB.
pub fn orphans(kb_id: &str) -> Result<Vec<Note>> {
    index_db()?.list_orphan_notes(kb_id)
}

/// Owner-plane node cap for the graph view. Force-directed layout stays
/// responsive well below this; beyond it we return the most-connected slice
/// flagged `truncated` so the UI can say so rather than freeze on a huge vault.
const OWNER_GRAPH_NODE_CAP: usize = 2000;

/// Owner: the whole-KB link graph (WS1), capped to the most-connected nodes.
pub fn graph(kb_id: &str) -> Result<super::types::KnowledgeGraph> {
    let db = index_db()?;
    let g = super::graph::build_kb_graph(&db, kb_id)?;
    Ok(super::graph::cap_nodes(g, OWNER_GRAPH_NODE_CAP))
}

/// Owner: read a note by its `[[ ]]` reference (title or `folder/note` path
/// form), resolved deterministically (design #8) over the KB — the single source
/// of truth for transclusion (`![[ ]]`) preview. Returns `None` when the ref
/// resolves to nothing (broken embed) so the UI shows a placeholder.
pub fn note_read_ref(kb_id: &str, reference: &str) -> Result<Option<NoteReadResult>> {
    let db = index_db()?;
    let notes = db.note_refs(kb_id)?;
    // Strip `#anchor` / `|alias` before resolving (the resolver expects a clean
    // target, like the parser stores for graph edges) so `![[Note#H]]` /
    // `![[Note|label]]` embeds resolve instead of showing as broken.
    let target = super::parser::wikilink_target(reference);
    let Some(id) = super::resolver::resolve(target, &notes) else {
        return Ok(None);
    };
    let Some(rel) = notes.into_iter().find(|n| n.id == id).map(|n| n.rel_path) else {
        return Ok(None);
    };
    // The ref resolved against the index, but the `.md` may be gone on disk
    // (deleted/moved before the watcher reconciled). Honor the broken-embed
    // contract (`None`) instead of surfacing a hard error to the caller.
    match note_read(kb_id, &rel) {
        Ok(r) => Ok(Some(r)),
        Err(e) => {
            crate::app_warn!(
                "knowledge",
                "service",
                "note_read_ref resolved '{}' but read failed (stale index?): {}",
                rel,
                e
            );
            Ok(None)
        }
    }
}

/// Owner: delete a folder and all its contents (recursive), then reconcile index.
/// rm -rf + reconcile run off the async worker.
pub async fn delete_dir(kb_id: &str, rel: &str) -> Result<()> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    drop(scope);
    let kb = kb_id.to_string();
    let rel = rel.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let scope =
            WorkspaceScope::for_knowledge(&kb).map_err(|e| anyhow!(e.message().to_string()))?;
        filesystem::project_delete(&scope, &rel, true)
            .map_err(|e| anyhow!(e.message().to_string()))?;
        if let Err(e) = index::reindex_kb(&kb, false) {
            crate::app_warn!("knowledge", "service", "delete_dir reindex failed: {}", e);
        }
        Ok(())
    })
    .await
    .map_err(|e| anyhow!("delete_dir task failed: {e}"))??;
    emit(kb_id, "delete");
    Ok(())
}

/// Owner backlinks for a note path.
pub fn backlinks(kb_id: &str, rel_path: &str) -> Result<Vec<Backlink>> {
    let db = index_db()?;
    let note = db
        .get_note_by_rel_path(kb_id, rel_path)?
        .ok_or_else(|| anyhow!("note not indexed: {}", rel_path))?;
    db.backlinks(note.id)
}

/// Owner hybrid search. `kb_id = None` searches all non-archived KBs.
pub fn search(kb_id: Option<&str>, query: &str, limit: usize) -> Result<Vec<NoteSearchHit>> {
    let db = index_db()?;
    let kb_ids: Vec<String> = match kb_id {
        Some(k) => vec![k.to_string()],
        None => registry()?
            .list(false)?
            .into_iter()
            .map(|m| m.kb.id)
            .collect(),
    };
    super::search::search_notes(&db, &kb_ids, query, limit)
}

/// Owner: flat list of notes the chat composer can reference via `[[ ]]`, across
/// the KBs reachable from this chat context. For an existing session this is the
/// effective (session ∪ project) attach set; for a brand-new chat (no session
/// yet) the caller passes the staged draft kbIds directly. Owner plane — no
/// `effective_kb_access` here (the user picks their own notes; `[[note]]`
/// injection re-gates at send time). Archived KBs are skipped.
pub fn list_referenceable_notes(
    session_id: Option<&str>,
    project_id: Option<&str>,
    draft_kb_ids: &[String],
) -> Result<Vec<ReferenceableNote>> {
    let reg = registry()?;
    let mut kb_ids: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(sid) = session_id {
        for (kb_id, _access) in reg.list_session_attachments(sid)? {
            if seen.insert(kb_id.clone()) {
                kb_ids.push(kb_id);
            }
        }
        if let Some(pid) = project_id {
            for (kb_id, _access) in reg.list_project_attachments(pid)? {
                if seen.insert(kb_id.clone()) {
                    kb_ids.push(kb_id);
                }
            }
        }
    } else {
        for kb_id in draft_kb_ids {
            if seen.insert(kb_id.clone()) {
                kb_ids.push(kb_id.clone());
            }
        }
    }

    let db = index_db()?;
    let mut out = Vec::new();
    for kb_id in kb_ids {
        let Ok(Some(kb)) = reg.get(&kb_id) else {
            continue;
        };
        if kb.archived {
            continue;
        }
        for n in db.list_notes(&kb_id).unwrap_or_default() {
            out.push(ReferenceableNote {
                kb_id: kb_id.clone(),
                kb_name: kb.name.clone(),
                kb_emoji: kb.emoji.clone(),
                rel_path: n.rel_path,
                title: n.title,
            });
        }
    }
    Ok(out)
}

/// Owner: every tag used across a KB's notes, ordered by frequency then name.
/// Feeds the editor `#tag` autocomplete (design D13).
pub fn list_tags(kb_id: &str) -> Result<Vec<String>> {
    Ok(index_db()?
        .all_tags(&[kb_id.to_string()])?
        .into_iter()
        .map(|(tag, _count)| tag)
        .collect())
}

/// Apply composer-staged KB attaches to a freshly auto-created session (the
/// `chat` command's auto-create branch, mirroring draft `working_dir`). No-op for
/// incognito sessions — D10: incognito gets zero KB and leaves no trace, so this
/// is the authoritative backend guard against a staged draft persisting an attach
/// row onto an incognito session regardless of any frontend race.
pub fn apply_draft_attachments(session_id: &str, incognito: bool, attaches: &[KbAttachInput]) {
    if incognito || attaches.is_empty() {
        return;
    }
    let Some(reg) = crate::get_knowledge_db() else {
        return;
    };
    for a in attaches {
        let access = KbAccess::from_str_lenient(&a.access);
        match reg.attach_session(session_id, &a.kb_id, access) {
            Ok(()) => emit(&a.kb_id, "attach"),
            Err(e) => crate::app_warn!(
                "knowledge",
                "service",
                "draft attach {} failed: {}",
                a.kb_id,
                e
            ),
        }
    }
}

fn emit(kb_id: &str, op: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "knowledge:changed",
            serde_json::json!({ "kbId": kb_id, "op": op }),
        );
    }
}

// ── Chunking config (advanced; owner plane, GUI-only) ────────────

/// Current chunking parameters, clamped to sane bounds.
pub fn get_chunk_config() -> super::ChunkConfig {
    crate::config::cached_config().knowledge_chunk.clamped()
}

/// Persist new chunking parameters and kick off a full reindex (re-chunk +
/// re-embed) of every KB so existing notes pick up the new sizing. Values are
/// clamped first; the clamped result is returned (and saved).
///
/// The reindex runs through `start_knowledge_reembed_job(Some(all_ids), ...)`,
/// which works whether or not vector search is enabled (embedding on → re-embed;
/// off → FTS-only re-chunk) and never stamps the embedding signature (a chunk
/// change isn't a model-coverage event). A spawn error (e.g. embedding enabled
/// but the model is missing) is logged, not fatal — the config is already saved
/// and the user can rebuild manually.
pub fn set_chunk_config(
    max_chars: usize,
    overlap_chars: usize,
    source: &str,
) -> Result<super::ChunkConfig> {
    let cfg = super::ChunkConfig {
        max_chars,
        overlap_chars,
    }
    .clamped();
    let to_save = cfg.clone();
    crate::config::mutate_config(("knowledge_chunk", source), move |store| {
        store.knowledge_chunk = to_save.clone();
        Ok(())
    })?;

    let ids = registry()?.list_all_ids()?;
    if !ids.is_empty() {
        if let Err(e) = super::start_knowledge_reembed_job(Some(ids), source) {
            crate::app_warn!(
                "knowledge",
                "service",
                "chunk config saved but reindex spawn failed: {}",
                e
            );
        }
    }
    Ok(cfg)
}
