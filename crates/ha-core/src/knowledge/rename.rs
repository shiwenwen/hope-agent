//! Note/folder rename + move with inbound `[[ ]]` link rewriting (design
//! `note_rename`/`note_move`, #9).
//!
//! Renaming a note moves its `.md` file, but path-form links (`[[folder/note]]`)
//! and links to filename-derived titles in *other* notes would dangle. This
//! module rewrites those inbound links so the graph stays intact. The pure text
//! transform ([`rewrite_content`]) is split out from the IO so it is unit-tested
//! without the registry / index globals. All matching follows the deterministic
//! resolver conventions (NFC + case-insensitive; path form vs basename, #8).
//!
//! Shared by the owner plane ([`super::service`] / GUI) and the agent tools
//! ([`crate::tools::note`]); both reject read-only external roots (D11).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use super::db::IndexDb;
use super::types::{LinkType, RenameOutcome};
use super::{index, parser};
use crate::filesystem::{self, WorkspaceScope};

/// One note's old→new identity, in both reference forms (path + basename).
struct Move {
    /// Full rel path without extension (path-form link key).
    old_path: String,
    /// File basename without extension (basename-form link key).
    old_stem: String,
    new_path: String,
    new_stem: String,
}

impl Move {
    fn new(old_rel: &str, new_rel: &str) -> Self {
        Self {
            old_path: strip_md(old_rel),
            old_stem: basename_no_md(old_rel),
            new_path: strip_md(new_rel),
            new_stem: basename_no_md(new_rel),
        }
    }
}

/// Rename/move a single note within a KB and rewrite inbound `[[ ]]` links in
/// other notes. Rejects read-only (external) roots. Optional stale-write guard
/// on the source note (disk raw BLAKE3 vs `expected_file_hash`).
pub fn rename_note(
    kb_id: &str,
    from_rel: &str,
    to_rel: &str,
    expected_file_hash: Option<&str>,
) -> Result<RenameOutcome> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    let to_rel = normalize_md(to_rel);

    // Stale-write guard on the source note (compare disk raw hash, never cache).
    if let Some(expected) = expected_file_hash {
        match scope
            .resolve_existing(from_rel)
            .ok()
            .and_then(|abs| std::fs::read(&abs).ok())
        {
            Some(b) => {
                let cur = super::blake3_hex(&b);
                if cur != expected {
                    bail!(
                        "stale write: note '{}' changed on disk (expected_file_hash mismatch, current {})",
                        from_rel,
                        cur
                    );
                }
            }
            None => bail!("note '{}' not found", from_rel),
        }
    }

    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let root = scope.root().to_path_buf();

    // The index stores the on-disk casing; on a case-insensitive FS a
    // differently-cased `from_rel` (agent / HTTP body) would make the exact-match
    // index delete + link path-match silently miss, leaving a stale duplicate row.
    // Resolve the source back to its canonical rel before the move erases it.
    let canon_from = scope
        .resolve_existing(from_rel)
        .ok()
        .map(|abs| scope.rel_of(&abs))
        .unwrap_or_else(|| from_rel.to_string());

    // Move the file (creates parent dirs, refuses to clobber).
    let res = filesystem::project_rename(&scope, from_rel, &to_rel, false)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    let new_rel = res.rel_path;

    // Drop the old index row + index the new path (batch-resolve once at the end).
    db.delete_note(kb_id, &canon_from)?;
    index::reindex_note_no_resolve(kb_id, &root, &new_rel)?;

    let moves = vec![Move::new(&canon_from, &new_rel)];
    let (files_changed, links_rewritten) =
        rewrite_inbound_for_moves(&scope, &db, kb_id, &root, &moves)?;
    db.reresolve_kb_links(kb_id)?;
    emit_changed(kb_id, "rename");
    Ok(RenameOutcome {
        new_rel,
        files_changed,
        links_rewritten,
    })
}

/// Rename/move a folder and everything under it, rewriting inbound path-form
/// links across the KB. Blocking (full reindex inside) — call from a blocking
/// context. Rejects read-only roots.
pub fn rename_dir(kb_id: &str, from_rel: &str, to_rel: &str) -> Result<RenameOutcome> {
    let scope =
        WorkspaceScope::for_knowledge(kb_id).map_err(|e| anyhow!(e.message().to_string()))?;
    if scope.is_read_only() {
        bail!("knowledge base '{}' root is read-only", kb_id);
    }
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let root = scope.root().to_path_buf();

    // Notes under the old folder (captured before the move) → the move map. Use
    // the canonical on-disk casing so a differently-cased `from_rel` still matches
    // the indexed rel_paths (case-insensitive FS) — otherwise the move set is
    // empty and inbound links silently never get rewritten.
    let canon_from = scope
        .resolve_existing(from_rel)
        .ok()
        .map(|abs| scope.rel_of(&abs))
        .unwrap_or_else(|| from_rel.to_string());
    let from_prefix = format!("{}/", canon_from.trim_matches('/'));
    let moved: Vec<String> = db
        .list_notes(kb_id)?
        .into_iter()
        .map(|n| n.rel_path)
        .filter(|p| p.starts_with(&from_prefix))
        .collect();

    let res = filesystem::project_rename(&scope, from_rel, to_rel, false)
        .map_err(|e| anyhow!(e.message().to_string()))?;
    let new_dir_rel = res.rel_path;
    let to_prefix = format!("{}/", new_dir_rel.trim_matches('/'));

    // Rebuild the KB index (prune old paths, index new). Resolves at its end.
    index::reindex_kb(kb_id, false)?;

    let moves: Vec<Move> = moved
        .iter()
        .map(|old| {
            let suffix = &old[from_prefix.len()..];
            Move::new(old, &format!("{to_prefix}{suffix}"))
        })
        .collect();

    let (files_changed, links_rewritten) =
        rewrite_inbound_for_moves(&scope, &db, kb_id, &root, &moves)?;
    db.reresolve_kb_links(kb_id)?;
    emit_changed(kb_id, "rename");
    Ok(RenameOutcome {
        new_rel: new_dir_rel,
        files_changed,
        links_rewritten,
    })
}

