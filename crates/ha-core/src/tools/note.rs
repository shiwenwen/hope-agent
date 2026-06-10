//! `note_*` knowledge-base tools (design AI Knowledge Operations, Layer 1).
//!
//! Every tool routes its `kb` through [`effective_kb_access`] (D10), writes are
//! confined to `WorkspaceScope::for_knowledge` (external roots rejected, D11),
//! and stale-write guards re-hash the **disk file** (never the index cache) per
//! the v4.6 contract. Mutations re-index synchronously and emit
//! `knowledge:changed`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::ToolExecContext;
use crate::filesystem::{self, WorkspaceScope};
use crate::knowledge::{
    self, effective_kb_access, index, search, KbAccess, KbAccessSource, KnowledgeAccessContext,
};

// ── Access helpers ──────────────────────────────────────────────

fn access_map(ctx: &ToolExecContext) -> HashMap<String, KbAccess> {
    let mut source = ctx.chat_source.unwrap_or(KbAccessSource::Gui);
    // Call-chain origin (D10): a subagent carries its parent turn's origin so an
    // IM-origin chain can't reacquire KB access via the neutral Subagent source.
    // Falls back to `source` when unset (contexts not built by the chat engine).
    let mut origin = ctx.origin_chat_source.unwrap_or(source);
    let mut channel_info = ctx.channel_kb_context.clone();

    // Defense in depth (WS8): a context whose `chat_source` was never threaded
    // (`None` → would default to `Gui` owner) must NOT bypass the IM opt-in gate
    // when it is actually running on an IM-bound session. The `/skill` direct-tool
    // path (and any other entry that builds a `..Default::default()` context)
    // reaches `note_*` without setting a source; on an IM session that would
    // silently elevate to owner-plane KB access. Reclassify it as an IM turn
    // carrying the session's own channel identity so `effective_kb_access` applies
    // the same WS8 gate as the sanctioned inbound-LLM path. A non-IM session has
    // no `channel_info`, so desktop/HTTP owner contexts are unaffected.
    if ctx.chat_source.is_none() {
        if let Some(ci) = im_kb_context_from_session(ctx.session_id.as_deref()) {
            source = KbAccessSource::Im;
            origin = KbAccessSource::Im;
            channel_info = Some(ci);
        }
    }

    let actx = KnowledgeAccessContext::resolve(
        ctx.session_id.clone(),
        ctx.project_id.clone(),
        source,
        origin,
        channel_info,
    );
    effective_kb_access(&actx)
}

/// Build a [`ChannelKbContext`] from a session's persisted IM binding, or `None`
/// if the session is not IM-bound. Used by [`access_map`]'s defense-in-depth
/// reclassification (WS8) so an un-threaded tool context can't launder owner KB
/// access on an IM session.
pub(crate) fn im_kb_context_from_session(
    session_id: Option<&str>,
) -> Option<crate::knowledge::ChannelKbContext> {
    let ci = crate::session::lookup_session_meta(session_id)?.channel_info?;
    Some(crate::knowledge::ChannelKbContext {
        channel_id: ci.channel_id,
        account_id: ci.account_id,
        chat_id: ci.chat_id,
        // Any non-DM chat needs per-chat confirmation (matches the dispatcher's
        // live `is_group` derivation).
        is_group: !ci.chat_type.eq_ignore_ascii_case("dm"),
    })
}

fn require_write(ctx: &ToolExecContext, kb_id: &str) -> Result<()> {
    match access_map(ctx).get(kb_id) {
        Some(KbAccess::Write) => Ok(()),
        Some(KbAccess::Read) => bail!(
            "knowledge base '{}' is read-only for this session (grant write access, or for an external vault enable editing in the space settings)",
            kb_id
        ),
        None => bail!(
            "no write access to knowledge base '{}' — attach it to this session/project with write access first",
            kb_id
        ),
    }
}

fn require_read(ctx: &ToolExecContext, kb_id: &str) -> Result<()> {
    if access_map(ctx).contains_key(kb_id) {
        Ok(())
    } else {
        bail!("no access to knowledge base '{}'", kb_id)
    }
}

fn accessible_kbs(ctx: &ToolExecContext) -> Vec<String> {
    let mut v: Vec<String> = access_map(ctx).into_keys().collect();
    v.sort();
    v
}

/// Whether this tool context can reach ANY knowledge base (its agent-plane
/// `effective_kb_access` set is non-empty). Used by `tool_search` to hide the
/// KB-scoped tools on a no-KB session, mirroring the eager-schema gate in
/// `Agent::build_tool_schemas` (so a tool hidden from the eager schema can't be
/// resurrected via `tool_search`). Same access set the tools themselves enforce.
pub(crate) fn session_has_kb_access(ctx: &ToolExecContext) -> bool {
    !access_map(ctx).is_empty()
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn nested_str<'a>(args: &'a Value, parent: &str, key: &str) -> Option<&'a str> {
    args.get(parent)
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Normalize a user-supplied note path to a `/`-separated `.md` rel-path.
fn norm_note_path(raw: &str) -> String {
    let p = raw.trim().trim_start_matches('/').replace('\\', "/");
    let lower = p.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        p
    } else {
        format!("{p}.md")
    }
}

// ── Scope / IO helpers ──────────────────────────────────────────

fn writable_scope(kb_id: &str) -> Result<WorkspaceScope> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    Ok(scope)
}

fn read_scope(kb_id: &str) -> Result<WorkspaceScope> {
    WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))
}

/// Read the raw bytes of a note file under the scope (containment-checked).
fn read_note_raw(scope: &WorkspaceScope, rel_path: &str) -> Result<Vec<u8>> {
    let abs = scope
        .resolve_existing(rel_path)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    Ok(std::fs::read(&abs)?)
}

/// Stale-write guard: compare `expected` against the **current disk** raw BLAKE3
/// (never the index cache), per the v4.6 contract. Returns the current bytes.
/// Emit a `file_change` diff metadata payload for a note edit so the chat tool
/// card renders an inline before/after diff (same shape `edit` / `apply_patch`
/// emit). Notes are always markdown. No-op when no metadata sink is wired.
async fn emit_note_diff(ctx: &ToolExecContext, rel_path: &str, before: &str, after: &str) {
    if ctx.metadata_sink.is_none() {
        return;
    }
    let (added, removed) = super::diff_util::compute_line_delta(before, after);
    let (before_t, before_trunc) = super::diff_util::truncate_for_metadata(before);
    let (after_t, after_trunc) = super::diff_util::truncate_for_metadata(after);
    ctx.emit_metadata(serde_json::json!({
        "kind": "file_change",
        "path": rel_path,
        "action": "edit",
        "linesAdded": added,
        "linesRemoved": removed,
        "before": before_t,
        "after": after_t,
        "language": "markdown",
        "truncated": before_trunc || after_trunc,
    }))
    .await;
}

fn guard_stale(scope: &WorkspaceScope, rel_path: &str, expected: Option<&str>) -> Result<Vec<u8>> {
    let bytes = read_note_raw(scope, rel_path).unwrap_or_default();
    if let Some(expected) = expected {
        let current = knowledge::blake3_hex(&bytes);
        if current != expected {
            bail!(
                "stale write: note '{}' changed on disk since you read it (expected_file_hash mismatch). \
                 Current hash is {}. Re-read the note and retry.",
                rel_path,
                current
            );
        }
    }
    Ok(bytes)
}

/// Persist content + reindex + emit event. `create_only` rejects overwrite.
fn write_and_index(
    scope: &WorkspaceScope,
    kb_id: &str,
    rel_path: &str,
    content: &str,
    create_only: bool,
) -> Result<String> {
    filesystem::project_write_text(scope, rel_path, content, create_only)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    let hash = knowledge::blake3_hex(content.as_bytes());
    if let Err(e) = index::reindex_note(kb_id, scope.root(), rel_path) {
        crate::app_warn!(
            "knowledge",
            "tool",
            "reindex {} after write failed: {}",
            rel_path,
            e
        );
    }
    emit_changed(kb_id, "upsert");
    Ok(hash)
}

fn emit_changed(kb_id: &str, op: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "knowledge:changed",
            serde_json::json!({ "kbId": kb_id, "op": op }),
        );
    }
}

