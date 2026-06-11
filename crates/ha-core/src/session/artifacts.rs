//! Read-time aggregation of a session's "workspace artifacts" — the files it
//! touched (read / modified) and the URL sources it referenced — by scanning
//! the FULL persisted message history (`tool_metadata` + tool results +
//! assistant text). This is the backend half of the workspace panel's hybrid
//! data model: it returns the complete set over the whole session (the
//! frontend's in-memory message list is only a paginated window), while the
//! frontend still merges a live tail from the in-memory current turn.
//!
//! IMPORTANT: the dedup + ordering rules here MUST stay in lock-step with the
//! TypeScript live-tail aggregators they mirror:
//!   - files   ↔ `src/components/chat/workspace/useSessionFileChanges.ts`
//!   - sources ↔ `src/components/chat/workspace/useSessionUrlSources.ts`
//! The two run on different data (full history here, loaded window there) and
//! their outputs are merged by key (file = `path`, source = `url`), so a
//! divergence would surface as duplicate or mis-ordered rows. Change one,
//! change both.

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::session::{MessageRole, SessionDB, SessionMessage};

/// Per-kind cap. A session touching more than this many distinct files / URLs
/// returns only the most-recent `MAX` with `*_truncated = true` so the UI can
/// say so (no silent truncation). Bounds the payload for pathological runs.
const MAX_ARTIFACTS_PER_KIND: usize = 1000;

/// One file the session touched. Summary only — the heavy `before`/`after`
/// diff snapshots in `tool_metadata` are intentionally omitted (they would
/// bloat the payload). Mirrors the frontend `SessionFileEntry` minus its
/// `diff` field; the frontend maps this back with `diff: null`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileArtifact {
    pub path: String,
    /// `"modified"` | `"read"`.
    pub kind: String,
    pub lines_added: i64,
    pub lines_removed: i64,
    /// Line count for read-only files; `None` for modified files.
    pub read_lines: Option<i64>,
}

/// One URL the session referenced.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UrlSource {
    pub url: String,
    /// `"web_search"` | `"message"` (the literal frontend `UrlSourceOrigin`).
    pub origin: String,
}

/// Aggregated artifacts for a whole session. `*_truncated` flags whether the
/// corresponding list was capped at [`MAX_ARTIFACTS_PER_KIND`].
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionArtifacts {
    pub files: Vec<FileArtifact>,
    pub sources: Vec<UrlSource>,
    pub files_truncated: bool,
    pub sources_truncated: bool,
}

/// `   URL: https://…` lines inside `web_search` tool results (see the
/// assembly in `tools/web_search/mod.rs`). Capture group 1 is the URL.
static WEB_SEARCH_URL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)URL:\s*(https?://\S+)").expect("valid web_search url regex"));

/// Bare http(s) URLs in assistant prose. Mirrors `src/lib/urlDetect.ts`.
static URL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)https?://[^\s<>"')\]]+"#).expect("valid url regex"));

/// Private / loopback hosts dropped from prose URLs. Mirrors
/// `PRIVATE_HOST_PATTERNS` in `src/lib/urlDetect.ts`.
static PRIVATE_HOST_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(?:localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|10\.\d+\.\d+\.\d+|172\.(?:1[6-9]|2\d|3[01])\.\d+\.\d+|192\.168\.\d+\.\d+|\[::1\])$",
    )
    .expect("valid private-host regex")
});

/// Asset extensions dropped from prose URLs. Mirrors `SKIP_EXTENSIONS` in
/// `src/lib/urlDetect.ts` — keep in sync.
const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp", "mp4", "webm", "mov", "avi", "mp3",
    "wav", "ogg", "flac", "zip", "tar", "gz", "rar", "7z", "pdf", "doc", "docx", "xls", "xlsx",
    "ppt", "pptx", "exe", "dmg", "iso",
];

/// Mirror of `shouldSkipUrl` in `src/lib/urlDetect.ts`: drop private/loopback
/// hosts + asset-extension URLs. Applied ONLY to assistant-prose URLs — the
/// `web_search` path is unfiltered, matching the TS pipeline where only
/// `extractUrls` runs this filter. Unparseable URLs are skipped (TS `catch`).
fn should_skip_message_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return true;
    };
    if PRIVATE_HOST_RE.is_match(parsed.host_str().unwrap_or("")) {
        return true;
    }
    let path = parsed.path().to_lowercase();
    if let Some((_, ext)) = path.rsplit_once('.') {
        if SKIP_EXTENSIONS.contains(&ext) {
            return true;
        }
    }
    false
}