// ── internals ───────────────────────────────────────────────────

/// Rewrite every note in the KB whose links point to a moved note. Reads each
/// file fresh + re-parses (so code-block `[[ ]]` are skipped and positions are
/// never stale), writes back changed files + reindexes them (no resolve — the
/// caller resolves once). Returns `(files_changed, links_rewritten)`.
fn rewrite_inbound_for_moves(
    scope: &WorkspaceScope,
    db: &IndexDb,
    kb_id: &str,
    root: &Path,
    moves: &[Move],
) -> Result<(usize, usize)> {
    if moves.is_empty() {
        return Ok((0, 0));
    }
    let all_notes = db.list_notes(kb_id)?;

    // Post-move "bare-ref" frequency → a basename link uses the path form when its
    // new stem would resolve ambiguously. The resolver matches a bare `[[X]]`
    // against BOTH file stem AND note title (resolver.rs), so count both (each note
    // once per distinct key) — counting stems only would miss a title collision and
    // let a bare rewrite mis-resolve via the tie-break.
    let mut stem_counts: HashMap<String, u32> = HashMap::new();
    for n in &all_notes {
        let mut keys = std::collections::HashSet::new();
        keys.insert(norm(&basename_no_md(&n.rel_path)));
        keys.insert(norm(&n.title));
        for k in keys {
            *stem_counts.entry(k).or_default() += 1;
        }
    }

    let (path_map, stem_map) = build_maps(moves, &stem_counts);

    let mut files_changed = 0usize;
    let mut links_rewritten = 0usize;
    for note in &all_notes {
        let Ok(abs) = scope.resolve_existing(&note.rel_path) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        // Skip notes with invalid UTF-8 rather than round-tripping through a lossy
        // conversion that would silently replace their stray bytes with U+FFFD on
        // write. Integrity over link-completeness: the link stays as-is.
        let Ok(content) = String::from_utf8(bytes) else {
            crate::app_warn!(
                "knowledge",
                "rename",
                "skipping link rewrite for non-utf8 note {}",
                note.rel_path
            );
            continue;
        };
        let Some((updated, n)) = rewrite_content(&content, &path_map, &stem_map) else {
            continue;
        };
        filesystem::project_write_text(scope, &note.rel_path, &updated, false)
            .map_err(|e| anyhow!(e.message().to_string()))?;
        index::reindex_note_no_resolve(kb_id, root, &note.rel_path)?;
        files_changed += 1;
        links_rewritten += n;
    }
    Ok((files_changed, links_rewritten))
}

/// Build the (path-form, basename-form) lookup maps for a set of moves.
fn build_maps(
    moves: &[Move],
    stem_counts: &HashMap<String, u32>,
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut path_map: HashMap<String, String> = HashMap::new();
    let mut stem_map: HashMap<String, String> = HashMap::new();
    for m in moves {
        path_map.insert(norm(&m.old_path), m.new_path.clone());
        let ambiguous = stem_counts.get(&norm(&m.new_stem)).copied().unwrap_or(0) > 1;
        let new_ref = if ambiguous {
            m.new_path.clone()
        } else {
            m.new_stem.clone()
        };
        stem_map.insert(norm(&m.old_stem), new_ref);
    }
    (path_map, stem_map)
}