/// Resolve a note reference (path or title) to `(kb_id, rel_path)` over the
/// accessible KB set; returns a disambiguation error on cross-KB ties.
fn resolve_target(
    ctx: &ToolExecContext,
    kb_opt: Option<&str>,
    reference: &str,
) -> Result<(String, String)> {
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let kbs: Vec<String> = match kb_opt {
        Some(kb) => {
            require_read(ctx, kb)?;
            vec![kb.to_string()]
        }
        None => accessible_kbs(ctx),
    };
    if kbs.is_empty() {
        bail!("no accessible knowledge bases for this session");
    }
    let mut matches: Vec<(String, String)> = Vec::new();
    for kb in &kbs {
        let notes = db.note_refs(kb)?;
        if let Some(id) = knowledge::resolver::resolve(reference, &notes) {
            if let Some(n) = notes.iter().find(|n| n.id == id) {
                matches.push((kb.clone(), n.rel_path.clone()));
            }
        }
    }
    match matches.len() {
        0 => bail!("note not found: '{}'", reference),
        1 => Ok(matches.remove(0)),
        _ => {
            let list = matches
                .iter()
                .map(|(k, p)| format!("{k}:{p}"))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "ambiguous note '{}' — matches in multiple knowledge bases: {}",
                reference,
                list
            )
        }
    }
}

// ── Tools: CRUD ─────────────────────────────────────────────────

/// `note_create({kb, path, title?, content?, frontmatter?, template?})`
pub(crate) async fn tool_note_create(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;

    let title = str_arg(args, "title");
    let body = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

    // Compose file: optional frontmatter (title) + optional H1 + body.
    let mut content = String::new();
    if let Some(fm) = args.get("frontmatter").and_then(|v| v.as_object()) {
        content.push_str("---\n");
        for (k, v) in fm {
            let val = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            content.push_str(&format!("{k}: {val}\n"));
        }
        content.push_str("---\n\n");
    } else if let Some(t) = title {
        content.push_str(&format!("# {t}\n\n"));
    }
    content.push_str(body);
    if !content.ends_with('\n') {
        content.push('\n');
    }

    let hash = write_and_index(&scope, kb, &rel, &content, true)?;
    Ok(format!(
        "Created note '{rel}' in knowledge base '{kb}' (file_hash: {hash})"
    ))
}

/// `note_read({kb?, path|title, include?})`
pub(crate) async fn tool_note_read(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb_opt = str_arg(args, "kb");
    let reference = str_arg(args, "path")
        .or_else(|| str_arg(args, "title"))
        .or_else(|| str_arg(args, "note"))
        .ok_or_else(|| anyhow!("Missing 'path' or 'title' parameter"))?;
    let (kb_id, rel) = resolve_target(ctx, kb_opt, reference)?;

    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let scope = read_scope(&kb_id)?;
    let bytes = read_note_raw(&scope, &rel)?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    let content_hash = knowledge::blake3_hex(&bytes);

    let note = db
        .get_note_by_rel_path(&kb_id, &rel)?
        .ok_or_else(|| anyhow!("note not indexed yet: {}", rel))?;
    let outgoing = db.outgoing_links(note.id)?;
    let backlinks = db.backlinks(note.id)?;
    let tags = db.tags_for_note(note.id)?;

    let result = knowledge::NoteReadResult {
        kb_id: kb_id.clone(),
        note_id: note.id,
        rel_path: rel,
        title: note.title,
        content,
        content_hash,
        frontmatter_json: note.frontmatter_json,
        outgoing_links: outgoing,
        backlinks,
        tags,
    };
    Ok(serde_json::to_string_pretty(&result)?)
}

/// `note_update({kb, path, content, expected_file_hash?})` — full replace.
pub(crate) async fn tool_note_update(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'content' parameter"))?;
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;
    let before_bytes = guard_stale(&scope, &rel, str_arg(args, "expected_file_hash"))?;
    let before = String::from_utf8_lossy(&before_bytes).to_string();

    let hash = write_and_index(&scope, kb, &rel, content, false)?;
    emit_note_diff(ctx, &rel, &before, content).await;
    Ok(format!("Updated note '{rel}' (file_hash: {hash})"))
}

/// `note_patch({kb, path, old, new, expected_file_hash?})` — unique-match edit.
pub(crate) async fn tool_note_patch(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let old = args
        .get("old")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'old' parameter"))?;
    let new = args
        .get("new")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'new' parameter"))?;
    if old.is_empty() {
        bail!("'old' must not be empty");
    }
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;
    let bytes = guard_stale(&scope, &rel, str_arg(args, "expected_file_hash"))?;
    let content = String::from_utf8_lossy(&bytes).to_string();

    let occurrences = content.matches(old).count();
    match occurrences {
        0 => bail!("'old' text not found in note '{}'", rel),
        1 => {}
        n => bail!(
            "'old' matched {} times in note '{}' — make it uniquely identifying (include surrounding context). Refusing to silently replace the first match.",
            n,
            rel
        ),
    }
    let updated = content.replacen(old, new, 1);
    let hash = write_and_index(&scope, kb, &rel, &updated, false)?;
    emit_note_diff(ctx, &rel, &content, &updated).await;
    Ok(format!("Patched note '{rel}' (file_hash: {hash})"))
}

/// `note_append({kb, path, content, section?, expected_file_hash?})`
pub(crate) async fn tool_note_append(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let add = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'content' parameter"))?;
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;
    let bytes = guard_stale(&scope, &rel, str_arg(args, "expected_file_hash"))?;
    let mut content = String::from_utf8_lossy(&bytes).to_string();

    if let Some(section) = str_arg(args, "section") {
        content = append_under_heading(&content, section, add);
    } else {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(add);
        if !content.ends_with('\n') {
            content.push('\n');
        }
    }

    let hash = write_and_index(&scope, kb, &rel, &content, false)?;
    Ok(format!("Appended to note '{rel}' (file_hash: {hash})"))
}

/// `note_delete({kb, path, expected_file_hash?})`
pub(crate) async fn tool_note_delete(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;
    guard_stale(&scope, &rel, str_arg(args, "expected_file_hash"))?;

    filesystem::project_delete(&scope, &rel, false)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    if let Err(e) = index::remove_note(kb, &rel) {
        crate::app_warn!("knowledge", "tool", "remove index {} failed: {}", rel, e);
    }
    emit_changed(kb, "delete");
    Ok(format!(
        "Deleted note '{rel}' from knowledge base '{kb}' (links to it are now broken)"
    ))
}

// ── Tools: links / graph ────────────────────────────────────────

/// `note_link({from:{kb,path}, to:{kb,path}, alias?, section?, expected_file_hash?})`
pub(crate) async fn tool_note_link(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let from_kb = nested_str(args, "from", "kb").ok_or_else(|| anyhow!("Missing 'from.kb'"))?;
    let from_path =
        nested_str(args, "from", "path").ok_or_else(|| anyhow!("Missing 'from.path'"))?;
    let to_kb = nested_str(args, "to", "kb").ok_or_else(|| anyhow!("Missing 'to.kb'"))?;
    let to_path = nested_str(args, "to", "path").ok_or_else(|| anyhow!("Missing 'to.path'"))?;

    // Wikilinks have no cross-KB concept (Phase 1).
    if from_kb != to_kb {
        bail!("note_link requires from.kb == to.kb (wikilinks are within a single knowledge base in Phase 1)");
    }
    require_write(ctx, from_kb)?;

    let from_rel = norm_note_path(from_path);
    let to_rel = norm_note_path(to_path);
    let scope = writable_scope(from_kb)?;
    let bytes = guard_stale(&scope, &from_rel, str_arg(args, "expected_file_hash"))?;
    let mut content = String::from_utf8_lossy(&bytes).to_string();

    // Build the wikilink target: prefer a basename so it stays stable.
    let target = to_rel
        .strip_suffix(".md")
        .unwrap_or(&to_rel)
        .rsplit('/')
        .next()
        .unwrap_or(&to_rel)
        .to_string();
    let alias = str_arg(args, "alias");
    let link = match alias {
        Some(a) => format!("[[{target}|{a}]]"),
        None => format!("[[{target}]]"),
    };
    let section = str_arg(args, "section").unwrap_or("Related");
    content = append_under_heading(&content, section, &format!("- {link}"));

    let hash = write_and_index(&scope, from_kb, &from_rel, &content, false)?;
    Ok(format!(
        "Linked '{from_rel}' → {link} under '{section}' (file_hash: {hash})"
    ))
}

/// `note_backlinks({kb?, note, block?})` — note-level backlinks, or (with
/// `block`) only `[[Note#^block]]` references pointing at that block (Phase 3 G).
pub(crate) async fn tool_note_backlinks(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let reference = str_arg(args, "note")
        .or_else(|| str_arg(args, "path"))
        .or_else(|| str_arg(args, "title"))
        .ok_or_else(|| anyhow!("Missing 'note' parameter"))?;
    let (kb_id, rel) = resolve_target(ctx, str_arg(args, "kb"), reference)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let note = db
        .get_note_by_rel_path(&kb_id, &rel)?
        .ok_or_else(|| anyhow!("note not indexed: {}", rel))?;
    // A `block` of just `^` / whitespace collapses to an empty id — reject it
    // rather than silently querying for a meaningless anchor.
    let block = str_arg(args, "block").map(|b| b.trim_start_matches('^').trim().to_string());
    if matches!(block.as_deref(), Some("")) {
        bail!("'block' must be a non-empty block id (e.g. '^my-id' or 'my-id')");
    }
    let backlinks = match &block {
        Some(b) => db.block_backlinks(note.id, b)?,
        None => db.backlinks(note.id)?,
    };
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb_id,
        "note": rel,
        "block": block,
        "count": backlinks.len(),
        "backlinks": backlinks,
    }))?)
}

