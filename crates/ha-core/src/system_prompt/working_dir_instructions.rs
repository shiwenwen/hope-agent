//! Auto-injected user instructions for the session's working directory.
//!
//! Discovers `AGENTS.md` (preferred) or `CLAUDE.md` (fallback) at the
//! working directory's top level, recursively expands `@path` references
//! into a flat list, and returns the resulting [`InstructionFile`]s for
//! the system prompt builder to render under `# Working Directory`.
//!
//! Semantics mirror Anthropic's claude-code (`~/Codes/claude-code/src/utils/claudemd.ts`):
//! `@path`, `@./path`, `@~/path`, `@/path` are all valid; references inside
//! fenced code blocks or inline code spans are ignored; recursion depth is
//! bounded; the visited set is keyed by canonicalized path so cycles
//! terminate. Each file's text is truncated to [`MAX_FILE_CHARS`] using the
//! shared head/tail helper. The total file count is also capped to keep a
//! buggy include graph from blowing up the prompt.

use super::constants::MAX_FILE_CHARS;
use super::helpers::truncate;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Maximum total instruction files (entry + transitively included) the
/// builder will inject in one system prompt.
const MAX_INSTRUCTION_FILES: usize = 16;

/// Maximum recursion depth for `@path` includes. Mirrors claude-code's
/// `MAX_INCLUDE_DEPTH = 5`.
const MAX_INCLUDE_DEPTH: usize = 5;

// TODO(working-dir-instructions): expose MAX_INSTRUCTION_FILES /
// MAX_INCLUDE_DEPTH via AppConfig once a real user need surfaces (see
// plan TODO #2). Same goes for the extension allowlist below.
//
// TODO(working-dir-instructions): `collect_working_dir_instructions` runs
// on every system_prompt::build() (every chat turn). Worst case it does
// ~20–70 syscalls (find_entry + read + canonicalize per file). On local
// SSD this is sub-millisecond, but if it ever shows up in profiles, add
// an mtime-keyed cache here (key = canonicalized working_dir, value =
// (entry mtime, Vec<InstructionFile>)).

/// Loaded instruction file ready to render.
#[derive(Debug, Clone)]
pub(super) struct InstructionFile {
    /// Canonicalized absolute path used in the section header. Falls back
    /// to the original path string when canonicalization fails.
    pub abs_path: String,
    /// Short label shown in the section header (e.g. `AGENTS.md` or
    /// `OTHER.md (referenced from AGENTS.md)`).
    pub display_label: String,
    /// File contents after truncation to [`MAX_FILE_CHARS`].
    pub content: String,
}

/// Discover and load instruction files for the session's working directory.
/// Returns an empty vec when the directory is invalid or no entry file
/// exists.
///
// TODO(working-dir-instructions): support ancestor traversal up to git root
// or home directory (see plan TODO #1).
pub(super) fn collect_working_dir_instructions(working_dir: &str) -> Vec<InstructionFile> {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let dir = Path::new(trimmed);
    if !dir.is_dir() {
        return Vec::new();
    }
    let entry = match find_entry(dir) {
        Some(p) => p,
        None => return Vec::new(),
    };
    expand(entry)
}

fn find_entry(dir: &Path) -> Option<PathBuf> {
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn expand(entry: PathBuf) -> Vec<InstructionFile> {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<(PathBuf, PathBuf, usize, Option<String>)> = VecDeque::new();
    let mut result: Vec<InstructionFile> = Vec::new();

    let entry_canonical = canonicalize_or_clone(&entry);
    visited.insert(entry_canonical.clone());
    queue.push_back((entry, entry_canonical, 0, None));

    while let Some((path, canonical, depth, parent_label)) = queue.pop_front() {
        if result.len() >= MAX_INSTRUCTION_FILES {
            break;
        }

        let original = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(err) => {
                crate::app_warn!(
                    "system_prompt",
                    "working_dir_instructions",
                    "skip {}: {}",
                    path.display(),
                    err
                );
                continue;
            }
        };

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        let display_label = match &parent_label {
            None => file_name.clone(),
            Some(parent) => format!("{} (referenced from {})", file_name, parent),
        };

        result.push(InstructionFile {
            abs_path: canonical.display().to_string(),
            display_label,
            content: truncate(&original, MAX_FILE_CHARS),
        });

        if depth >= MAX_INCLUDE_DEPTH {
            continue;
        }

        let base_dir: &Path = path.parent().unwrap_or_else(|| Path::new("."));
        for include in extract_at_includes(&original, base_dir) {
            if !is_text_extension(&include) {
                continue;
            }
            // canonicalize replaces a separate `is_file` check — non-existent
            // paths return Err, dedupe filters seen entries before any read.
            let include_canonical = match std::fs::canonicalize(&include) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if !visited.insert(include_canonical.clone()) {
                continue;
            }
            queue.push_back((
                include,
                include_canonical,
                depth + 1,
                Some(file_name.clone()),
            ));
        }
    }

    result
}