/// Pure transform: rewrite a single note's content given the move maps. Returns
/// `Some((new_content, count))` when at least one link changed, else `None`.
/// Path-form refs (`folder/note`) match `path_map`; basename refs match
/// `stem_map`. Code-block `[[ ]]` are skipped by the parser. No IO.
fn rewrite_content(
    content: &str,
    path_map: &HashMap<String, String>,
    stem_map: &HashMap<String, String>,
) -> Option<(String, usize)> {
    let parsed = parser::parse_document(content);
    let mut splices: Vec<(usize, usize, String)> = Vec::new();
    for link in &parsed.links {
        let new_ref = if link.target_ref.contains('/') {
            path_map.get(&norm(&link.target_ref)).cloned()
        } else {
            stem_map.get(&norm(&link.target_ref)).cloned()
        };
        let Some(new_ref) = new_ref else {
            continue;
        };
        let is_embed = link.link_type == LinkType::Embed;
        let new_raw = rebuild_link(
            is_embed,
            &new_ref,
            link.anchor.as_deref(),
            link.alias.as_deref(),
        );
        if new_raw == link.raw_text {
            continue;
        }
        let start = cp_to_byte(content, link.start.offset as usize);
        let len = link.raw_text.len();
        // Defensive: only splice when the bytes match the recorded link text.
        if content.get(start..start + len) != Some(link.raw_text.as_str()) {
            continue;
        }
        splices.push((start, len, new_raw));
    }
    if splices.is_empty() {
        return None;
    }
    // Apply right-to-left so earlier byte offsets stay valid.
    splices.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut out = content.to_string();
    for (start, len, new_raw) in &splices {
        out.replace_range(*start..(*start + *len), new_raw);
    }
    Some((out, splices.len()))
}

fn rebuild_link(is_embed: bool, target: &str, anchor: Option<&str>, alias: Option<&str>) -> String {
    let mut out = String::new();
    if is_embed {
        out.push('!');
    }
    out.push_str("[[");
    out.push_str(target);
    if let Some(a) = anchor {
        out.push('#');
        out.push_str(a);
    }
    if let Some(al) = alias {
        out.push('|');
        out.push_str(al);
    }
    out.push_str("]]");
    out
}

fn cp_to_byte(s: &str, cp: usize) -> usize {
    s.char_indices().nth(cp).map(|(b, _)| b).unwrap_or(s.len())
}

fn strip_md(s: &str) -> String {
    let p = s.replace('\\', "/");
    let lower = p.to_ascii_lowercase();
    if lower.ends_with(".md") {
        p[..p.len() - 3].to_string()
    } else if lower.ends_with(".markdown") {
        p[..p.len() - 9].to_string()
    } else {
        p
    }
}

fn basename_no_md(rel: &str) -> String {
    let p = strip_md(rel);
    p.rsplit('/').next().unwrap_or(&p).to_string()
}

/// NFC + lowercase + `\` → `/`, the canonical resolve key (matches resolver).
fn norm(s: &str) -> String {
    parser::nfc(&s.replace('\\', "/")).to_lowercase()
}

/// Normalize a destination rel-path the way `norm_note_path` does at the tool
/// layer (trim, drop leading `/`, `\` → `/`) and ensure a markdown extension, so
/// `note_rename`/`note_move` handle `to` consistently with every other note tool.
fn normalize_md(to_rel: &str) -> String {
    let p = to_rel.trim().trim_start_matches('/').replace('\\', "/");
    let lower = p.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        p
    } else {
        format!("{p}.md")
    }
}

fn emit_changed(kb_id: &str, op: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "knowledge:changed",
            serde_json::json!({ "kbId": kb_id, "op": op }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps(
        path: &[(&str, &str)],
        stem: &[(&str, &str)],
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        let p = path.iter().map(|(k, v)| (norm(k), v.to_string())).collect();
        let s = stem.iter().map(|(k, v)| (norm(k), v.to_string())).collect();
        (p, s)
    }

    #[test]
    fn rewrites_path_form_link() {
        let (p, s) = maps(&[("folder/old", "folder/new")], &[]);
        let (out, n) = rewrite_content("See [[folder/old]] here.\n", &p, &s).unwrap();
        assert_eq!(out, "See [[folder/new]] here.\n");
        assert_eq!(n, 1);
    }

    #[test]
    fn rewrites_basename_preserving_anchor_alias() {
        let (p, s) = maps(&[], &[("old", "new")]);
        let (out, n) = rewrite_content("Link [[old#Heading|Alias]].\n", &p, &s).unwrap();
        assert_eq!(out, "Link [[new#Heading|Alias]].\n");
        assert_eq!(n, 1);
    }

    #[test]
    fn rewrites_embed() {
        let (p, s) = maps(&[], &[("old", "new")]);
        let (out, _) = rewrite_content("![[old]]\n", &p, &s).unwrap();
        assert_eq!(out, "![[new]]\n");
    }

    #[test]
    fn skips_links_in_code() {
        let (p, s) = maps(&[], &[("old", "new")]);
        // Real link rewritten; the one inside inline code is left untouched.
        let (out, n) = rewrite_content("[[old]] and `[[old]]`\n", &p, &s).unwrap();
        assert_eq!(out, "[[new]] and `[[old]]`\n");
        assert_eq!(n, 1);
    }

    #[test]
    fn no_change_returns_none() {
        let (p, s) = maps(&[("folder/x", "folder/y")], &[]);
        assert!(rewrite_content("nothing to see [[unrelated]]\n", &p, &s).is_none());
    }

    #[test]
    fn rewrites_multiple_occurrences() {
        let (p, s) = maps(&[], &[("old", "new")]);
        let (out, n) = rewrite_content("[[old]] then [[old|a]]\n", &p, &s).unwrap();
        assert_eq!(out, "[[new]] then [[new|a]]\n");
        assert_eq!(n, 2);
    }
}