/// `note_rename` / `note_move` ({kb, from, to, expected_file_hash?}) — move or
/// rename a note and rewrite inbound `[[ ]]` links in other notes (#9).
pub(crate) async fn tool_note_rename(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let from = str_arg(args, "from")
        .or_else(|| str_arg(args, "path"))
        .ok_or_else(|| anyhow!("Missing 'from' parameter"))?;
    let to = str_arg(args, "to")
        .or_else(|| str_arg(args, "new_path"))
        .ok_or_else(|| anyhow!("Missing 'to' parameter"))?;
    let from_rel = norm_note_path(from);
    let outcome = knowledge::rename_note(kb, &from_rel, to, str_arg(args, "expected_file_hash"))?;
    Ok(format!(
        "Renamed '{}' → '{}' in knowledge base '{}' ({} inbound link(s) rewritten across {} note(s)).",
        from_rel, outcome.new_rel, kb, outcome.links_rewritten, outcome.files_changed
    ))
}

/// `note_set_frontmatter({kb, path, props, expected_file_hash?})` — merge YAML
/// frontmatter props (null value removes a key).
pub(crate) async fn tool_note_set_frontmatter(
    args: &Value,
    ctx: &ToolExecContext,
) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let props = args
        .get("props")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("Missing 'props' object parameter"))?;
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;
    let bytes = read_note_raw(&scope, &rel)?; // errors if the note is missing
    if let Some(expected) = str_arg(args, "expected_file_hash") {
        let current = knowledge::blake3_hex(&bytes);
        if current != expected {
            bail!(
                "stale write: note '{}' changed on disk (expected_file_hash mismatch, current {}). Re-read and retry.",
                rel,
                current
            );
        }
    }
    let content = String::from_utf8_lossy(&bytes).to_string();
    let updated = knowledge::parser::merge_frontmatter(&content, props);
    let hash = write_and_index(&scope, kb, &rel, &updated, false)?;
    Ok(format!(
        "Updated frontmatter of note '{rel}' (file_hash: {hash})"
    ))
}

/// Build a `[[inner#^id]]` block reference string.
fn fmt_block_ref(inner: &str, id: &str) -> String {
    format!("[[{inner}#^{id}]]")
}

/// A block reference that the resolver maps **unambiguously back to this note**:
/// the basename when it resolves uniquely to `rel` (Obsidian's preferred short
/// form), otherwise the path form (`folder/Note`, always exact). Falls back to
/// the path form if the index is unavailable. Fixes the basename-collision case
/// where `a/Note.md` + `b/Note.md` would make `[[Note#^id]]` resolve to the wrong
/// note (resolver tie-break: shortest path then lexicographic).
fn stable_block_ref(kb_id: &str, rel: &str, id: &str) -> String {
    let stem = rel.strip_suffix(".md").unwrap_or(rel);
    let base = stem.rsplit('/').next().unwrap_or(stem);
    if let Some(db) = index::get_index_db() {
        if let Ok(notes) = db.note_refs(kb_id) {
            if let Some(me) = notes.iter().find(|n| n.rel_path == rel) {
                if knowledge::resolver::resolve(base, &notes) == Some(me.id) {
                    return fmt_block_ref(base, id);
                }
            }
        }
    }
    fmt_block_ref(stem, id)
}

/// Derive a short, collision-free block id from the block text (deterministic —
/// no RNG, so a re-run on the same content is stable).
fn gen_block_id(seed: &str, existing: &std::collections::HashSet<String>) -> String {
    let h = knowledge::blake3_hex(seed.trim().as_bytes());
    for len in [6usize, 8, 12, 16] {
        let cand = &h[..len];
        if !existing.contains(cand) {
            return cand.to_string();
        }
    }
    let mut n = 0;
    loop {
        let cand = format!("{}-{n}", &h[..6]);
        if !existing.contains(&cand) {
            return cand;
        }
        n += 1;
    }
}

/// Every `^block-id` physically present in the file (raw line scan, so it catches
/// anchors even when their block span didn't resolve — a stricter collision set
/// than `parse_document().blocks`).
fn collect_block_ids(content: &str) -> std::collections::HashSet<String> {
    content
        .lines()
        .filter_map(knowledge::parser::line_block_anchor)
        .collect()
}

/// Where a new `^id` should be spliced for `block_text` — or the id the target
/// block already carries (idempotent). Resolves the **whole leaf block** (not
/// just the matched line) so a multi-line paragraph, or an existing `^id` sitting
/// on its own line below the block, is handled correctly; frontmatter / fenced
/// code matches are rejected (they can't carry a real block anchor).
struct AnchorPlacement {
    existing_id: Option<String>,
    /// Byte offset to insert ` ^id` (the trimmed end of the block's last line).
    insert_at: usize,
}

