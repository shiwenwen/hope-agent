//! Filesystem listing & search primitives shared by the desktop (Tauri) and
//! HTTP (axum) shells. Lives in ha-core so both runtimes can reuse the same
//! validation, walk, and scoring logic — no Tauri or axum types leak in.
//!
//! Two public entry points:
//! - `list_dir(path)` — single-level read of a directory, used by the
//!   server-mode directory picker and the `@` chat-mention popper when the
//!   token contains a `/`.
//! - `create_dir(path)` — create a user-selected absolute directory for the
//!   directory picker, then return the created directory listing.
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

// ---- Project file-browser API (workspace-scoped CRUD) ----------------------

mod git;
pub(crate) use git::isolate_repository_env;
mod ops;
mod workspace;

pub use git::{git_info, GitInfo, WorktreeInfo};
pub use ops::{
    extract_abs, project_delete, project_fs_extract, project_list_dir, project_mkdir,
    project_read_text, project_rename, project_upload, project_write_text, read_text_abs,
    ExtractedContent, FileTextContent, RenameResult, UploadResult, WorkspaceEntry,
    WorkspaceListing, WriteResult,
};
pub use workspace::WorkspaceScope;

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

/// Create an absolute directory and return its listing.
///
/// This is intentionally absolute-path only because it backs owner-facing
/// directory pickers. Workspace-relative project file operations use
/// [`project_mkdir`] instead.
pub fn create_dir(requested: &str) -> Result<DirListing> {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return Err(FilesystemError::bad_input("directory path is empty"));
    }
    let target = Path::new(trimmed);
    if !target.is_absolute() {
        return Err(FilesystemError::bad_input(format!(
            "path must be absolute: {}",
            trimmed
        )));
    }

    std::fs::create_dir_all(target).map_err(|e| {
        FilesystemError::bad_input(format!("cannot create directory '{}': {}", trimmed, e))
    })?;
    let canon = target.canonicalize().map_err(|e| {
        FilesystemError::bad_input(format!("cannot resolve path '{}': {}", trimmed, e))
    })?;
    if !canon.is_dir() {
        return Err(FilesystemError::bad_input(format!(
            "path is not a directory: {}",
            canon.display()
        )));
    }
    let canon_str = canon.to_string_lossy().to_string();
    list_dir(Some(&canon_str))
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
/// - `query` is matched with a v2 path-aware scorer: exact/prefix/substring
///   matches win first, multi-token queries can match across path segments,
///   camelCase / kebab / snake / dotted words are tokenized, and subsequence
///   matching remains as the fallback. Results are sorted by score desc, path asc.
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

    let query = FileSearchQuery::new(trimmed_query);
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

        let Some(score) = best_score(&query, &name, &rel_str, is_dir) else {
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

#[derive(Debug)]
struct FileSearchQuery {
    raw_lower: String,
    raw_chars: Vec<char>,
    raw_is_ascii: bool,
    tokens: Vec<String>,
}

impl FileSearchQuery {
    fn new(raw: &str) -> Self {
        let raw_lower = raw.to_lowercase();
        Self {
            raw_chars: raw_lower.chars().collect(),
            raw_is_ascii: raw_lower.is_ascii(),
            tokens: tokenize_search_text(raw),
            raw_lower,
        }
    }
}

#[derive(Debug)]
struct FileSearchCandidate {
    name_lower: String,
    stem_lower: String,
    rel_lower: String,
    name_tokens: Vec<String>,
    rel_tokens: Vec<String>,
    depth: i32,
}

impl FileSearchCandidate {
    fn new(name: &str, rel_path: &str) -> Self {
        let name_lower = name.to_lowercase();
        let stem_lower = Path::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| name_lower.clone());
        Self {
            name_lower,
            stem_lower,
            rel_lower: rel_path.to_lowercase(),
            name_tokens: tokenize_search_text(name),
            rel_tokens: tokenize_search_text(rel_path),
            depth: rel_path.matches('/').count() as i32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TokenMatch {
    score: i32,
    in_name: bool,
    rel_pos: i32,
}

fn best_score(query: &FileSearchQuery, name: &str, rel_path: &str, is_dir: bool) -> Option<i32> {
    let candidate = FileSearchCandidate::new(name, rel_path);
    let mut best = raw_score(query, &candidate);

    if let Some(score) = token_score(query, &candidate) {
        best = Some(best.map_or(score, |prev| prev.max(score)));
    }

    if let Some(score) = subsequence_score(query, &candidate) {
        best = Some(best.map_or(score, |prev| prev.max(score)));
    }

    best.map(|score| {
        let dir_bonus = if is_dir { 30 } else { 0 };
        score + dir_bonus - candidate.depth * 8 - (candidate.rel_lower.len() as i32 / 80)
    })
}

fn raw_score(query: &FileSearchQuery, candidate: &FileSearchCandidate) -> Option<i32> {
    let q = query.raw_lower.as_str();
    let mut best: Option<i32> = None;

    let mut record = |score: i32| {
        best = Some(best.map_or(score, |prev| prev.max(score)));
    };

    if candidate.name_lower == q {
        record(24_000);
    }
    if candidate.stem_lower == q {
        record(23_500);
    }
    if candidate.name_lower.starts_with(q) {
        record(22_000 - length_gap(&candidate.name_lower, q) * 3);
    }
    if candidate.name_tokens.iter().any(|token| token == q) {
        record(21_500);
    }
    if candidate
        .name_tokens
        .iter()
        .any(|token| token.starts_with(q))
    {
        record(20_500);
    }
    if let Some(pos) = candidate.name_lower.find(q) {
        record(18_000 - pos as i32 * 4);
    }
    if let Some(pos) = candidate.rel_lower.find(q) {
        record(14_000 - pos as i32 * 2);
    }

    best
}

fn token_score(query: &FileSearchQuery, candidate: &FileSearchCandidate) -> Option<i32> {
    if query.tokens.is_empty() {
        return None;
    }

    let mut total = 10_000;
    let mut in_name_count = 0usize;
    let mut positions: Vec<i32> = Vec::with_capacity(query.tokens.len());

    for token in &query.tokens {
        let matched = best_token_match(token, candidate)?;
        total += matched.score;
        if matched.in_name {
            in_name_count += 1;
        }
        positions.push(matched.rel_pos);
    }

    if in_name_count == query.tokens.len() {
        total += 1_000;
    } else if in_name_count > 0 {
        total += 350;
    }

    if query.tokens.len() > 1 {
        if positions.windows(2).all(|pair| pair[0] <= pair[1]) {
            total += 450;
        }
        if let (Some(min), Some(max)) = (positions.iter().min(), positions.iter().max()) {
            total -= ((*max - *min) / 8).min(500);
        }
    }

    Some(total)
}

fn best_token_match(token: &str, candidate: &FileSearchCandidate) -> Option<TokenMatch> {
    if token.is_empty() {
        return None;
    }

    let mut best: Option<TokenMatch> = None;
    let mut record = |score: i32, in_name: bool, rel_pos: i32| {
        let next = TokenMatch {
            score,
            in_name,
            rel_pos,
        };
        if best.map_or(true, |prev| next.score > prev.score) {
            best = Some(next);
        }
    };

    let name_pos = candidate.name_lower.find(token).map(|p| p as i32);
    let rel_pos = candidate.rel_lower.find(token).map(|p| p as i32);

    if candidate.name_lower == token {
        record(1_500, true, rel_pos.unwrap_or(0));
    }
    if candidate.stem_lower == token {
        record(1_450, true, rel_pos.unwrap_or(0));
    }
    if candidate.name_lower.starts_with(token) {
        record(
            1_300 - length_gap(&candidate.name_lower, token).min(100),
            true,
            rel_pos.unwrap_or(0),
        );
    }
    if candidate.name_tokens.iter().any(|word| word == token) {
        record(1_220, true, rel_pos.unwrap_or(0));
    }
    if candidate
        .name_tokens
        .iter()
        .any(|word| word.starts_with(token))
    {
        record(1_060, true, rel_pos.unwrap_or(0));
    }
    if let Some(pos) = name_pos {
        record(900 - pos * 3, true, rel_pos.unwrap_or(pos));
    }
    if candidate.rel_tokens.iter().any(|word| word == token) {
        record(780, false, rel_pos.unwrap_or(0));
    }
    if candidate
        .rel_tokens
        .iter()
        .any(|word| word.starts_with(token))
    {
        record(660, false, rel_pos.unwrap_or(0));
    }
    if let Some(pos) = rel_pos {
        record(520 - pos / 4, false, pos);
    }

    let token_chars: Vec<char> = token.chars().collect();
    let token_is_ascii = token.is_ascii();
    if let Some(score) = score_in(
        token,
        &token_chars,
        token_is_ascii,
        candidate.name_lower.as_str(),
    ) {
        record(320 + score / 10, true, rel_pos.unwrap_or(0));
    }
    if let Some(score) = score_in(
        token,
        &token_chars,
        token_is_ascii,
        candidate.rel_lower.as_str(),
    ) {
        record(180 + score / 20, false, rel_pos.unwrap_or(0));
    }

    best
}

fn subsequence_score(query: &FileSearchQuery, candidate: &FileSearchCandidate) -> Option<i32> {
    let name_score = score_in(
        query.raw_lower.as_str(),
        &query.raw_chars,
        query.raw_is_ascii,
        candidate.name_lower.as_str(),
    )
    .map(|s| s + 5_000);
    let path_score = score_in(
        query.raw_lower.as_str(),
        &query.raw_chars,
        query.raw_is_ascii,
        candidate.rel_lower.as_str(),
    )
    .map(|s| s + 3_000);
    match (name_score, path_score) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn tokenize_search_text(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut prev_was_lower_or_digit = false;

    for c in input.chars() {
        if !c.is_alphanumeric() {
            push_token(&mut tokens, &mut current);
            prev_was_lower_or_digit = false;
            continue;
        }

        if c.is_uppercase() && prev_was_lower_or_digit {
            push_token(&mut tokens, &mut current);
        }
        current.extend(c.to_lowercase());
        prev_was_lower_or_digit = c.is_lowercase() || c.is_ascii_digit();
    }
    push_token(&mut tokens, &mut current);

    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn length_gap(candidate: &str, query: &str) -> i32 {
    candidate
        .chars()
        .count()
        .saturating_sub(query.chars().count()) as i32
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

    #[test]
    fn search_files_matches_tokens_across_path_segments() {
        let dir = tmpdir("token-path");
        std::fs::create_dir_all(dir.join("src/components/chat/input")).unwrap();
        std::fs::create_dir_all(dir.join("src/input")).unwrap();
        std::fs::write(
            dir.join("src/components/chat/input").join("ChatInput.tsx"),
            b"x",
        )
        .unwrap();
        std::fs::write(dir.join("src/input").join("chatty.tsx"), b"y").unwrap();

        let res = search_files(dir.to_str().unwrap(), "chat input", Some(10)).unwrap();
        let paths: Vec<&str> = res.matches.iter().map(|m| m.rel_path.as_str()).collect();
        assert!(
            paths.contains(&"src/components/chat/input/ChatInput.tsx"),
            "multi-token query should match across path segments: {paths:?}"
        );
        assert_eq!(
            paths.first().copied(),
            Some("src/components/chat/input/ChatInput.tsx"),
            "ordered path/name token match should rank first: {paths:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_files_prioritizes_filename_tokens_over_directory_tokens() {
        let dir = tmpdir("token-name");
        std::fs::create_dir_all(dir.join("src/components")).unwrap();
        std::fs::create_dir_all(dir.join("docs/file/search")).unwrap();
        std::fs::write(dir.join("src/components").join("FileSearchPanel.tsx"), b"x").unwrap();
        std::fs::write(dir.join("docs/file/search").join("panel.tsx"), b"y").unwrap();

        let res = search_files(dir.to_str().unwrap(), "file search", Some(10)).unwrap();
        let paths: Vec<&str> = res.matches.iter().map(|m| m.rel_path.as_str()).collect();
        assert_eq!(
            paths.first().copied(),
            Some("src/components/FileSearchPanel.tsx"),
            "filename token matches should outrank directory-only token matches: {paths:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
