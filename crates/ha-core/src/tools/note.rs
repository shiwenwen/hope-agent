//! `note_*` knowledge-base tools (design AI Knowledge Operations, Layer 1).
//!
//! Every tool routes its `kb` through [`effective_kb_access`] (D10), writes are
//! confined to `WorkspaceScope::for_knowledge` (external roots rejected, D11),
//! and stale-write guards re-hash the **disk file** (never the index cache) per
//! the v4.6 contract. Mutations re-index synchronously and emit
//! `knowledge:changed`.

use std::collections::HashMap;
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
    let source = ctx.chat_source.unwrap_or(KbAccessSource::Gui);
    // Call-chain origin (D10): a subagent carries its parent turn's origin so an
    // IM-origin chain can't reacquire KB access via the neutral Subagent source.
    // Falls back to `source` when unset (contexts not built by the chat engine).
    let origin = ctx.origin_chat_source.unwrap_or(source);
    let actx = KnowledgeAccessContext::resolve(
        ctx.session_id.clone(),
        ctx.project_id.clone(),
        source,
        origin,
    );
    effective_kb_access(&actx)
}

fn require_write(ctx: &ToolExecContext, kb_id: &str) -> Result<()> {
    match access_map(ctx).get(kb_id) {
        Some(KbAccess::Write) => Ok(()),
        Some(KbAccess::Read) => bail!(
            "knowledge base '{}' is read-only for this session (external vaults are read-only in Phase 1)",
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
    guard_stale(&scope, &rel, str_arg(args, "expected_file_hash"))?;

    let hash = write_and_index(&scope, kb, &rel, content, false)?;
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

/// `note_backlinks({kb?, note})`
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
    let backlinks = db.backlinks(note.id)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "kbId": kb_id,
        "note": rel,
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
            knowledge::graph::cap_nodes(
                knowledge::graph::ego_subgraph(&full, center, depth),
                200,
            )
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
}