fn resolve_anchor_placement(content: &str, block_text: &str, rel: &str) -> Result<AnchorPlacement> {
    match content.matches(block_text).count() {
        0 => bail!("'block_text' not found in note '{}'", rel),
        1 => {}
        n => bail!(
            "'block_text' matched {} times in note '{}' — include surrounding context so it uniquely identifies one block.",
            n,
            rel
        ),
    }
    let match_start = content.find(block_text).unwrap();
    let match_end = match_start + block_text.len();

    // Frontmatter is metadata, not a referenceable block.
    if match_start < knowledge::parser::parse_document(content).body_start_byte {
        bail!("'block_text' matches inside the note's frontmatter, not a referenceable block");
    }

    struct Line {
        start: usize,
        end: usize, // trimmed content end (drops trailing whitespace + CR/LF)
        full_end: usize,
        in_code: bool,
    }
    let mut lines: Vec<Line> = Vec::new();
    let mut pos = 0usize;
    let mut fence: Option<(u8, usize)> = None;
    for raw in content.split_inclusive('\n') {
        let start = pos;
        pos += raw.len();
        let body = raw.trim_end_matches(['\r', '\n']);
        let trimmed = body.trim_start();
        let indent = body.len() - trimmed.len();
        let mut delim = false;
        if indent <= 3 {
            if let Some(&c0) = trimmed.as_bytes().first() {
                if c0 == b'`' || c0 == b'~' {
                    let flen = trimmed.bytes().take_while(|&c| c == c0).count();
                    if flen >= 3 {
                        let rest = &trimmed[flen..];
                        match fence {
                            None => {
                                fence = Some((c0, flen));
                                delim = true;
                            }
                            Some((fc, fl)) => {
                                if c0 == fc && flen >= fl && rest.trim().is_empty() {
                                    fence = None;
                                    delim = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        lines.push(Line {
            start,
            end: start + body.trim_end().len(),
            full_end: pos,
            in_code: fence.is_some() || delim,
        });
    }

    let anchor_byte = match_end.saturating_sub(1).max(match_start);
    let li = lines
        .iter()
        .position(|l| anchor_byte >= l.start && anchor_byte < l.full_end)
        .unwrap_or(lines.len().saturating_sub(1));
    if lines[li].in_code {
        bail!("'block_text' matches inside a code block — code spans can't carry a block id");
    }

    let line_str = |l: &Line| &content[l.start..l.end];
    let is_blank = |l: &Line| line_str(l).trim().is_empty();
    let is_heading = |s: &str| s.trim_start().starts_with('#');
    let is_list = |s: &str| {
        let t = s.trim_start();
        if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
            return true;
        }
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        digits > 0 && t[digits..].starts_with(['.', ')']) && t[digits + 1..].starts_with(' ')
    };

    let cur = line_str(&lines[li]).to_string();
    let mut last = li;
    let mut found = knowledge::parser::line_block_anchor(&cur);
    // Only extend through a *paragraph* (list items / headings are single-line
    // blocks). Stop at a blank line, a new block, an own-line `^id`, or a
    // continuation line that already carries a trailing anchor.
    if found.is_none() && !is_list(&cur) && !is_heading(&cur) {
        while last + 1 < lines.len() {
            let nxt = &lines[last + 1];
            let nxt_s = line_str(nxt);
            if nxt.in_code || is_blank(nxt) || is_heading(nxt_s) || is_list(nxt_s) {
                break;
            }
            if nxt_s.trim_start().starts_with('^') {
                found = knowledge::parser::line_block_anchor(nxt_s);
                break;
            }
            last += 1;
            if let Some(id) = knowledge::parser::line_block_anchor(nxt_s) {
                found = Some(id);
                break;
            }
        }
    }

    Ok(AnchorPlacement {
        existing_id: found,
        insert_at: lines[last].end,
    })
}

/// `note_assign_block({kb, path, block_text, block_id?, expected_file_hash?})`
/// — assign an Obsidian `^block-id` to a target block so it can be referenced
/// with `[[Note#^id]]` / `![[Note#^id]]` (Phase 3 G). `block_text` must uniquely
/// identify the block (mirrors `note_patch`'s unique-match rule). Idempotent: if
/// the matched block already has an id, returns it. Generates a stable id when
/// `block_id` is omitted. Gated by the same three guards as every KB write:
/// writable scope (external read-only / WS7 opt-in), stale-write, and unique
/// match.
pub(crate) async fn tool_note_assign_block(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let path = str_arg(args, "path").ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
    let block_text = str_arg(args, "block_text")
        .or_else(|| str_arg(args, "text"))
        .ok_or_else(|| {
            anyhow!("Missing 'block_text' parameter (a unique snippet of the target block)")
        })?;
    let rel = norm_note_path(path);
    let scope = writable_scope(kb)?;
    let bytes = guard_stale(&scope, &rel, str_arg(args, "expected_file_hash"))?;
    let content = String::from_utf8_lossy(&bytes).to_string();

    // Locate the whole leaf block the anchor attaches to (handles multi-line
    // paragraphs / own-line existing anchors; rejects frontmatter / code).
    let placement = resolve_anchor_placement(&content, block_text, &rel)?;
    if let Some(existing) = placement.existing_id {
        return Ok(format!(
            "Block already has id '^{existing}' in note '{rel}' — reference it as {}",
            stable_block_ref(kb, &rel, &existing)
        ));
    }

    let existing_ids = collect_block_ids(&content);
    let id = match str_arg(args, "block_id") {
        Some(raw) => {
            let raw = raw.trim_start_matches('^');
            if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                bail!("'block_id' must be non-empty and contain only letters, digits, and dashes");
            }
            if existing_ids.contains(raw) {
                bail!("block id '^{raw}' already exists in note '{rel}' — choose another");
            }
            raw.to_string()
        }
        None => gen_block_id(block_text, &existing_ids),
    };

    // Append ` ^id` at the trimmed end of the block's last line (always a single
    // space before the caret; never crossing into another block).
    let insert_at = placement.insert_at;
    let mut updated = String::with_capacity(content.len() + id.len() + 2);
    updated.push_str(&content[..insert_at]);
    updated.push_str(&format!(" ^{id}"));
    updated.push_str(&content[insert_at..]);

    let hash = write_and_index(&scope, kb, &rel, &updated, false)?;
    Ok(format!(
        "Assigned block id '^{id}' in note '{rel}' (file_hash: {hash}). Reference it as {}",
        stable_block_ref(kb, &rel, &id)
    ))
}

/// `note_broken_links({kb})` — dangling `[[ ]]` links to create or fix.
pub(crate) async fn tool_note_broken_links(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_read(ctx, kb)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let broken = db.list_broken_links(kb)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb,
        "count": broken.len(),
        "brokenLinks": broken,
    }))?)
}

/// `note_orphans({kb})` — notes with no resolved inbound or outbound link.
pub(crate) async fn tool_note_orphans(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_read(ctx, kb)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let orphans = db.list_orphan_notes(kb)?;
    let rows: Vec<_> = orphans
        .iter()
        .map(|n| serde_json::json!({ "path": n.rel_path, "title": n.title }))
        .collect();
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb,
        "count": rows.len(),
        "orphans": rows,
    }))?)
}

/// `note_graph({kb?, note?, depth?})` — the resolved link graph (nodes = notes,
/// edges = resolved `[[ ]]`/`![[ ]]`). With `note`: the ego neighbourhood
/// (default depth 1, max 3). Without it: the whole-KB graph, capped. `kb` is
/// required unless a `note` pins it down or exactly one KB is accessible.
pub(crate) async fn tool_note_graph(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let kb_opt = str_arg(args, "kb");
    let note_ref = str_arg(args, "note")
        .or_else(|| str_arg(args, "path"))
        .or_else(|| str_arg(args, "title"));

    // Resolve which KB the graph is for (and check access).
    let kb_id = match (kb_opt, note_ref) {
        (Some(kb), _) => {
            require_read(ctx, kb)?;
            kb.to_string()
        }
        (None, Some(reference)) => resolve_target(ctx, None, reference)?.0,
        (None, None) => {
            let kbs = accessible_kbs(ctx);
            match kbs.len() {
                0 => bail!("no accessible knowledge bases for this session"),
                1 => kbs.into_iter().next().expect("len checked == 1"),
                _ => bail!("multiple knowledge bases accessible — pass 'kb' to choose one"),
            }
        }
    };

    let full = knowledge::graph::build_kb_graph(&db, &kb_id)?;
    let graph = match note_ref {
        Some(reference) => {
            let notes = db.note_refs(&kb_id)?;
            let Some(center) = knowledge::resolver::resolve(reference, &notes) else {
                bail!("note not found: '{}'", reference);
            };
            let depth = args
                .get("depth")
                .and_then(|v| v.as_u64())
                .map(|d| d.clamp(1, 3) as usize)
                .unwrap_or(1);
            // Cap the ego neighbourhood too — a hub note at depth 3 can pull in a
            // whole connected component; the cap bounds the tool output + sets
            // `truncated` so the model isn't told an oversized result is complete.
            knowledge::graph::cap_nodes(knowledge::graph::ego_subgraph(&full, center, depth), 200)
        }
        None => knowledge::graph::cap_nodes(full, 200),
    };

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb_id,
        "nodeCount": graph.nodes.len(),
        "edgeCount": graph.edges.len(),
        "truncated": graph.truncated,
        "nodes": graph.nodes,
        "edges": graph.edges,
    }))?)
}

// ── Tools: smart retrieval (WS4) ────────────────────────────────

/// Resolve a note reference to `(kb_id, note, raw_content)` over the accessible
/// set — the shared front-half of the retrieval/AI tools.
fn read_resolved_note(
    ctx: &ToolExecContext,
    kb_opt: Option<&str>,
    reference: &str,
) -> Result<(String, knowledge::Note, String)> {
    let (kb_id, rel) = resolve_target(ctx, kb_opt, reference)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let note = db
        .get_note_by_rel_path(&kb_id, &rel)?
        .ok_or_else(|| anyhow!("note not indexed yet: {}", rel))?;
    let scope = read_scope(&kb_id)?;
    let bytes = read_note_raw(&scope, &rel)?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    Ok((kb_id, note, content))
}

/// `note_similar({kb?, note, k?})` — vector nearest-neighbour notes.
pub(crate) async fn tool_note_similar(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let reference = str_arg(args, "note")
        .or_else(|| str_arg(args, "path"))
        .or_else(|| str_arg(args, "title"))
        .ok_or_else(|| anyhow!("Missing 'note' parameter"))?;
    let k = args
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 25) as usize)
        .unwrap_or(8);
    let (kb_id, note, content) = read_resolved_note(ctx, str_arg(args, "kb"), reference)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let source_text = format!("{}\n\n{}", note.title, crate::truncate_utf8(&content, 8000));
    let hits = search::similar_notes(&db, std::slice::from_ref(&kb_id), note.id, &source_text, k)?;
    if hits.is_empty() {
        // Distinguish "vectors off" from "vectors on, no neighbours" so we don't
        // tell the user to enable an already-enabled model.
        let vectors_on =
            db.embedder().is_some() && knowledge::knowledge_active_embedding_signature().is_some();
        return Ok(if vectors_on {
            "No similar notes found.".to_string()
        } else {
            "No similar notes found. (note_similar needs vector search — enable a knowledge embedding model in Settings → Knowledge Space.)".to_string()
        });
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb_id,
        "note": note.rel_path,
        "count": hits.len(),
        "similar": hits,
    }))?)
}

