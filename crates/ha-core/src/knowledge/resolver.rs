//! Deterministic `[[ref]]` → note resolution (design D8 / #8).
//!
//! Resolution is **fully deterministic and never time-dependent** (no mtime), so
//! links don't silently drift as files are touched. Matching is NFC-normalized
//! and case-insensitive (Obsidian default); the displayed text keeps its
//! original case.

/// A minimal note descriptor for resolution.
#[derive(Debug, Clone)]
pub struct NoteRef {
    pub id: i64,
    /// Path relative to the KB root, `/`-separated, including `.md`.
    pub rel_path: String,
    pub title: String,
}

/// Resolve a wikilink target to a note id, or `None` (dangling/broken).
///
/// Priority:
/// 1. **Path form** (`folder/note`): exact normalized path match.
/// 2. **Unique basename** (`note`): the file stem or title matches.
/// 3. **Same-name ambiguity**: shortest path, then lexicographic path — never
///    mtime / most-recent.
pub fn resolve(target_ref: &str, notes: &[NoteRef]) -> Option<i64> {
    let target = clean_target(target_ref);
    if target.is_empty() {
        return None;
    }

    if target.contains('/') {
        // Path form — match the full normalized rel-path (sans `.md`).
        let candidates: Vec<&NoteRef> = notes
            .iter()
            .filter(|n| norm(&strip_md(&n.rel_path)) == target)
            .collect();
        return pick(&candidates);
    }

    // Basename form — match file stem or title.
    let candidates: Vec<&NoteRef> = notes
        .iter()
        .filter(|n| norm(&basename_no_md(&n.rel_path)) == target || norm(&n.title) == target)
        .collect();
    pick(&candidates)
}

/// Deterministic ambiguity tie-break: shortest path (fewest `/`), then
/// lexicographic rel-path.
fn pick(candidates: &[&NoteRef]) -> Option<i64> {
    candidates
        .iter()
        .min_by(|a, b| {
            let da = a.rel_path.matches('/').count();
            let db = b.rel_path.matches('/').count();
            da.cmp(&db).then_with(|| a.rel_path.cmp(&b.rel_path))
        })
        .map(|n| n.id)
}

/// Trim, drop a leading `./`, strip a trailing `.md`, normalize.
fn clean_target(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("./").unwrap_or(s);
    norm(&strip_md(s))
}

fn strip_md(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    if lower.ends_with(".md") {
        s[..s.len() - 3].to_string()
    } else if lower.ends_with(".markdown") {
        s[..s.len() - 9].to_string()
    } else {
        s.to_string()
    }
}

fn basename_no_md(rel_path: &str) -> String {
    let p = rel_path.replace('\\', "/");
    let last = p.rsplit('/').next().unwrap_or(&p).to_string();
    strip_md(&last)
}

/// NFC + lowercase + `\` → `/`, the canonical resolve key.
fn norm(s: &str) -> String {
    super::parser::nfc(&s.replace('\\', "/")).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: i64, rel: &str, title: &str) -> NoteRef {
        NoteRef {
            id,
            rel_path: rel.into(),
            title: title.into(),
        }
    }

    #[test]
    fn path_form_exact() {
        let notes = vec![n(1, "a/note.md", "Note"), n(2, "b/note.md", "Note")];
        assert_eq!(resolve("a/note", &notes), Some(1));
        assert_eq!(resolve("b/note", &notes), Some(2));
    }

    #[test]
    fn basename_unique() {
        let notes = vec![n(1, "deep/folder/Foo.md", "Foo")];
        assert_eq!(resolve("foo", &notes), Some(1));
        assert_eq!(resolve("Foo", &notes), Some(1));
    }

    #[test]
    fn basename_matches_title() {
        let notes = vec![n(1, "x/file.md", "My Title")];
        assert_eq!(resolve("My Title", &notes), Some(1));
    }

    #[test]
    fn ambiguity_shortest_then_lexicographic() {
        let notes = vec![
            n(1, "deep/sub/Note.md", "Note"),
            n(2, "Note.md", "Note"),
            n(3, "other/Note.md", "Note"),
        ];
        // Shortest path (root) wins.
        assert_eq!(resolve("Note", &notes), Some(2));
    }

    #[test]
    fn ambiguity_same_depth_lexicographic() {
        let notes = vec![n(1, "b/Note.md", "Note"), n(2, "a/Note.md", "Note")];
        assert_eq!(resolve("Note", &notes), Some(2)); // "a/" < "b/"
    }

    #[test]
    fn no_match_is_none() {
        let notes = vec![n(1, "a.md", "A")];
        assert_eq!(resolve("missing", &notes), None);
    }

    #[test]
    fn case_and_md_insensitive() {
        let notes = vec![n(1, "Folder/Note.md", "Note")];
        assert_eq!(resolve("folder/note.md", &notes), Some(1));
    }
}