fn canonicalize_or_clone(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ── @ include extraction ─────────────────────────────────────────────

/// Extract `@path` references from markdown text, skipping fenced code
/// blocks and inline code spans, then resolve each reference against
/// `base_dir`.
fn extract_at_includes(text: &str, base_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for raw in scan_at_tokens(text) {
        if let Some(resolved) = resolve_include_path(&raw, base_dir) {
            if seen.insert(resolved.clone()) {
                out.push(resolved);
            }
        }
    }
    out
}

/// Scan `text` and yield raw `@token` strings, skipping content inside
/// fenced code blocks (``` or ~~~) and inline code spans (`...`,
/// ``...``). The scanner only matches `@` when preceded by start-of-text
/// or whitespace, mirroring the regex `(?:^|\s)@…` used by claude-code.
fn scan_at_tokens(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut in_fence: Option<u8> = None;
    let mut at_line_start = true;

    while i < len {
        let c = bytes[i];

        // Fence open / close detection at line start.
        if at_line_start && (c == b'`' || c == b'~') && line_has_fence(bytes, i, c) {
            match in_fence {
                Some(fc) if fc == c => in_fence = None,
                None => in_fence = Some(c),
                _ => {} // inside fence of other char — keep skipping
            }
            i = skip_to_eol(bytes, i);
            // skip_to_eol leaves i at '\n' or len. Move past '\n' so the next
            // iteration's `at_line_start` is true.
            if i < len && bytes[i] == b'\n' {
                i += 1;
            }
            at_line_start = true;
            continue;
        }

        if in_fence.is_some() {
            at_line_start = c == b'\n';
            i += 1;
            continue;
        }

        // Inline code span: backtick run of length N opens; matching run of
        // same length on the same line closes. Unmatched runs are skipped
        // as plain content.
        if c == b'`' {
            let mut run_end = i;
            while run_end < len && bytes[run_end] == b'`' {
                run_end += 1;
            }
            let run_len = run_end - i;
            let mut j = run_end;
            let mut closed_at: Option<usize> = None;
            while j < len {
                if bytes[j] == b'\n' {
                    break;
                }
                if bytes[j] == b'`' {
                    let mut close_end = j;
                    while close_end < len && bytes[close_end] == b'`' {
                        close_end += 1;
                    }
                    if close_end - j == run_len {
                        closed_at = Some(close_end);
                        break;
                    }
                    j = close_end;
                    continue;
                }
                j += 1;
            }
            match closed_at {
                Some(end) => {
                    i = end;
                }
                None => {
                    i = run_end;
                }
            }
            at_line_start = false;
            continue;
        }

        if c == b'@' {
            let prev_ok = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
            if prev_ok {
                let (token, next) = read_at_path(text, i + 1);
                if !token.is_empty() {
                    tokens.push(token);
                }
                i = next;
                at_line_start = false;
                continue;
            }
        }

        at_line_start = c == b'\n';
        i += 1;
    }

    tokens
}

fn line_has_fence(bytes: &[u8], start: usize, fence_char: u8) -> bool {
    if start + 2 >= bytes.len() {
        return false;
    }
    bytes[start] == fence_char && bytes[start + 1] == fence_char && bytes[start + 2] == fence_char
}

fn skip_to_eol(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

/// Read the path part of an `@token` starting at byte offset `start`.
/// Returns the decoded path (with `\ ` unescaped) and the byte index
/// immediately after the token.
///
/// All terminator bytes (space, tab, CR, LF, `\\`) are ASCII, so the byte
/// scan never lands inside a multi-byte UTF-8 codepoint — `&text[start..end]`
/// is always a valid `&str` slice, even for paths with non-ASCII chars.
fn read_at_path(text: &str, start: usize) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() {
        let ch = bytes[end];
        if ch == b'\\' && end + 1 < bytes.len() && bytes[end + 1] == b' ' {
            end += 2;
            continue;
        }
        if matches!(ch, b' ' | b'\t' | b'\n' | b'\r') {
            break;
        }
        end += 1;
    }
    let raw = &text[start..end];
    let unescaped = if raw.contains("\\ ") {
        raw.replace("\\ ", " ")
    } else {
        raw.to_string()
    };
    (unescaped, end)
}

fn resolve_include_path(raw: &str, base_dir: &Path) -> Option<PathBuf> {
    let path = match raw.find('#') {
        Some(idx) => &raw[..idx],
        None => raw,
    };
    if path.is_empty() {
        return None;
    }
    if path.starts_with('@') {
        return None;
    }
    let first = path.chars().next()?;
    let valid = path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with("~/")
        || path.starts_with('/')
        || first.is_ascii_alphanumeric()
        || first == '_'
        || first == '.';
    if !valid {
        return None;
    }

    let expanded: PathBuf = if let Some(rest) = path.strip_prefix("~/") {
        match dirs::home_dir() {
            Some(home) => home.join(rest),
            None => return None,
        }
    } else if path.starts_with('/') {
        PathBuf::from(path)
    } else if let Some(rest) = path.strip_prefix("./") {
        base_dir.join(rest)
    } else {
        base_dir.join(path)
    };

    Some(expanded)
}

const ALLOWED_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "text", "json", "yaml", "yml", "toml",
];