/// `note_related({kb?, note})` — fused recall: backlinks ∪ resolved out-links ∪
/// vector neighbours ∪ shared tags, ranked by how many channels agree.
pub(crate) async fn tool_note_related(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let reference = str_arg(args, "note")
        .or_else(|| str_arg(args, "path"))
        .or_else(|| str_arg(args, "title"))
        .ok_or_else(|| anyhow!("Missing 'note' parameter"))?;
    let (kb_id, note, content) = read_resolved_note(ctx, str_arg(args, "kb"), reference)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;

    struct Related {
        rel_path: String,
        title: String,
        reasons: BTreeSet<String>,
        score: f32,
    }
    let mut map: HashMap<i64, Related> = HashMap::new();
    let mut bump = |id: i64, rel: &str, title: &str, reason: &str, score: f32| {
        if id == note.id {
            return;
        }
        let e = map.entry(id).or_insert_with(|| Related {
            rel_path: rel.to_string(),
            title: title.to_string(),
            reasons: BTreeSet::new(),
            score: 0.0,
        });
        e.reasons.insert(reason.to_string());
        e.score += score;
    };

    // Backlinks (who links to this note). Dedupe by source note — a source that
    // links here multiple times still counts as one backlink (no score inflation).
    let mut seen_back: HashSet<i64> = HashSet::new();
    for b in db.backlinks(note.id)? {
        if seen_back.insert(b.src_note_id) {
            bump(
                b.src_note_id,
                &b.src_rel_path,
                &b.src_title,
                "backlink",
                0.5,
            );
        }
    }
    // Resolved outgoing links (what this note links to) — dedupe parallel links
    // to the same target so it scores once.
    let mut out_targets: Vec<i64> = Vec::new();
    let mut seen_out: HashSet<i64> = HashSet::new();
    for l in db.outgoing_links(note.id)? {
        if let Some(id) = l.target_note_id {
            if seen_out.insert(id) {
                out_targets.push(id);
            }
        }
    }
    if !out_targets.is_empty() {
        let meta = db.notes_for_ids(&out_targets)?;
        for id in out_targets {
            if let Some((_kb, rel, title)) = meta.get(&id) {
                bump(id, rel, title, "link", 0.5);
            }
        }
    }
    // Shared tags.
    for tag in db.tags_for_note(note.id)? {
        for n in db.notes_by_tag(std::slice::from_ref(&kb_id), &tag)? {
            bump(n.id, &n.rel_path, &n.title, &format!("tag:{tag}"), 0.3);
        }
    }
    // Vector neighbours. One of four fused channels — tolerate an embedding outage
    // (degrade to link/tag recall) rather than failing the whole tool.
    let source_text = format!("{}\n\n{}", note.title, crate::truncate_utf8(&content, 8000));
    for h in search::similar_notes(&db, std::slice::from_ref(&kb_id), note.id, &source_text, 10)
        .unwrap_or_default()
    {
        bump(
            h.note_id,
            &h.rel_path,
            &h.title,
            "similar",
            0.4 + h.score * 0.6,
        );
    }

    let mut ranked: Vec<(i64, Related)> = map.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.rel_path.cmp(&b.1.rel_path))
    });
    ranked.truncate(20);
    let rows: Vec<_> = ranked
        .iter()
        .map(|(_, r)| {
            serde_json::json!({
                "path": r.rel_path,
                "title": r.title,
                "score": (r.score * 1000.0).round() / 1000.0,
                "reasons": r.reasons.iter().collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb_id,
        "note": note.rel_path,
        "count": rows.len(),
        "related": rows,
    }))?)
}

/// `note_suggest_links({kb?, note})` — other notes whose title/basename appears in
/// this note's body but isn't linked yet (candidates to wire up with `[[ ]]`).
pub(crate) async fn tool_note_suggest_links(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let reference = str_arg(args, "note")
        .or_else(|| str_arg(args, "path"))
        .or_else(|| str_arg(args, "title"))
        .ok_or_else(|| anyhow!("Missing 'note' parameter"))?;
    let (kb_id, note, content) = read_resolved_note(ctx, str_arg(args, "kb"), reference)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;

    // Notes this one already links to (don't re-suggest them).
    let already: HashSet<i64> = db
        .outgoing_links(note.id)?
        .into_iter()
        .filter_map(|l| l.target_note_id)
        .collect();

    let haystack = strip_links_and_code(&content).to_lowercase();
    // `note_refs` has no ORDER BY (rowid order drifts after reindex) — sort by
    // rel_path so the scan window + the 25-cap are deterministic across runs.
    let mut candidates = db.note_refs(&kb_id)?;
    candidates.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    let mut suggestions: Vec<(String, String, String)> = Vec::new(); // (path, title, matched)
    let mut seen: HashSet<i64> = HashSet::new();
    for c in candidates.iter().take(5000) {
        if c.id == note.id || already.contains(&c.id) || seen.contains(&c.id) {
            continue;
        }
        let stem = basename_no_md(&c.rel_path);
        // Prefer the longer/more specific term; require ≥3 chars to cut noise.
        let mut terms = [c.title.trim(), stem.trim()];
        terms.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
        for term in terms {
            if term.chars().count() < 3 {
                continue;
            }
            if contains_word(&haystack, &term.to_lowercase()) {
                suggestions.push((c.rel_path.clone(), c.title.clone(), term.to_string()));
                seen.insert(c.id);
                break;
            }
        }
        if suggestions.len() >= 25 {
            break;
        }
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb_id,
        "note": note.rel_path,
        "count": suggestions.len(),
        "suggestions": suggestions.iter().map(|(p, t, m)| serde_json::json!({
            "path": p, "title": t, "matched": m,
        })).collect::<Vec<_>>(),
    }))?)
}

// ── Tools: AI high-level operations (WS5, side_query driven) ─────

/// Build a background analysis agent + run one bounded side-query, returning the
/// trimmed text. Shared by the WS5 AI note tools (decoupled from the main chat
/// agent via `recap.analysisAgent`, like recall-summary / dreaming).
async fn run_kb_side_query(prompt: &str, max_tokens: u32) -> Result<String> {
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    let res = agent.side_query(prompt, max_tokens).await?;
    Ok(res.text.trim().to_string())
}

/// Title → filesystem-safe note name (kept readable: collapse whitespace, strip
/// path separators + frontmatter/wikilink-hostile chars). Never empty.
fn slugify(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Free rel-path under `scope`, appending ` 2`, ` 3`… on collision so AI writes
/// never silently overwrite an existing note. `resolve_existing` errors = free.
fn unique_rel_path(scope: &WorkspaceScope, base_rel: &str) -> String {
    if scope.resolve_existing(base_rel).is_err() {
        return base_rel.to_string();
    }
    let (stem, ext) = match base_rel.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (base_rel.to_string(), String::new()),
    };
    for n in 2..1000 {
        let candidate = format!("{stem} {n}{ext}");
        if scope.resolve_existing(&candidate).is_err() {
            return candidate;
        }
    }
    base_rel.to_string()
}

/// Whether an existing file looks like a previously-generated MOC (carries the
/// `moc: true` frontmatter marker). Lets `note_moc` refresh its own output while
/// never clobbering a user-authored note that merely shares the slug.
fn is_generated_moc(content: &str) -> bool {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    for l in lines {
        let t = l.trim();
        if t == "---" {
            break;
        }
        if t == "moc: true" {
            return true;
        }
    }
    false
}

/// Pick the MOC write path: refresh `base_rel` only if it's our own prior MOC,
/// otherwise fall back to a unique path so a same-slug user note isn't destroyed.
fn moc_target_path(scope: &WorkspaceScope, base_rel: &str) -> String {
    match scope.resolve_existing(base_rel) {
        Ok(abs) => {
            let is_moc = std::fs::read_to_string(&abs)
                .map(|c| is_generated_moc(&c))
                .unwrap_or(false);
            if is_moc {
                base_rel.to_string()
            } else {
                unique_rel_path(scope, base_rel)
            }
        }
        Err(_) => base_rel.to_string(),
    }
}