/// Aggregate a session's files + URL sources from its full message history.
pub fn aggregate_session_artifacts(db: &SessionDB, session_id: &str) -> Result<SessionArtifacts> {
    let messages = db.load_session_messages(session_id)?;
    let (files, files_truncated) = aggregate_files(&messages);
    let (sources, sources_truncated) = aggregate_sources(&messages);
    Ok(SessionArtifacts {
        files,
        sources,
        files_truncated,
        sources_truncated,
    })
}

/// Internal accumulator: a file summary plus the sequence number of its most
/// recent touch (higher = more recently touched).
struct FileAgg {
    art: FileArtifact,
    order: u64,
}

/// Upsert a `file_change`-shaped JSON object as a modified file (latest diff
/// wins; recency bumped). Mirrors `upsertWrite` in the TS aggregator.
fn upsert_write(map: &mut HashMap<String, FileAgg>, seq: &mut u64, c: &Value) {
    let Some(path) = c.get("path").and_then(Value::as_str) else {
        return;
    };
    let art = FileArtifact {
        path: path.to_string(),
        kind: "modified".to_string(),
        lines_added: c.get("linesAdded").and_then(Value::as_i64).unwrap_or(0),
        lines_removed: c.get("linesRemoved").and_then(Value::as_i64).unwrap_or(0),
        read_lines: None,
    };
    map.insert(path.to_string(), FileAgg { art, order: *seq });
    *seq += 1;
}

/// Upsert a tool-produced file (send_attachment / image_generate / exec via the
/// `__MEDIA_ITEMS__` header) as a modified artifact. These never go through
/// write/edit so they carry no diff metadata, but they're still session output.
/// An existing entry (e.g. a richer `write` diff) is kept untouched — NOT even a
/// recency bump — to mirror the frontend live tail's `if (!entries.has(path))`
/// guard in `useSessionFileChanges.ts`. The two aggregators must stay in lockstep
/// (AGENTS red line: change one, change both).
fn upsert_media(map: &mut HashMap<String, FileAgg>, seq: &mut u64, path: &str) {
    if map.contains_key(path) {
        return;
    }
    map.insert(
        path.to_string(),
        FileAgg {
            art: FileArtifact {
                path: path.to_string(),
                kind: "modified".to_string(),
                lines_added: 0,
                lines_removed: 0,
                read_lines: None,
            },
            order: *seq,
        },
    );
    *seq += 1;
}

