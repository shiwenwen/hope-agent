//! Tool diff metadata helpers used by file-mutating tools (write / edit /
//! apply_patch) and file-reading tools (read) to emit before/after snapshots
//! plus line deltas via [`super::ToolExecContext::emit_metadata`]. The
//! frontend consumes the resulting JSON to render `+N -M` summaries and the
//! right-side diff panel.
//!
//! Pure helpers — no IO, no Tauri deps.

use similar::{ChangeTag, TextDiff};

/// Single-side cap for the embedded `before` / `after` content. Larger inputs
/// are still passed to [`compute_line_delta`] so the line counters stay
/// accurate, but only the truncated form is stored in metadata. The frontend
/// shows a "file too large" hint when `truncated == true`.
pub const MAX_METADATA_CONTENT_BYTES: usize = 256 * 1024;

/// Compute (added, removed) line counts. Uses [`TextDiff::from_lines`] so the
/// numbers always agree with what the frontend's `diffLines` rendering will
/// show.
pub fn compute_line_delta(before: &str, after: &str) -> (u32, u32) {
    let diff = TextDiff::from_lines(before, after);
    let mut added: u32 = 0;
    let mut removed: u32 = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added = added.saturating_add(1),
            ChangeTag::Delete => removed = removed.saturating_add(1),
            ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// Map a file extension to a Shiki language id, returning `"text"` for
/// unknowns. Lower-cased so `Foo.RS` and `foo.rs` agree.
pub fn detect_language(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let ext = std::path::Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "shell",
        "ps1" => "powershell",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" | "html" | "htm" | "svg" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "lua" => "lua",
        "dart" => "dart",
        _ => "text",
    }
}

/// Truncate `content` for metadata storage. Returns `(truncated_string,
/// was_truncated)`. UTF-8 safe via [`crate::truncate_utf8`].
pub fn truncate_for_metadata(content: &str) -> (String, bool) {
    if content.len() <= MAX_METADATA_CONTENT_BYTES {
        (content.to_string(), false)
    } else {
        (
            crate::truncate_utf8(content, MAX_METADATA_CONTENT_BYTES).to_string(),
            true,
        )
    }
}

/// Read a file's pre-write snapshot for diff metadata, **bounded** by
/// [`MAX_METADATA_CONTENT_BYTES`] so that overwriting a huge file does not
/// pull the whole thing into memory just for a panel that will only render
/// 256 KiB anyway.
///
/// Returns:
/// - `None` if the path does not exist, is not a regular file, or cannot be
///   read as UTF-8 (binary file). Caller treats as "create".
/// - `Some((content, false))` if the file fit under the cap.
/// - `Some((String::new(), true))` if the file existed but exceeded the cap.
///   Caller still classifies as "edit" but the panel will show a truncated
///   marker instead of the actual diff.
pub async fn read_for_diff_metadata(path: &str) -> Option<(String, bool)> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() as usize > MAX_METADATA_CONTENT_BYTES {
        return Some((String::new(), true));
    }
    tokio::fs::read_to_string(path).await.ok().map(|s| (s, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_delta_simple_edit() {
        let before = "line1\nline2\nline3\n";
        let after = "line1\nline2-changed\nline3\n";
        let (added, removed) = compute_line_delta(before, after);
        assert_eq!((added, removed), (1, 1));
    }

    #[test]
    fn line_delta_pure_insert() {
        let before = "line1\nline2\n";
        let after = "line1\nline2\nline3\n";
        let (added, removed) = compute_line_delta(before, after);
        assert_eq!((added, removed), (1, 0));
    }

    #[test]
    fn line_delta_create_from_empty() {
        let (added, removed) = compute_line_delta("", "a\nb\nc\n");
        assert_eq!((added, removed), (3, 0));
    }

    #[test]
    fn detect_language_known_extensions() {
        assert_eq!(detect_language("a.rs"), "rust");
        assert_eq!(detect_language("foo/bar.TSX"), "tsx");
        assert_eq!(detect_language("readme.md"), "markdown");
        assert_eq!(detect_language("noext"), "text");
    }

    #[test]
    fn truncate_preserves_short_input() {
        let (out, was_trunc) = truncate_for_metadata("hello");
        assert_eq!(out, "hello");
        assert!(!was_trunc);
    }

    #[test]
    fn truncate_caps_long_input() {
        let big = "x".repeat(MAX_METADATA_CONTENT_BYTES + 100);
        let (out, was_trunc) = truncate_for_metadata(&big);
        assert!(was_trunc);
        assert!(out.len() <= MAX_METADATA_CONTENT_BYTES);
    }
}