/// Quote a YAML scalar when it could be misparsed (special leading/inner chars).
fn yaml_inline(s: &str) -> String {
    let risky = s.is_empty()
        || s.trim() != s
        || s.contains([':', '#', '[', ']', '{', '}', ',', '"', '\'', '\n'].as_slice());
    if risky {
        // Double-quoted YAML scalar: escape backslash + quote, and turn raw
        // control chars into their YAML escapes so a multi-line title stays a
        // single valid scalar (otherwise the frontmatter block breaks on reindex).
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

/// Strip a single wrapping ```lang … ``` fence the model may have added. Only unwraps
/// a *whole-reply* wrap (opening fence + info line + a closing ``` at the very end);
/// content that merely starts with a code block, or a single-line fence, is left
/// untouched so a legitimate leading code block isn't corrupted.
fn strip_code_fence(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some((_info, body)) = rest.split_once('\n') {
            if let Some(inner) = body.strip_suffix("```") {
                return inner.trim().to_string();
            }
        }
    }
    t.to_string()
}

#[derive(serde::Deserialize)]
struct DistilledNote {
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Parse the distill side-query output: the outermost JSON array of notes,
/// tolerating a wrapping code fence + leading/trailing prose.
fn parse_distilled(text: &str) -> Result<Vec<DistilledNote>> {
    let cleaned = strip_code_fence(text);
    let (Some(s), Some(e)) = (cleaned.find('['), cleaned.rfind(']')) else {
        bail!("distill output was not a JSON array");
    };
    if e <= s {
        bail!("distill output was not a JSON array");
    }
    serde_json::from_str::<Vec<DistilledNote>>(&cleaned[s..=e])
        .map_err(|err| anyhow!("could not parse distilled notes JSON: {err}"))
}

/// `note_distill({kb, source?|text?, folder?})` — split a long note / capture into
/// multiple atomic permanent notes (LLM via side_query). Creates new `.md` files.
pub(crate) async fn tool_note_distill(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    // Fail fast (also rejects an external read-only root) before the LLM call.
    let scope = writable_scope(kb)?;
    let source_text =
        if let Some(reference) = str_arg(args, "source").or_else(|| str_arg(args, "note")) {
            let (_kb, note, content) = read_resolved_note(ctx, Some(kb), reference)?;
            format!("# {}\n\n{}", note.title, content)
        } else if let Some(text) = str_arg(args, "text") {
            text.to_string()
        } else {
            bail!("provide 'source' (a note path/title) or 'text' to distill");
        };
    let folder = str_arg(args, "folder")
        .map(|f| f.trim_matches('/').to_string())
        .unwrap_or_default();

    let prompt = format!(
        "You are organizing material into atomic permanent notes (Zettelkasten style). Split the \
         SOURCE into self-contained atomic notes — each capturing ONE idea, with a concise \
         descriptive title and a markdown body that stands on its own. Produce 2 to 8 notes. \
         Return ONLY a JSON array (no prose, no code fence) of objects \
         {{\"title\": string, \"content\": markdown string, \"tags\": [string]}}.\n\nSOURCE:\n{}",
        crate::truncate_utf8(&source_text, 16000)
    );
    let out = run_kb_side_query(&prompt, 4096).await?;
    let notes = parse_distilled(&out)?;
    if notes.is_empty() {
        bail!("distill produced no notes");
    }

    let mut created = Vec::new();
    for n in notes.into_iter().take(12) {
        let title = if n.title.trim().is_empty() {
            "untitled".to_string()
        } else {
            n.title.trim().to_string()
        };
        let base = if folder.is_empty() {
            format!("{}.md", slugify(&title))
        } else {
            format!("{}/{}.md", folder, slugify(&title))
        };
        let rel = unique_rel_path(&scope, &base);
        let mut body = String::from("---\n");
        body.push_str(&format!("title: {}\n", yaml_inline(&title)));
        let tags: Vec<String> = n.tags.iter().map(|t| yaml_inline(t)).collect();
        if !tags.is_empty() {
            body.push_str(&format!("tags: [{}]\n", tags.join(", ")));
        }
        body.push_str("---\n\n");
        body.push_str(n.content.trim());
        body.push('\n');
        write_and_index(&scope, kb, &rel, &body, true)?;
        created.push(rel);
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb,
        "count": created.len(),
        "created": created,
    }))?)
}

/// `note_moc({kb, topic?|tag?})` — generate/refresh a Map-of-Content hub note
/// linking the topic's related notes (LLM via side_query).
pub(crate) async fn tool_note_moc(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    let topic = str_arg(args, "topic");
    let tag = str_arg(args, "tag").map(knowledge::parser::normalize_tag);
    if topic.is_none() && tag.is_none() {
        bail!("provide 'topic' or 'tag' to build a MOC");
    }
    // Fail fast (also rejects an external read-only root) before spending an LLM call.
    let scope = writable_scope(kb)?;
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;

    // A MOC links a curated set; a popular tag (#inbox/#todo) in a big KB can match
    // thousands of notes, which would blow up the side-query prompt (context overflow,
    // latency, cost). Cap the tag fetch — notes_by_tag is ORDER BY rel_path, so the
    // truncation is deterministic — and surface that it was capped.
    const MOC_MAX_TAG_NOTES: usize = 100;
    let mut tag_total: Option<usize> = None;
    let mut collected: Vec<(String, String)> = Vec::new(); // (rel_path, title)
    if let Some(t) = &tag {
        let all = db.notes_by_tag(&[kb.to_string()], t)?;
        if all.len() > MOC_MAX_TAG_NOTES {
            tag_total = Some(all.len());
        }
        for n in all.into_iter().take(MOC_MAX_TAG_NOTES) {
            collected.push((n.rel_path, n.title));
        }
    }
    if let Some(tp) = topic {
        for h in search::search_notes(&db, &[kb.to_string()], tp, 30)? {
            collected.push((h.rel_path, h.title));
        }
    }
    let mut seen = HashSet::new();
    collected.retain(|(p, _)| seen.insert(p.clone()));
    if collected.is_empty() {
        bail!("no notes matched this topic/tag to build a MOC");
    }

    let label = topic
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("#{}", tag.as_deref().unwrap_or("")));
    // Disambiguate same-basename notes (a/Intro.md vs b/Intro.md): use the path
    // form for collisions so the LLM's `[[ ]]` resolve to the intended note.
    let mut base_counts: HashMap<String, usize> = HashMap::new();
    for (p, _) in &collected {
        *base_counts
            .entry(basename_no_md(p).to_lowercase())
            .or_insert(0) += 1;
    }
    let list = collected
        .iter()
        .map(|(p, t)| {
            let bn = basename_no_md(p);
            let r = if base_counts.get(&bn.to_lowercase()).copied().unwrap_or(0) > 1 {
                p.strip_suffix(".md")
                    .or_else(|| p.strip_suffix(".markdown"))
                    .unwrap_or(p)
                    .to_string()
            } else {
                bn
            };
            format!("- [[{r}]] — {t}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "Create a Map of Content (MOC) hub note for \"{label}\". Below are related notes in the \
         knowledge base. Write a concise markdown MOC: a short intro paragraph, then the notes \
         grouped into thematic sections with a one-line annotation each, linking every note with \
         [[wikilinks]] using exactly the refs given. Return ONLY the markdown body (no frontmatter, \
         no code fence).\n\nRELATED NOTES (ref — title):\n{list}"
    );

    let slug = slugify(topic.unwrap_or_else(|| tag.as_deref().unwrap_or("topic")));
    let rel = moc_target_path(&scope, &format!("MOCs/{slug}.md"));

    let body_md = strip_code_fence(&run_kb_side_query(&prompt, 2048).await?);
    if body_md.trim().is_empty() {
        bail!("MOC generation returned empty content");
    }
    // `moc: true` marks this as a generated MOC so a later refresh recognizes it.
    let mut content = format!(
        "---\ntitle: {}\nmoc: true\n---\n\n",
        yaml_inline(&format!("{label} (MOC)"))
    );
    content.push_str(body_md.trim());
    content.push('\n');
    write_and_index(&scope, kb, &rel, &content, false)?;
    let truncated_note = match tag_total {
        Some(total) => format!(
            " Tag matched {total} notes; linked the first {MOC_MAX_TAG_NOTES} (by path) — narrow the tag or split the MOC."
        ),
        None => String::new(),
    };
    Ok(format!(
        "Wrote MOC '{}' in knowledge base '{}' ({} related note(s) linked).{}",
        rel,
        kb,
        collected.len(),
        truncated_note
    ))
}

/// `session_to_note({kb, session?, path?})` — distill a conversation into a single
/// structured permanent note (LLM via side_query). Refuses incognito sources.
pub(crate) async fn tool_session_to_note(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kb = str_arg(args, "kb").ok_or_else(|| anyhow!("Missing 'kb' parameter"))?;
    require_write(ctx, kb)?;
    // Fail fast (also rejects an external read-only root) before the LLM call.
    let scope = writable_scope(kb)?;
    let session_id = str_arg(args, "session")
        .or_else(|| str_arg(args, "session_id"))
        .map(|s| s.to_string())
        .or_else(|| ctx.session_id.clone())
        .ok_or_else(|| anyhow!("no session id — pass 'session' or call from within a session"))?;
    // "Close = burn": never persist an incognito conversation into a note.
    if crate::session::is_session_incognito(Some(&session_id)) {
        bail!("refusing to distill an incognito session into a permanent note");
    }
    let sdb = crate::get_session_db().ok_or_else(|| anyhow!("session db not available"))?;
    let messages = sdb.load_session_messages(&session_id)?;
    let mut transcript = String::new();
    for m in &messages {
        let role = match m.role {
            crate::session::MessageRole::User => "User",
            crate::session::MessageRole::Assistant => "Assistant",
            _ => continue,
        };
        if m.content.trim().is_empty() {
            continue;
        }
        transcript.push_str(&format!("{role}: {}\n\n", m.content.trim()));
    }
    if transcript.trim().is_empty() {
        bail!(
            "session '{}' has no user/assistant text to distill",
            session_id
        );
    }

    let prompt = format!(
        "Distill the following conversation into a single, well-structured permanent note in \
         markdown. Capture the topic, key points, decisions, and any action items. Begin with a \
         concise H1 title. Be faithful — do not invent. Return ONLY the markdown (no frontmatter, \
         no code fence).\n\nCONVERSATION:\n{}",
        crate::truncate_utf8(&transcript, 16000)
    );
    let body_md = strip_code_fence(&run_kb_side_query(&prompt, 3072).await?);
    if body_md.trim().is_empty() {
        bail!("session distillation returned empty content");
    }
    let title = body_md
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Session note".to_string());

    let base = match str_arg(args, "path") {
        Some(p) => norm_note_path(p),
        None => format!("Sessions/{}.md", slugify(&title)),
    };
    let rel = unique_rel_path(&scope, &base);
    let mut content = body_md.trim().to_string();
    content.push('\n');
    write_and_index(&scope, kb, &rel, &content, true)?;
    Ok(format!(
        "Wrote session note '{}' in knowledge base '{}' from {} message(s).",
        rel,
        kb,
        messages.len()
    ))
}

// ── Tools: search / tags ────────────────────────────────────────

/// `note_search({query, kb?, limit?})`
pub(crate) async fn tool_note_search(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let query = str_arg(args, "query").ok_or_else(|| anyhow!("Missing 'query' parameter"))?;
    let kbs = match str_arg(args, "kb") {
        Some(kb) => {
            require_read(ctx, kb)?;
            vec![kb.to_string()]
        }
        None => accessible_kbs(ctx),
    };
    if kbs.is_empty() {
        return Ok("No accessible knowledge bases for this session.".to_string());
    }
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 50) as usize)
        .unwrap_or(10);
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let hits = search::search_notes(&db, &kbs, query, limit)?;
    if hits.is_empty() {
        return Ok(format!("No notes matched '{query}'."));
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "query": query,
        "count": hits.len(),
        "hits": hits,
    }))?)
}