fn is_text_extension(p: &Path) -> bool {
    match p.extension() {
        Some(ext) => {
            let ext_lower = ext.to_string_lossy().to_lowercase();
            ALLOWED_EXTENSIONS
                .iter()
                .any(|allowed| *allowed == ext_lower)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn empty_or_invalid_dir_yields_nothing() {
        assert!(collect_working_dir_instructions("").is_empty());
        assert!(collect_working_dir_instructions("   ").is_empty());
        assert!(
            collect_working_dir_instructions("/tmp/__definitely_does_not_exist_abc123__")
                .is_empty()
        );
    }

    #[test]
    fn picks_agents_md_first_then_claude_md() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        // No file → empty.
        assert!(collect_working_dir_instructions(path.to_str().unwrap()).is_empty());

        // CLAUDE.md only.
        write(path, "CLAUDE.md", "fallback content");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 1);
        assert!(r[0].display_label.starts_with("CLAUDE.md"));
        assert!(r[0].content.contains("fallback"));

        // AGENTS.md added → preferred.
        write(path, "AGENTS.md", "primary content");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 1);
        assert!(r[0].display_label.starts_with("AGENTS.md"));
        assert!(r[0].content.contains("primary"));
    }

    #[test]
    fn expands_relative_at_reference() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "see @./CONVENTIONS.md for rules\n");
        write(path, "CONVENTIONS.md", "always run `cargo fmt`");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].display_label, "AGENTS.md");
        assert!(r[1].display_label.starts_with("CONVENTIONS.md"));
        assert!(r[1].display_label.contains("AGENTS.md"));
    }

    #[test]
    fn expands_bare_relative_at_reference() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "@OTHER.md\n");
        write(path, "OTHER.md", "x");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].display_label, "OTHER.md (referenced from AGENTS.md)");
    }

    #[test]
    fn skips_at_inside_fenced_code_block() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(
            path,
            "AGENTS.md",
            "intro\n```\nignore @./HIDDEN.md inside fence\n```\nreal @./SHOWN.md\n",
        );
        write(path, "HIDDEN.md", "hidden");
        write(path, "SHOWN.md", "shown");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        let names: Vec<_> = r.iter().map(|f| f.display_label.clone()).collect();
        assert_eq!(r.len(), 2);
        assert!(names.iter().any(|n| n.starts_with("SHOWN.md")));
        assert!(!names.iter().any(|n| n.contains("HIDDEN.md")));
    }

    #[test]
    fn skips_at_inside_inline_code_span() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(
            path,
            "AGENTS.md",
            "use `@./HIDDEN.md` syntax to reference @./SHOWN.md actually.\n",
        );
        write(path, "HIDDEN.md", "h");
        write(path, "SHOWN.md", "s");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        let names: Vec<_> = r.iter().map(|f| f.display_label.clone()).collect();
        assert!(names.iter().any(|n| n.starts_with("SHOWN.md")));
        assert!(!names.iter().any(|n| n.contains("HIDDEN.md")));
    }

    #[test]
    fn strips_fragment_anchor() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "see @./OTHER.md#section-1 please");
        write(path, "OTHER.md", "y");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn cycle_does_not_loop() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "@./B.md");
        write(path, "B.md", "@./AGENTS.md");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn deep_chain_caps_at_max_depth() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        // Chain: AGENTS.md → 1.md → 2.md → 3.md → 4.md → 5.md → 6.md
        // depth 0       1       2       3       4       5       6
        // MAX_INCLUDE_DEPTH = 5, so depth 6 is not loaded.
        write(path, "AGENTS.md", "@./1.md");
        for i in 1..=6u32 {
            let next = if i < 6 {
                format!("@./{}.md", i + 1)
            } else {
                "leaf".to_string()
            };
            write(path, &format!("{}.md", i), &next);
        }
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 6, "AGENTS + 1..=5, 6.md should be skipped");
        let last = r.last().unwrap();
        assert!(last.display_label.starts_with("5.md"));
    }

    #[test]
    fn rejects_non_text_extension() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "logo: @./logo.png\nrules: @./EXTRA.md");
        write(path, "logo.png", "fakebinary");
        write(path, "EXTRA.md", "rules");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        let names: Vec<_> = r.iter().map(|f| f.display_label.clone()).collect();
        assert_eq!(r.len(), 2);
        assert!(names.iter().any(|n| n.starts_with("EXTRA.md")));
        assert!(!names.iter().any(|n| n.contains("logo.png")));
    }

    #[test]
    fn truncates_oversized_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        let big = "x".repeat(MAX_FILE_CHARS * 2);
        write(path, "AGENTS.md", &big);
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 1);
        assert!(r[0].content.contains("[... truncated"));
    }

    #[test]
    fn caps_total_files() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        // AGENTS.md fans out to 20 sibling files. MAX_INSTRUCTION_FILES = 16.
        let mut body = String::new();
        for i in 0..20 {
            body.push_str(&format!("@./f{}.md ", i));
            write(path, &format!("f{}.md", i), &format!("file {}", i));
        }
        write(path, "AGENTS.md", &body);
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert!(r.len() <= MAX_INSTRUCTION_FILES);
        assert!(r.len() >= 2, "should load AGENTS.md plus some fan-outs");
    }

    #[test]
    fn rejects_email_like_at_token() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "ping me at user@example.com");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        // Only AGENTS.md itself, no spurious include.
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn at_with_escaped_space() {
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "see @./My\\ Notes.md");
        write(path, "My Notes.md", "n");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn at_with_multibyte_unicode_filename() {
        // Regression: byte-by-byte path scanner would split multi-byte UTF-8
        // codepoints and produce a String that doesn't match the on-disk file.
        let tmp = tempdir().unwrap();
        let path = tmp.path();
        write(path, "AGENTS.md", "见 @./约定.md\n");
        write(path, "约定.md", "约定内容");
        let r = collect_working_dir_instructions(path.to_str().unwrap());
        assert_eq!(r.len(), 2);
        assert!(r[1].display_label.starts_with("约定.md"));
        assert!(r[1].content.contains("约定内容"));
    }
}