/// Files the session read / modified, most-recently-touched first. Reads
/// structured `tool_metadata` only (no legacy content-block fallback — those
/// pre-metadata rows are covered by the frontend live tail within the loaded
/// window; older ones are a known gap).
fn aggregate_files(messages: &[SessionMessage]) -> (Vec<FileArtifact>, bool) {
    let mut map: HashMap<String, FileAgg> = HashMap::new();
    let mut seq: u64 = 0;

    // Single interleaved pass per message — structured file metadata first, then
    // that same message's tool-produced media — to mirror the frontend live
    // tail's per-tool ordering in `useSessionFileChanges.ts` (AGENTS red line:
    // the two aggregators must stay in lockstep).
    for msg in messages {
        // 1) Structured file metadata (write / edit / apply_patch / read).
        if let Some(meta) = msg
            .tool_metadata
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
        {
            match meta.get("kind").and_then(Value::as_str) {
                Some("file_change") => upsert_write(&mut map, &mut seq, &meta),
                Some("file_changes") => {
                    if let Some(changes) = meta.get("changes").and_then(Value::as_array) {
                        for c in changes {
                            upsert_write(&mut map, &mut seq, c);
                        }
                    }
                }
                Some("file_read") => {
                    if let Some(path) = meta.get("path").and_then(Value::as_str) {
                        // A read after a modify keeps the file "modified" — only bump recency.
                        let bump_only = map.get(path).is_some_and(|e| e.art.kind == "modified");
                        if bump_only {
                            if let Some(existing) = map.get_mut(path) {
                                existing.order = seq;
                                seq += 1;
                            }
                        } else {
                            map.insert(
                                path.to_string(),
                                FileAgg {
                                    art: FileArtifact {
                                        path: path.to_string(),
                                        kind: "read".to_string(),
                                        lines_added: 0,
                                        lines_removed: 0,
                                        read_lines: meta.get("lines").and_then(Value::as_i64),
                                    },
                                    order: seq,
                                },
                            );
                            seq += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        // 2) Tool-produced files (send_attachment / image_generate / exec) ride a
        //    `__MEDIA_ITEMS__` header in the tool result instead of file-diff
        //    metadata — scan this message's result for their local paths.
        if let Some(result) = msg.tool_result.as_deref() {
            let (_, items) = crate::agent::extract_media_items(result);
            for item in items {
                if let Some(local_path) = item.local_path {
                    upsert_media(&mut map, &mut seq, &local_path);
                }
            }
        }
    }

    let mut entries: Vec<FileAgg> = map.into_values().collect();
    // Most-recently-touched first.
    entries.sort_by_key(|e| std::cmp::Reverse(e.order));
    let truncated = entries.len() > MAX_ARTIFACTS_PER_KIND;
    entries.truncate(MAX_ARTIFACTS_PER_KIND);
    (entries.into_iter().map(|e| e.art).collect(), truncated)
}

/// Strip trailing sentence punctuation from a URL. Mirrors the TS
/// `rawUrl.replace(/[.,;:!?)\]]+$/, "")`.
fn normalize_url(raw: &str) -> String {
    raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']'])
        .to_string()
}

fn add_url(
    seen: &mut HashSet<String>,
    sources: &mut Vec<UrlSource>,
    raw: &str,
    origin: &str,
    skip_filtered: bool,
) {
    let url = normalize_url(raw);
    if url.is_empty() || seen.contains(&url) {
        return;
    }
    // Prose URLs run the urlDetect.ts skip-filter; web_search URLs do not.
    if skip_filtered && should_skip_message_url(&url) {
        return;
    }
    seen.insert(url.clone());
    sources.push(UrlSource {
        url,
        origin: origin.to_string(),
    });
}

/// URL sources the session referenced, most-recently-introduced first. Collects
/// `web_search` result URLs (structured origin) + bare URLs in assistant /
/// intermediate text-block prose, deduped by normalized URL (first origin kept).
fn aggregate_sources(messages: &[SessionMessage]) -> (Vec<UrlSource>, bool) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut sources: Vec<UrlSource> = Vec::new(); // chronological first-occurrence

    for msg in messages {
        if msg.tool_name.as_deref() == Some("web_search") {
            if let Some(result) = msg.tool_result.as_deref() {
                for cap in WEB_SEARCH_URL_RE.captures_iter(result) {
                    if let Some(m) = cap.get(1) {
                        add_url(&mut seen, &mut sources, m.as_str(), "web_search", false);
                    }
                }
            }
        }
        // Assistant prose lives in `assistant` (final) + `text_block`
        // (intermediate, before tool calls) rows — both carry user-visible text.
        if matches!(msg.role, MessageRole::Assistant | MessageRole::TextBlock) {
            for m in URL_RE.find_iter(&msg.content) {
                add_url(&mut seen, &mut sources, m.as_str(), "message", true);
            }
        }
    }

    let truncated = sources.len() > MAX_ARTIFACTS_PER_KIND;
    // Most-recently-introduced first, then keep the most recent MAX.
    sources.reverse();
    sources.truncate(MAX_ARTIFACTS_PER_KIND);
    (sources, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `SessionMessage`; only the fields the aggregators read
    /// are meaningful, the rest are inert defaults.
    fn msg(
        id: i64,
        role: MessageRole,
        content: &str,
        tool_name: Option<&str>,
        tool_result: Option<&str>,
        tool_metadata: Option<&str>,
    ) -> SessionMessage {
        SessionMessage {
            id,
            session_id: "s".to_string(),
            role,
            content: content.to_string(),
            timestamp: String::new(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: tool_name.map(str::to_string),
            tool_arguments: None,
            tool_result: tool_result.map(str::to_string),
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: tool_metadata.map(str::to_string),
            stream_status: None,
        }
    }

    fn tool_meta(role: MessageRole, meta: &str) -> SessionMessage {
        msg(0, role, "", Some("edit"), None, Some(meta))
    }

    #[test]
    fn files_dedup_modified_beats_read_and_recency_order() {
        let messages = vec![
            tool_meta(
                MessageRole::Tool,
                r#"{"kind":"file_read","path":"/a.txt","lines":10}"#,
            ),
            tool_meta(
                MessageRole::Tool,
                r#"{"kind":"file_change","path":"/b.rs","action":"edit","linesAdded":3,"linesRemoved":1}"#,
            ),
            // Re-reading /b.rs must NOT downgrade it from modified to read.
            tool_meta(
                MessageRole::Tool,
                r#"{"kind":"file_read","path":"/b.rs","lines":20}"#,
            ),
        ];
        let (files, truncated) = aggregate_files(&messages);
        assert!(!truncated);
        // /b.rs was touched most recently (the re-read) → first.
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "/b.rs");
        assert_eq!(files[0].kind, "modified");
        assert_eq!(files[0].lines_added, 3);
        assert_eq!(files[0].lines_removed, 1);
        assert_eq!(files[0].read_lines, None);
        assert_eq!(files[1].path, "/a.txt");
        assert_eq!(files[1].kind, "read");
        assert_eq!(files[1].read_lines, Some(10));
    }

    #[test]
    fn files_changes_expands_multi_file_patch() {
        let messages = vec![tool_meta(
            MessageRole::Tool,
            r#"{"kind":"file_changes","changes":[
                {"kind":"file_change","path":"/x","action":"create","linesAdded":5,"linesRemoved":0},
                {"kind":"file_change","path":"/y","action":"delete","linesAdded":0,"linesRemoved":7}
            ]}"#,
        )];
        let (files, _) = aggregate_files(&messages);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/x"));
        assert!(paths.contains(&"/y"));
        assert!(files.iter().all(|f| f.kind == "modified"));
    }

    #[test]
    fn sources_extract_dedup_normalize_and_recency() {
        let messages = vec![
            msg(
                1,
                MessageRole::Tool,
                "",
                Some("web_search"),
                Some("1. Title\n   URL: https://example.com/a\n   Source: x\n"),
                None,
            ),
            msg(
                2,
                MessageRole::Assistant,
                "see https://example.com/b. and again https://example.com/a)",
                None,
                None,
                None,
            ),
        ];
        let (sources, truncated) = aggregate_sources(&messages);
        assert!(!truncated);
        // example.com/a is deduped (web_search origin kept, first occurrence);
        // trailing `.` / `)` stripped. Most-recently-introduced first → b first.
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].url, "https://example.com/b");
        assert_eq!(sources[0].origin, "message");
        assert_eq!(sources[1].url, "https://example.com/a");
        assert_eq!(sources[1].origin, "web_search");
    }

    #[test]
    fn sources_ignore_non_assistant_roles() {
        // A user message URL must not be collected (only assistant / text_block).
        let messages = vec![msg(
            1,
            MessageRole::User,
            "https://should-not-appear.example",
            None,
            None,
            None,
        )];
        let (sources, _) = aggregate_sources(&messages);
        assert!(sources.is_empty());
    }

    #[test]
    fn sources_prose_drops_private_hosts_and_asset_extensions() {
        // Mirrors urlDetect.ts shouldSkipUrl on the prose path.
        let messages = vec![msg(
            1,
            MessageRole::Assistant,
            "real https://example.com/page local https://localhost:3000/x asset https://cdn.example.com/pic.png",
            None,
            None,
            None,
        )];
        let (sources, _) = aggregate_sources(&messages);
        let urls: Vec<&str> = sources.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(urls, vec!["https://example.com/page"]);
    }

    #[test]
    fn sources_web_search_bypasses_skip_filter() {
        // web_search URLs are NOT run through the prose skip-filter (parity with
        // the TS pipeline, where only extractUrls filters).
        let messages = vec![msg(
            1,
            MessageRole::Tool,
            "",
            Some("web_search"),
            Some("1. Report\n   URL: https://cdn.site.com/report.pdf\n   Source: site\n"),
            None,
        )];
        let (sources, _) = aggregate_sources(&messages);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].url, "https://cdn.site.com/report.pdf");
        assert_eq!(sources[0].origin, "web_search");
    }
}