/// `knowledge_recall({query, limit?, kb?, type?})`
///
/// **Store-aware unified recall (D7).** Searches the **memory** store and the
/// **knowledge** notes in one call and returns them as two *separately ranked*
/// sections — never merged or score-normalized (memories are one-line facts,
/// notes are whole documents; mixing pollutes both stores). This is a thin
/// orchestrator: it READS both backends and **does not touch `recall_memory` /
/// the memory store**. The KB side goes through `effective_kb_access` (empty when
/// nothing is attached / incognito / IM not opted-in); the memory side is skipped
/// entirely in an incognito session (close-on-exit red line).
pub(crate) async fn tool_knowledge_recall(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let query = str_arg(args, "query")
        .ok_or_else(|| anyhow!("Missing 'query' parameter"))?
        .to_string();
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 50) as usize)
        .unwrap_or(10);

    // ── Knowledge notes (effective_kb_access gated, same as note_search) ──
    let kbs = match str_arg(args, "kb") {
        Some(kb) => {
            require_read(ctx, kb)?;
            vec![kb.to_string()]
        }
        None => accessible_kbs(ctx),
    };
    let note_hits = if kbs.is_empty() {
        Vec::new()
    } else {
        // Run on a blocking thread: search_notes does SQLite FTS5 + (when vector
        // search is on) an embedding call for the query — same treatment as the
        // memory side below, so neither stalls the async runtime.
        let kbs_c = kbs.clone();
        let q = query.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<knowledge::NoteSearchHit>> {
            let db =
                index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
            search::search_notes(&db, &kbs_c, &q, limit)
        })
        .await??
    };

    // ── Memory store (skipped in incognito — close-on-exit) ──
    let memory_hits: Vec<Value> = if crate::session::is_session_incognito(ctx.session_id.as_deref())
    {
        Vec::new()
    } else {
        let type_filter =
            str_arg(args, "type").map(|t| vec![crate::memory::MemoryType::from_str(t)]);
        let agent_id = ctx.agent_id.clone();
        let q = query.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<Value>> {
            let Some(backend) = crate::get_memory_backend() else {
                return Ok(Vec::new());
            };
            let mq = crate::memory::MemorySearchQuery {
                query: q,
                types: type_filter,
                scope: None,
                agent_id,
                limit: Some(limit),
            };
            Ok(backend
                .search(&mq)?
                .iter()
                .map(|m| {
                    let scope = match &m.scope {
                        crate::memory::MemoryScope::Global => "global".to_string(),
                        crate::memory::MemoryScope::Agent { id } => format!("agent:{id}"),
                        crate::memory::MemoryScope::Project { id } => format!("project:{id}"),
                    };
                    serde_json::json!({
                        "id": m.id,
                        "type": m.memory_type.as_str(),
                        "scope": scope,
                        "pinned": m.pinned,
                        "tags": m.tags,
                        "content": m.content,
                    })
                })
                .collect())
        })
        .await??
    };

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "query": query,
        "memories": { "count": memory_hits.len(), "hits": memory_hits },
        "notes": { "count": note_hits.len(), "hits": note_hits },
    }))?)
}

/// `note_by_tag({kb?, tag})`
pub(crate) async fn tool_note_by_tag(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let tag = str_arg(args, "tag").ok_or_else(|| anyhow!("Missing 'tag' parameter"))?;
    let tag = knowledge::parser::normalize_tag(tag);
    let kbs = match str_arg(args, "kb") {
        Some(kb) => {
            require_read(ctx, kb)?;
            vec![kb.to_string()]
        }
        None => accessible_kbs(ctx),
    };
    if kbs.is_empty() {
        return Ok("No accessible knowledge bases.".to_string());
    }
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let notes = db.notes_by_tag(&kbs, &tag)?;
    let rows: Vec<_> = notes
        .iter()
        .map(|n| serde_json::json!({ "kbId": n.kb_id, "path": n.rel_path, "title": n.title }))
        .collect();
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "tag": tag,
        "count": rows.len(),
        "notes": rows,
    }))?)
}

/// `note_tags({kb?})`
pub(crate) async fn tool_note_tags(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let kbs = match str_arg(args, "kb") {
        Some(kb) => {
            require_read(ctx, kb)?;
            vec![kb.to_string()]
        }
        None => accessible_kbs(ctx),
    };
    if kbs.is_empty() {
        return Ok("No accessible knowledge bases.".to_string());
    }
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let tags = db.all_tags(&kbs)?;
    let rows: Vec<_> = tags
        .iter()
        .map(|(t, c)| serde_json::json!({ "tag": t, "count": c }))
        .collect();
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({ "tags": rows }),
    )?)
}

// ── Text helpers ────────────────────────────────────────────────

/// Append a line under a markdown heading, creating the heading at the end if
/// it does not exist. Inserts before the next same-or-higher-level heading.
fn append_under_heading(content: &str, heading: &str, line: &str) -> String {
    let heading_norm = heading.trim();
    let target = format!("## {heading_norm}");
    // Find the heading line.
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut heading_idx: Option<usize> = None;
    let mut heading_level = 0usize;
    for (i, l) in lines.iter().enumerate() {
        let trimmed = l.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let text = trimmed.trim_start_matches('#').trim();
            if text.eq_ignore_ascii_case(heading_norm) {
                heading_idx = Some(i);
                heading_level = level;
                break;
            }
        }
    }

    match heading_idx {
        Some(idx) => {
            // Insert after the last content line of this section (before the next
            // heading of the same or higher level).
            let mut insert_at = lines.len();
            for (i, l) in lines.iter().enumerate().skip(idx + 1) {
                let trimmed = l.trim_start();
                if trimmed.starts_with('#') {
                    let level = trimmed.chars().take_while(|c| *c == '#').count();
                    if level <= heading_level {
                        insert_at = i;
                        break;
                    }
                }
            }
            lines.insert(insert_at, line.to_string());
            let mut out = lines.join("\n");
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out
        }
        None => {
            let mut out = content.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&target);
            out.push('\n');
            out.push_str(line);
            out.push('\n');
            out
        }
    }
}

#[allow(dead_code)]
fn _root_of(scope: &WorkspaceScope) -> &Path {
    scope.root()
}

/// `folder/Note.md` → `Note` (stem, no extension), `/`-normalized.
fn basename_no_md(rel: &str) -> String {
    let norm = rel.replace('\\', "/");
    let base = norm.rsplit('/').next().unwrap_or(&norm);
    base.strip_suffix(".md")
        .or_else(|| base.strip_suffix(".markdown"))
        .unwrap_or(base)
        .to_string()
}

