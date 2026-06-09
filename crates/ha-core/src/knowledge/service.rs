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
    Backlink, CreateKnowledgeBaseInput, KbAccess, KbAttachInput, KnowledgeBaseMeta, Note,
    NoteReadResult, NoteSearchHit, ReferenceableNote, RenameOutcome,
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
            "knowledge base '{}' root is read-only (enable external editing in the space settings to write a bound vault)",
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

/// Owner: read the user-pinned graph layout (Batch J).
pub fn graph_layout(kb_id: &str) -> Result<Vec<super::types::GraphNodePosition>> {
    registry()?.get_graph_layout(kb_id)
}

/// Owner: replace the pinned graph layout (Batch J). Empty = reset.
pub fn save_graph_layout(kb_id: &str, positions: &[super::types::GraphNodePosition]) -> Result<()> {
    registry()?.save_graph_layout(kb_id, positions)
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
        Ok(mut r) => {
            // Phase 3 G: `![[Note#Heading]]` / `![[Note#^block]]` embed only the
            // referenced section / block. An anchor that can't be located degrades
            // gracefully to the whole note (the note itself resolved fine).
            if let Some(anchor) = wikilink_anchor(reference) {
                if let Some(sliced) = slice_by_anchor(&r.content, &anchor) {
                    r.content = sliced;
                }
            }
            Ok(Some(r))
        }
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

/// Extract the `#anchor` from a `[[ ]]` reference (alias dropped first), trimmed.
/// `None` when there is no anchor. A leading `^` marks a block anchor.
fn wikilink_anchor(reference: &str) -> Option<String> {
    let before_alias = reference
        .split_once('|')
        .map(|(t, _)| t)
        .unwrap_or(reference);
    before_alias
        .split_once('#')
        .map(|(_, a)| a.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Slice a note's content to a `#Heading` section or `#^block` (Phase 3 G).
/// Returns `None` when the anchor doesn't resolve (caller keeps whole content).
fn slice_by_anchor(content: &str, anchor: &str) -> Option<String> {
    let parsed = super::parser::parse_document(content);
    if let Some(block_id) = anchor.strip_prefix('^') {
        return parsed
            .blocks
            .iter()
            .find(|b| b.block_id.eq_ignore_ascii_case(block_id))
            .map(|b| b.text.clone());
    }
    // Heading section: from the matching heading to the next heading of the same
    // or higher level (or EOF). Match is NFC + case-insensitive (resolve parity).
    let target = super::parser::nfc(anchor).to_lowercase();
    let idx = parsed
        .headings
        .iter()
        .position(|h| super::parser::nfc(&h.title).to_lowercase() == target)?;
    let h = &parsed.headings[idx];
    let end = parsed.headings[idx + 1..]
        .iter()
        .find(|n| n.level <= h.level)
        .map(|n| n.byte_start)
        .unwrap_or(content.len());
    Some(content[h.byte_start..end].trim_end().to_string())
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

// ── Maintenance config (WS6, owner plane GUI) ───────────────────

/// Current (clamped) maintenance config for the GUI panel.
pub fn get_maintenance_config() -> super::maintenance::MaintenanceConfig {
    crate::config::cached_config()
        .knowledge_maintenance
        .clamped()
}

/// Persist maintenance config (clamped). Emits `config:changed`, which wakes the
/// cron loop to re-evaluate its schedule. Returns the clamped value saved.
pub fn set_maintenance_config(
    cfg: super::maintenance::MaintenanceConfig,
    source: &str,
) -> Result<super::maintenance::MaintenanceConfig> {
    let clamped = cfg.clamped();
    let to_save = clamped.clone();
    crate::config::mutate_config(("knowledge_maintenance", source), move |store| {
        store.knowledge_maintenance = to_save.clone();
        Ok(())
    })?;
    Ok(clamped)
}

// ── Passive recall config (read bridge ③, owner plane GUI) ──────

/// Current (clamped) passive related-notes config for the GUI panel.
pub fn get_passive_recall_config() -> super::PassiveRecallConfig {
    crate::config::cached_config()
        .knowledge_passive_recall
        .clamped()
}

/// Persist passive recall config (clamped). Returns the clamped value saved.
/// No reindex side effect — it only changes per-turn prompt injection.
pub fn set_passive_recall_config(
    cfg: super::PassiveRecallConfig,
    source: &str,
) -> Result<super::PassiveRecallConfig> {
    let clamped = cfg.clamped();
    let to_save = clamped.clone();
    crate::config::mutate_config(("knowledge_passive_recall", source), move |store| {
        store.knowledge_passive_recall = to_save.clone();
        Ok(())
    })?;
    Ok(clamped)
}

/// Strip a ```markdown … ``` wrapper the model may add around its reply. Only unwraps
/// when it's confidently a *markdown wrapper* of the whole reply: opening fence whose
/// info string is empty / `markdown` / `md`, + a closing ``` at the very end. A reply
/// that is a real code block in some language (```python …), one that merely *starts*
/// with a code block (content after the inner closing fence), or a single-line fence is
/// returned untouched — so a rewrite the user explicitly asked to be a fenced code
/// block isn't silently de-fenced.
fn strip_md_fence(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some((info, body)) = rest.split_once('\n') {
            let info = info.trim().to_ascii_lowercase();
            let is_md_wrapper = info.is_empty() || info == "markdown" || info == "md";
            if is_md_wrapper {
                if let Some(inner) = body.strip_suffix("```") {
                    return inner.trim().to_string();
                }
            }
        }
    }
    t.to_string()
}

/// AI-assisted note rewrite (WS9, owner plane): run an LLM rewrite of `text`
/// following `instruction` and return the rewritten Markdown. The GUI shows a diff
/// and the user confirms before saving — **this never touches disk**, so it needs
/// no `WorkspaceScope` / write gate (the eventual save goes through `note_save`).
/// Decoupled background call via the analysis agent (like recap / distill).
pub async fn ai_rewrite(text: &str, instruction: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("nothing to rewrite");
    }
    let instruction = instruction.trim();
    if instruction.is_empty() {
        bail!("provide a rewrite instruction");
    }
    let prompt = format!(
        "You are editing a Markdown note. Rewrite the TEXT below following the \
         INSTRUCTION. Preserve Markdown structure and any [[wikilinks]] / #tags \
         unless the instruction says otherwise. Return ONLY the rewritten Markdown — \
         no preamble, no commentary, no surrounding code fence.\n\n\
         INSTRUCTION:\n{instruction}\n\nTEXT:\n{text}"
    );
    // Output budget ~1 token/char + headroom (CJK ≈ 1 token/char, so don't halve),
    // saturating to avoid an overflow on a pathological length. Hard-capped at 8192
    // tokens — a very long whole-note rewrite can still hit the cap (rewrite a
    // selection instead), but that's an explicit ceiling, not silent under-budgeting.
    let max_tokens = u32::try_from(text.chars().count())
        .unwrap_or(u32::MAX)
        .saturating_add(512)
        .clamp(512, 8192);
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    let res = agent.side_query(&prompt, max_tokens).await?;
    let out = strip_md_fence(res.text.trim());
    if out.is_empty() {
        bail!("the model returned empty content");
    }
    Ok(out)
}

// ── First-run default knowledge space ───────────────────────────────────────

/// Seed one usable knowledge space on first run so a fresh install isn't empty.
///
/// Idempotent via a `<root>/.default-kb-seeded` sentinel: created **exactly once
/// ever**, so a user who later deletes it never gets it auto-recreated. Existing
/// installs that already have ≥1 KB just get the sentinel (no redundant space).
/// Best-effort — every failure logs and returns without panicking (boot must not
/// depend on it); a hard error before the sentinel is written simply retries next
/// launch. Called from `app_init` after the registry + index are ready.
pub fn ensure_default_knowledge_base() {
    let Ok(sentinel) = crate::paths::root_dir().map(|d| d.join(".default-kb-seeded")) else {
        return;
    };
    if sentinel.exists() {
        return;
    }
    let Ok(reg) = registry() else {
        return; // registry not ready — retry next boot (no sentinel written)
    };
    match reg.list_all_ids() {
        // Existing user already has spaces — mark seeded, don't add a redundant one.
        Ok(ids) if !ids.is_empty() => {
            let _ = std::fs::write(&sentinel, b"");
            return;
        }
        Ok(_) => {}
        Err(e) => {
            crate::app_warn!("knowledge", "seed", "count KBs failed: {}", e);
            return;
        }
    }

    let (name, emoji) = default_kb_label();
    let kb = match reg.create(CreateKnowledgeBaseInput {
        name,
        emoji: Some(emoji),
        root_dir: None, // internal — lazily materializes ~/.hope-agent/knowledge/{id}/notes/
    }) {
        Ok(kb) => kb,
        Err(e) => {
            crate::app_warn!("knowledge", "seed", "create default KB failed: {}", e);
            return;
        }
    };

    // Best-effort welcome note so the space isn't empty on first open + teaches
    // the basics. A failure here still leaves a usable (empty) space.
    let (rel, content) = welcome_note();
    if let Err(e) = note_save(&kb.id, &rel, &content, None, true) {
        crate::app_warn!("knowledge", "seed", "welcome note failed: {}", e);
    }

    let _ = std::fs::write(&sentinel, b"");
    crate::app_info!(
        "knowledge",
        "seed",
        "created default knowledge space {}",
        kb.id
    );
}

/// Resolve the UI locale for the first-run seed: `AppConfig.language` when set to
/// a concrete code, else sniff the OS env (`LANG` / `LC_ALL` / `LANGUAGE`) for the
/// `"auto"` default. Normalized to a base code (`zh` / `en` / …) with `zh-TW` kept
/// distinct for Traditional Chinese.
fn seed_locale() -> String {
    let cfg = crate::config::cached_config().language.clone();
    let raw = if cfg.is_empty() || cfg.eq_ignore_ascii_case("auto") {
        // Skip set-but-empty vars (e.g. LANG="") so they don't short-circuit the
        // chain and starve LC_ALL/LANGUAGE.
        ["LANG", "LC_ALL", "LANGUAGE"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|s| !s.is_empty()))
            .unwrap_or_default()
    } else {
        cfg
    };
    normalize_seed_locale(&raw)
}

/// Normalize a config/env locale string to a base code (`zh` / `en` / …), keeping
/// `zh-TW` distinct for Traditional Chinese. Empty/unknown → `en`. Pure.
fn normalize_seed_locale(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.starts_with("zh")
        && (lower.contains("tw") || lower.contains("hk") || lower.contains("hant"))
    {
        return "zh-TW".to_string();
    }
    let base = lower.split(['-', '_', '.', ':']).next().unwrap_or("");
    if base.is_empty() {
        "en".to_string()
    } else {
        base.to_string()
    }
}

/// Localized name + emoji for the default knowledge space (editable by the user).
fn default_kb_label() -> (String, String) {
    let name = match seed_locale().as_str() {
        "zh" => "我的笔记",
        "zh-TW" => "我的筆記",
        "ja" => "マイノート",
        "ko" => "내 노트",
        "es" => "Mis notas",
        "pt" => "Minhas notas",
        "ru" => "Мои заметки",
        "ar" => "ملاحظاتي",
        "tr" => "Notlarım",
        "vi" => "Ghi chú của tôi",
        "ms" => "Nota Saya",
        _ => "My Notes",
    };
    (name.to_string(), "📒".to_string())
}

/// Welcome note content for the seeded space (Chinese for zh / zh-TW, English
/// otherwise). File name stays ASCII-stable; the title inside is localized.
fn welcome_note() -> (String, String) {
    let loc = seed_locale();
    let content = if loc.starts_with("zh") {
        "# 欢迎使用知识空间\n\n\
         这是你的第一个知识空间。在这里用 Markdown 记笔记,并用 `[[双链]]` 把它们连接成网络。\n\n\
         ## 试试看\n\n\
         - 新建一篇笔记,记录一个想法\n\
         - 用 `[[笔记名]]` 引用另一篇笔记,自动建立链接\n\
         - 在对话输入框点知识空间图标,把这个空间挂载给助手——它就能搜索、阅读并帮你整理笔记\n\n\
         随时可以重命名或删除这个知识空间。\n"
    } else {
        "# Welcome to your knowledge space\n\n\
         This is your first knowledge space. Capture notes in Markdown and connect them \
         into a network with `[[wikilinks]]`.\n\n\
         ## Try it\n\n\
         - Create a note to jot down an idea\n\
         - Reference another note with `[[Note name]]` to auto-create a link\n\
         - Attach this space to a chat (the knowledge icon by the composer) so the \
         assistant can search, read, and organize your notes\n\n\
         You can rename or delete this space anytime.\n"
    };
    ("Welcome.md".to_string(), content.to_string())
}

#[cfg(test)]
mod seed_locale_tests {
    use super::normalize_seed_locale;

    #[test]
    fn normalizes_base_codes() {
        assert_eq!(normalize_seed_locale("en"), "en");
        assert_eq!(normalize_seed_locale("zh"), "zh");
        assert_eq!(normalize_seed_locale("zh_CN.UTF-8"), "zh");
        assert_eq!(normalize_seed_locale("en_US.UTF-8"), "en");
        assert_eq!(normalize_seed_locale("ja_JP"), "ja");
        assert_eq!(normalize_seed_locale("pt-BR"), "pt");
    }

    #[test]
    fn detects_traditional_chinese() {
        assert_eq!(normalize_seed_locale("zh-TW"), "zh-TW");
        assert_eq!(normalize_seed_locale("zh_TW.UTF-8"), "zh-TW");
        assert_eq!(normalize_seed_locale("zh_HK"), "zh-TW");
        assert_eq!(normalize_seed_locale("zh-Hant"), "zh-TW");
        // Simplified stays base zh.
        assert_eq!(normalize_seed_locale("zh-CN"), "zh");
    }

    #[test]
    fn empty_falls_back_to_en() {
        assert_eq!(normalize_seed_locale(""), "en");
    }
}

#[cfg(test)]
mod strip_md_fence_tests {
    use super::strip_md_fence;

    #[test]
    fn unwraps_markdown_wrapper() {
        assert_eq!(
            strip_md_fence("```markdown\n# Title\n\nbody\n```"),
            "# Title\n\nbody"
        );
        assert_eq!(strip_md_fence("```\n# Title\n```"), "# Title");
    }

    #[test]
    fn preserves_language_code_block() {
        // User asked to turn the selection into a code block — don't de-fence it.
        let py = "```python\nprint(\"hi\")\n```";
        assert_eq!(strip_md_fence(py), py);
    }

    #[test]
    fn preserves_content_that_only_starts_with_a_code_block() {
        let s = "```rust\nlet x = 1;\n```\n\nmore prose";
        assert_eq!(strip_md_fence(s), s);
    }

    #[test]
    fn leaves_plain_markdown_untouched() {
        assert_eq!(strip_md_fence("# Title\n\nbody"), "# Title\n\nbody");
    }
}

#[cfg(test)]
mod anchor_slice_tests {
    use super::{slice_by_anchor, wikilink_anchor};

    #[test]
    fn extracts_anchor_after_alias() {
        assert_eq!(wikilink_anchor("Note#Heading").as_deref(), Some("Heading"));
        assert_eq!(wikilink_anchor("Note#^blk").as_deref(), Some("^blk"));
        // Alias split happens first; `#` inside the alias is ignored.
        assert_eq!(
            wikilink_anchor("Note#Heading|Label#x").as_deref(),
            Some("Heading")
        );
        assert_eq!(wikilink_anchor("Note"), None);
        assert_eq!(wikilink_anchor("Note|alias"), None);
    }

    #[test]
    fn slices_heading_section_to_next_same_level() {
        let md = "# Top\n\nintro\n\n## A\n\nalpha body\n\n### A.1\n\nsub\n\n## B\n\nbeta\n";
        let sliced = slice_by_anchor(md, "A").unwrap();
        assert!(sliced.starts_with("## A"));
        assert!(sliced.contains("alpha body"));
        assert!(sliced.contains("### A.1")); // deeper heading stays in the section
        assert!(!sliced.contains("## B")); // stops at the next same-level heading
    }

    #[test]
    fn slices_block_by_id() {
        let md = "# T\n\nFirst paragraph. ^p1\n\nSecond. ^p2\n";
        assert_eq!(
            slice_by_anchor(md, "^p1").as_deref(),
            Some("First paragraph.")
        );
        assert_eq!(slice_by_anchor(md, "^p2").as_deref(), Some("Second."));
    }

    #[test]
    fn unresolved_anchor_returns_none() {
        let md = "# T\n\nbody ^known\n";
        assert_eq!(slice_by_anchor(md, "Missing Heading"), None);
        assert_eq!(slice_by_anchor(md, "^missing"), None);
    }
}
