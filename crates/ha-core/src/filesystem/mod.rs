//! Filesystem listing & search primitives shared by the desktop (Tauri) and
//! HTTP (axum) shells. Lives in ha-core so both runtimes can reuse the same
//! validation, walk, and scoring logic — no Tauri or axum types leak in.
//!
//! Two public entry points:
//! - `list_dir(path)` — single-level read of a directory, used by the
//!   server-mode directory picker and the `@` chat-mention popper when the
//!   token contains a `/`.
//! - `search_files(root, query, limit)` — fuzzy walk of `root`, respecting
//!   `.gitignore` / hidden-file rules. Used by the chat-mention popper when
//!   the user typed a bare `@chat`-style token.
//!
//! The error type splits user-input failures (bad path, empty query) from
//! genuine I/O failures so the HTTP and Tauri shells can map them to 4xx vs
//! 5xx without parsing error strings.

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

// ---- Error type ------------------------------------------------------------

/// Error returned by filesystem helpers. `BadInput` maps to HTTP 400 / Tauri
/// user-friendly message; `Internal` maps to HTTP 500.
#[derive(Debug)]
pub enum FilesystemError {
    BadInput(String),
    Internal(String),
}

impl FilesystemError {
    pub fn bad_input(msg: impl Into<String>) -> Self {
        Self::BadInput(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn is_bad_input(&self) -> bool {
        matches!(self, Self::BadInput(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BadInput(m) | Self::Internal(m) => m,
        }
    }
}

impl std::fmt::Display for FilesystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for FilesystemError {}

impl From<std::io::Error> for FilesystemError {
    fn from(e: std::io::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, FilesystemError>;

// ---- DTOs ------------------------------------------------------------------

/// Cap so huge directories (`/nix/store`, populated `node_modules`, …) don't
/// balloon memory or serialize into a multi-MB JSON response the picker can't
/// render anyway.
const MAX_LIST_ENTRIES: usize = 5000;

/// Hard cap on result count returned to the UI; limit param is clamped here.
const MAX_SEARCH_RESULTS: usize = 200;

/// Hard cap on filesystem entries visited during a single search. Stops the
/// walk early on monorepo-scale roots so the UI gets *some* answer instead of
/// timing out.
const MAX_SEARCH_WALK: usize = 50_000;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    /// Absolute path of this entry.
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    /// mtime in unix millis. `None` when the platform can't report it.
    pub modified_ms: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
    /// `true` when the directory held more than `MAX_LIST_ENTRIES` children.
    pub truncated: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub name: String,
    /// Absolute path.
    pub path: String,
    /// Path relative to the search root, with `/` separator. Used directly as
    /// the chat-input mention text.
    pub rel_path: String,
    pub is_dir: bool,
    pub score: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchResponse {
    pub root: String,
    pub matches: Vec<FileMatch>,
    /// `true` when the walk hit `MAX_SEARCH_WALK` and stopped early.
    pub truncated: bool,
}

// ---- list_dir --------------------------------------------------------------

/// List one level of a directory.
///
/// - `requested` MUST be an absolute path; relative paths are rejected.
/// - `canonicalize` is applied so the returned `path` is symlink-free and the
///   UI sees a stable identity across navigations.
/// - Entries are sorted directories-first, then name ascending (case-insensitive).
/// - When `requested` is `None`, returns the platform default root.
pub fn list_dir(requested: Option<&str>) -> Result<DirListing> {
    let requested = requested.map(str::trim).filter(|s| !s.is_empty());

    let target: PathBuf = match requested {
        Some(p) => {
            let path = Path::new(p);
            if !path.is_absolute() {
                return Err(FilesystemError::bad_input(format!(
                    "path must be absolute: {}",
                    p
                )));
            }
            path.canonicalize().map_err(|e| {
                FilesystemError::bad_input(format!("cannot resolve path '{}': {}", p, e))
            })?
        }
        None => default_root(),
    };

    if !target.is_dir() {
        return Err(FilesystemError::bad_input(format!(
            "path is not a directory: {}",
            target.display()
        )));
    }

    let read_dir = std::fs::read_dir(&target).map_err(|e| {
        FilesystemError::bad_input(format!(
            "cannot read directory '{}': {}",
            target.display(),
            e
        ))
    })?;

    let target_str = target.to_string_lossy().to_string();
    let parent = target
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty() && *s != target_str);

    let mut entries: Vec<DirEntry> = Vec::new();
    let mut truncated = false;
    for entry in read_dir {
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            app_warn!(
                "filesystem",
                "list_dir",
                "skipping unreadable entry under {}",
                target.display()
            );
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let file_type = meta.file_type();
        // Resolve `is_dir` through the symlink so a symlink to a directory
        // shows up as browsable.
        let is_dir = if file_type.is_symlink() {
            std::fs::metadata(entry.path())
                .map(|m| m.is_dir())
                .unwrap_or(false)
        } else {
            file_type.is_dir()
        };
        let size = if !is_dir { Some(meta.len()) } else { None };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().to_string_lossy().to_string(),
            is_dir,
            is_symlink: file_type.is_symlink(),
            size,
            modified_ms,
        });
    }

    entries.sort_by(|a, b| match b.is_dir.cmp(&a.is_dir) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    app_info!(
        "filesystem",
        "list_dir",
        "path={} entries={} truncated={}",
        target.display(),
        entries.len(),
        truncated
    );

    Ok(DirListing {
        path: target_str,
        parent,
        entries,
        truncated,
    })
}

#[cfg(unix)]
fn default_root() -> PathBuf {
    PathBuf::from("/")
}

#[cfg(windows)]
fn default_root() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

// ---- search_files ----------------------------------------------------------

/// Fuzzy search files & directories under `root`.
///
/// - `root` MUST be absolute; relative paths are rejected.
/// - Walks honor `.gitignore` / `.git/info/exclude` / `.ignore` (`ignore`
///   crate defaults), and skip hidden files (Unix dot-prefix).
/// - `query` is matched as a case-insensitive subsequence against entry name
///   first, then against the relative path. Tighter spans and earlier matches
///   score higher; results are sorted by score desc, path asc.
/// - `limit` defaults to 50 and is clamped to `MAX_SEARCH_RESULTS`.
pub fn search_files(root: &str, query: &str, limit: Option<usize>) -> Result<FileSearchResponse> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return Err(FilesystemError::bad_input(
            "query must not be empty".to_string(),
        ));
    }

    let root_path = Path::new(root);
    if !root_path.is_absolute() {
        return Err(FilesystemError::bad_input(format!(
            "root must be absolute: {}",
            root
        )));
    }
    let canon = root_path.canonicalize().map_err(|e| {
        FilesystemError::bad_input(format!("cannot resolve root '{}': {}", root, e))
    })?;
    if !canon.is_dir() {
        return Err(FilesystemError::bad_input(format!(
            "root is not a directory: {}",
            canon.display()
        )));
    }

    let limit = limit.unwrap_or(50).min(MAX_SEARCH_RESULTS);
    // `WalkBuilder` defaults: respect .gitignore / .ignore / .git/info/exclude,
    // skip hidden, follow_links=false. Exactly what we want.
    let walker = WalkBuilder::new(&canon).build();

    let q_lower = trimmed_query.to_lowercase();
    let q_chars: Vec<char> = q_lower.chars().collect();
    let q_is_ascii = q_lower.is_ascii();
    let mut scored: Vec<FileMatch> = Vec::new();
    let mut visited = 0usize;
    let mut truncated = false;

    for result in walker {
        if visited >= MAX_SEARCH_WALK {
            truncated = true;
            break;
        }
        let Ok(entry) = result else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        visited += 1;

        let path = entry.path();
        let Ok(rel) = path.strip_prefix(&canon) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        let Some(score) = best_score(&q_lower, &q_chars, q_is_ascii, &name, &rel_str) else {
            continue;
        };

        scored.push(FileMatch {
            name,
            path: path.to_string_lossy().to_string(),
            rel_path: rel_str,
            is_dir,
            score,
        });
    }

    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    scored.truncate(limit);

    app_info!(
        "filesystem",
        "search_files",
        "root={} q='{}' visited={} returned={} truncated={}",
        canon.display(),
        trimmed_query,
        visited,
        scored.len(),
        truncated
    );

    Ok(FileSearchResponse {
        root: canon.to_string_lossy().to_string(),
        matches: scored,
        truncated,
    })
}

/// Pick the better of a name-match and a path-match. Name matches get +1000
/// over path matches so "chat-engine.md" beats ".../engine/chat.rs" for "chat".
fn best_score(
    q_lower: &str,
    q_chars: &[char],
    q_is_ascii: bool,
    name: &str,
    rel_path: &str,
) -> Option<i32> {
    let name_score = score_in(q_lower, q_chars, q_is_ascii, name).map(|s| s + 1000);
    let path_score = score_in(q_lower, q_chars, q_is_ascii, rel_path).map(|s| s + 200);
    match (name_score, path_score) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Case-insensitive subsequence match. Returns `None` if `query` is not a
/// subsequence of `hay`; higher score = closer to start + tighter span.
///
/// ASCII fast path (q_is_ascii && hay.is_ascii()) iterates bytes and never
/// allocates a lowercase haystack — the dominant case for source-tree paths.
/// Non-ASCII falls back to char iteration with a per-call `to_lowercase()`.
fn score_in(q_lower: &str, q_chars: &[char], q_is_ascii: bool, hay: &str) -> Option<i32> {
    if q_chars.is_empty() {
        return None;
    }
    let q_len = q_chars.len() as i32;

    let (first, last) = if q_is_ascii && hay.is_ascii() {
        let q_bytes = q_lower.as_bytes();
        let mut qi = 0usize;
        let mut first_match: Option<usize> = None;
        let mut last_match: Option<usize> = None;
        for (hi, b) in hay.bytes().enumerate() {
            if qi < q_bytes.len() && b.to_ascii_lowercase() == q_bytes[qi] {
                if first_match.is_none() {
                    first_match = Some(hi);
                }
                last_match = Some(hi);
                qi += 1;
            }
        }
        if qi != q_bytes.len() {
            return None;
        }
        (first_match.unwrap() as i32, last_match.unwrap() as i32)
    } else {
        let hay_lower = hay.to_lowercase();
        let mut qi = 0usize;
        let mut first_match: Option<usize> = None;
        let mut last_match: Option<usize> = None;
        for (hi, c) in hay_lower.chars().enumerate() {
            if qi < q_chars.len() && c == q_chars[qi] {
                if first_match.is_none() {
                    first_match = Some(hi);
                }
                last_match = Some(hi);
                qi += 1;
            }
        }
        if qi != q_chars.len() {
            return None;
        }
        (first_match.unwrap() as i32, last_match.unwrap() as i32)
    };

    let span = last - first + 1;
    Some(500 - first * 2 - (span - q_len) * 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ha-core-fs-{}-{}-{}",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn list_dir_returns_entries_sorted_dirs_first() {
        let dir = tmpdir("list");
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::fs::create_dir_all(dir.join("zz_sub")).unwrap();

        let res = list_dir(Some(dir.to_str().unwrap())).unwrap();
        let names: Vec<&str> = res.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"zz_sub"));
        let dir_pos = res.entries.iter().position(|e| e.name == "zz_sub").unwrap();
        let file_pos = res.entries.iter().position(|e| e.name == "a.txt").unwrap();
        assert!(
            dir_pos < file_pos,
            "directories should come before files even if name sorts later"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_dir_rejects_relative_path() {
        match list_dir(Some("relative/path")) {
            Err(FilesystemError::BadInput(_)) => {}
            other => panic!("expected BadInput, got {:?}", other),
        }
    }

    #[test]
    fn list_dir_rejects_non_directory() {
        let dir = tmpdir("not-dir");
        let file = dir.join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        match list_dir(Some(file.to_str().unwrap())) {
            Err(FilesystemError::BadInput(_)) => {}
            other => panic!("expected BadInput, got {:?}", other),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_files_finds_by_name() {
        let dir = tmpdir("search");
        std::fs::write(dir.join("hello_world.rs"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("nested.txt"), b"y").unwrap();

        let res = search_files(dir.to_str().unwrap(), "hello", Some(50)).unwrap();
        assert!(
            res.matches.iter().any(|m| m.name == "hello_world.rs"),
            "should match hello in name"
        );

        let res2 = search_files(dir.to_str().unwrap(), "nested", Some(50)).unwrap();
        assert!(
            res2.matches
                .iter()
                .any(|m| m.rel_path.ends_with("nested.txt")),
            "should descend into sub/"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_files_rejects_empty_query() {
        let dir = tmpdir("search-empty");
        match search_files(dir.to_str().unwrap(), "  ", Some(10)) {
            Err(FilesystemError::BadInput(_)) => {}
            other => panic!("expected BadInput, got {:?}", other),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_files_respects_gitignore() {
        let dir = tmpdir("gitignore");
        // `ignore` activates .gitignore handling when there's a `.git` marker
        // or an explicit `.ignore` file. Use `.ignore` so we don't need a real repo.
        std::fs::write(dir.join(".ignore"), b"node_modules/\n").unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules").join("skip_me.txt"), b"x").unwrap();
        std::fs::write(dir.join("keep_me.txt"), b"y").unwrap();

        let res = search_files(dir.to_str().unwrap(), "skip", Some(50)).unwrap();
        assert!(
            res.matches
                .iter()
                .all(|m| !m.rel_path.contains("node_modules")),
            "node_modules should be excluded by .ignore: got {:?}",
            res.matches.iter().map(|m| &m.rel_path).collect::<Vec<_>>()
        );

        let res2 = search_files(dir.to_str().unwrap(), "keep", Some(50)).unwrap();
        assert!(
            res2.matches.iter().any(|m| m.rel_path == "keep_me.txt"),
            "non-ignored file should still be found"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_files_scores_name_match_higher_than_path_match() {
        let dir = tmpdir("score");
        std::fs::create_dir_all(dir.join("foo")).unwrap();
        std::fs::write(dir.join("foo").join("bar.txt"), b"x").unwrap();
        std::fs::write(dir.join("foo.txt"), b"y").unwrap();

        let res = search_files(dir.to_str().unwrap(), "foo", Some(10)).unwrap();
        assert!(!res.matches.is_empty());
        // Top result should be foo.txt (name match) ahead of foo/bar.txt (path match).
        let top = &res.matches[0];
        assert!(
            top.name == "foo.txt" || top.name == "foo",
            "top result should be name-match, got {:?}",
            top
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