/// Remove the text between each `open`/`close` pair (inclusive). A dangling
/// `open` with no matching `close` drops only the delimiter and keeps the trailing
/// text (so a stray ``` ` ``` in prose doesn't truncate the rest of the haystack).
fn strip_spans(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        match after.find(close) {
            Some(end) => rest = &after[end + close.len()..],
            None => {
                // Unbalanced open: keep the remaining text (minus the delimiter).
                out.push_str(after);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Strip fenced code, inline code, and existing `[[ ]]` / `![[ ]]` spans from a
/// note body so link suggestions never match inside code or already-linked text.
fn strip_links_and_code(content: &str) -> String {
    let mut body = String::with_capacity(content.len());
    let mut in_fence = false;
    // Lines held while inside a fence — discarded on a proper close, but restored
    // if the fence is never closed (a lone ``` in prose shouldn't eat the rest).
    let mut pending: Vec<&str> = Vec::new();
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            pending.clear();
            continue;
        }
        if in_fence {
            pending.push(line);
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    if in_fence {
        for l in pending {
            body.push_str(l);
            body.push('\n');
        }
    }
    // `![[x]]` is covered by the `[[`..`]]` pass (the `!` is left, harmless).
    let body = strip_spans(&body, "[[", "]]");
    strip_spans(&body, "`", "`")
}

/// Case-insensitive whole-word-ish containment: `needle` occurs in `haystack`
/// with non-alphanumeric ASCII boundaries (so "cat" doesn't match "category").
/// Both inputs must already be lowercased. CJK (no ASCII boundary) → substring.
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = start == 0
            || !haystack[..start]
                .chars()
                .next_back()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        let after_ok = end >= haystack.len()
            || !haystack[end..]
                .chars()
                .next()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_path_adds_md() {
        assert_eq!(norm_note_path("foo"), "foo.md");
        assert_eq!(norm_note_path("a/b.md"), "a/b.md");
        assert_eq!(norm_note_path("/x"), "x.md");
    }

    #[test]
    fn append_under_existing_heading() {
        let c = "# T\n\n## Related\n\n- [[a]]\n\n## Other\n\nx\n";
        let out = append_under_heading(c, "Related", "- [[b]]");
        // New line lands within the Related section, before "## Other".
        let related_pos = out.find("## Related").unwrap();
        let other_pos = out.find("## Other").unwrap();
        let b_pos = out.find("[[b]]").unwrap();
        assert!(b_pos > related_pos && b_pos < other_pos);
    }

    #[test]
    fn append_creates_missing_heading() {
        let c = "# T\n\nbody\n";
        let out = append_under_heading(c, "Related", "- [[b]]");
        assert!(out.contains("## Related"));
        assert!(out.contains("[[b]]"));
    }

    #[test]
    fn fmt_block_ref_builds_reference() {
        assert_eq!(fmt_block_ref("Note", "x1"), "[[Note#^x1]]");
        assert_eq!(fmt_block_ref("a/Note", "abc"), "[[a/Note#^abc]]");
    }

    /// Apply a placement the way the tool does, for assertion convenience.
    fn place(content: &str, block_text: &str) -> Result<(Option<String>, String)> {
        let p = resolve_anchor_placement(content, block_text, "n.md")?;
        if let Some(id) = p.existing_id {
            return Ok((Some(id), content.to_string()));
        }
        let mut out = String::new();
        out.push_str(&content[..p.insert_at]);
        out.push_str(" ^new");
        out.push_str(&content[p.insert_at..]);
        Ok((None, out))
    }

    #[test]
    fn placement_single_line_paragraph() {
        let (_, out) = place("# T\n\nHello world.\n\nNext.\n", "Hello world.").unwrap();
        assert!(out.contains("Hello world. ^new\n"), "got: {out}");
    }

    #[test]
    fn placement_multiline_paragraph_anchors_last_line() {
        // block_text matches the FIRST line; the id must land on the LAST line of
        // the paragraph, not mid-block (would truncate the block).
        let (_, out) = place("A para\nspanning lines.\n\nNext.\n", "A para").unwrap();
        assert!(out.contains("A para\nspanning lines. ^new"), "got: {out}");
        assert!(
            !out.contains("A para ^new"),
            "anchor must not split the block: {out}"
        );
    }

    #[test]
    fn placement_idempotent_trailing_anchor() {
        let (id, out) = place("Hello world. ^abc\n", "Hello world.").unwrap();
        assert_eq!(id.as_deref(), Some("abc"));
        assert_eq!(out, "Hello world. ^abc\n", "must not write a second anchor");
    }

    #[test]
    fn placement_idempotent_own_line_anchor_below_paragraph() {
        // Existing id sits on its own line below a multi-line block; matching the
        // first line must still detect it (no double anchor).
        let (id, _) = place("A para\nspanning.\n^xyz\n", "A para").unwrap();
        assert_eq!(id.as_deref(), Some("xyz"));
    }

    #[test]
    fn placement_list_item_is_single_block() {
        let (_, out) = place("- first\n- second item\n- third\n", "second item").unwrap();
        assert!(out.contains("- second item ^new\n"), "got: {out}");
        assert!(
            !out.contains("- third ^new"),
            "must not anchor a sibling item: {out}"
        );
    }

    #[test]
    fn placement_rejects_frontmatter_and_code() {
        assert!(place("---\ntitle: Secret\n---\n\nbody\n", "Secret").is_err());
        assert!(place("text\n\n```\nfn secret() {}\n```\n", "secret").is_err());
    }

    #[test]
    fn gen_block_id_is_deterministic_and_collision_free() {
        let mut existing = std::collections::HashSet::new();
        let a = gen_block_id("the same line", &existing);
        let b = gen_block_id("the same line", &existing);
        assert_eq!(a, b, "same seed → same id (no RNG)");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        // With the 6-char prefix taken, a different length is chosen.
        existing.insert(a.clone());
        let c = gen_block_id("the same line", &existing);
        assert_ne!(a, c, "must avoid an existing id");
        assert!(!existing.contains(&c));
    }

    #[test]
    fn contains_word_respects_ascii_boundaries() {
        assert!(contains_word("a cat sits", "cat"));
        assert!(!contains_word("category theory", "cat"));
        assert!(contains_word("see [foo] here", "foo"));
        // CJK has no ASCII boundary → substring match is fine.
        assert!(contains_word("机器学习很有趣", "机器学习"));
        assert!(!contains_word("anything", ""));
    }

    #[test]
    fn strip_links_and_code_removes_links_and_code() {
        let src = "see [[Linked Note]] and `code` here\n```\n[[InFence]]\n```\nplain Other Note";
        let out = strip_links_and_code(src);
        assert!(!out.contains("Linked Note"));
        assert!(!out.contains("InFence"));
        assert!(!out.contains("code"));
        assert!(out.contains("Other Note"));
    }

    #[test]
    fn strip_links_and_code_survives_stray_backtick() {
        // A lone inline backtick must not truncate the rest of the haystack.
        let out = strip_links_and_code("use the `--flag and mention Other Note");
        assert!(out.contains("Other Note"));
    }

    #[test]
    fn strip_links_and_code_survives_unclosed_fence() {
        // An unclosed fence must not swallow the remainder for suggestions.
        let out = strip_links_and_code("intro\n```\ncode line\nmention Other Note");
        assert!(out.contains("Other Note"));
    }

    #[test]
    fn is_generated_moc_detects_marker() {
        assert!(is_generated_moc(
            "---\ntitle: X (MOC)\nmoc: true\n---\n\nbody"
        ));
        assert!(!is_generated_moc("---\ntitle: X\n---\n\nbody"));
        assert!(!is_generated_moc("# Just a user note\n\nmoc: true")); // not in frontmatter
        assert!(!is_generated_moc("no frontmatter"));
    }

    #[test]
    fn slugify_sanitizes() {
        assert_eq!(slugify("Hello / World: test"), "Hello World test");
        assert_eq!(slugify("  spaced   out  "), "spaced out");
        assert_eq!(slugify("///"), "untitled");
    }

    #[test]
    fn parse_distilled_handles_fenced_array() {
        let out = "```json\n[{\"title\":\"A\",\"content\":\"body\",\"tags\":[\"x\"]}]\n```";
        let notes = parse_distilled(out).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "A");
        assert_eq!(notes[0].tags, vec!["x".to_string()]);
        // Prose with an embedded array still parses.
        let messy = "Here you go:\n[{\"title\":\"B\",\"content\":\"\"}]\nDone.";
        assert_eq!(parse_distilled(messy).unwrap()[0].title, "B");
        // Non-array → error.
        assert!(parse_distilled("not json").is_err());
    }

    #[test]
    fn yaml_inline_quotes_risky() {
        assert_eq!(yaml_inline("plain"), "plain");
        assert_eq!(yaml_inline("a: b"), "\"a: b\"");
        assert_eq!(yaml_inline(" leading"), "\" leading\"");
        // A newline must become a `\n` escape, not a raw line break.
        assert_eq!(yaml_inline("line1\nline2"), "\"line1\\nline2\"");
    }
}
