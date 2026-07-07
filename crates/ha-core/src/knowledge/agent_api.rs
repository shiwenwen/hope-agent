//! Phase 6 external-agent API for Knowledge Space.
//!
//! This is a stable, owner-plane facade for local HTTP/Tauri/MCP-style callers.
//! It keeps compiled notes and raw sources physically and semantically separate:
//! `search` returns notes first and only includes raw source hits when the caller
//! opts in, while `sources` returns raw content only for an explicit source id.

use anyhow::{bail, Result};
use serde_json::Value;

use super::service;
use super::types::{
    CompileRun, CompileStartInput, KnowledgeAgentCompileProposeInput, KnowledgeAgentExpandInput,
    KnowledgeAgentExpandResult, KnowledgeAgentItemKind, KnowledgeAgentNoteHit,
    KnowledgeAgentReadInput, KnowledgeAgentReadResult, KnowledgeAgentSearchInput,
    KnowledgeAgentSearchResult, KnowledgeAgentSourceItem, KnowledgeAgentSourcesInput,
    KnowledgeAgentSourcesResult, KnowledgeSource, NoteReadResult, NoteSearchHit, NoteSourceRef,
};

const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;
const DEFAULT_SOURCE_LIMIT: usize = 20;
const MAX_SOURCE_LIMIT: usize = 50;
const MAX_SOURCE_SCAN: usize = 300;
const SNIPPET_CHARS: usize = 360;

pub fn search(input: KnowledgeAgentSearchInput) -> Result<KnowledgeAgentSearchResult> {
    let query = normalized_query(&input.query)?;
    let limit = clamp_limit(input.limit, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT);
    let note_hits = service::search(input.kb_id.as_deref(), &query, limit + 1)?;
    let notes_truncated = note_hits.len() > limit;
    let notes = note_hits
        .into_iter()
        .take(limit)
        .filter_map(|hit| note_hit_for_agent(hit).ok())
        .collect::<Vec<_>>();
    let (sources, sources_truncated) = if input.include_sources {
        let Some(kb_id) = input.kb_id.as_deref() else {
            bail!("includeSources requires kbId so raw sources remain explicitly scoped");
        };
        let result = sources(KnowledgeAgentSourcesInput {
            kb_id: kb_id.to_string(),
            source_id: None,
            query: Some(query),
            limit: input.limit,
            include_content: false,
        })?;
        (result.sources, result.truncated)
    } else {
        (Vec::new(), false)
    };
    Ok(KnowledgeAgentSearchResult {
        notes,
        sources,
        truncated: notes_truncated || sources_truncated,
    })
}

pub fn read(input: KnowledgeAgentReadInput) -> Result<KnowledgeAgentReadResult> {
    let include_refs = input.include_source_refs.unwrap_or(true);
    let path = clean_opt(input.path);
    let reference = clean_opt(input.reference);
    match (path, reference) {
        (Some(path), None) => read_note_for_agent(&input.kb_id, &path, include_refs),
        (None, Some(reference)) => {
            let Some(read) = service::note_read_ref(&input.kb_id, &reference)? else {
                bail!("note reference not found: {reference}");
            };
            read_result_for_agent(read, include_refs)
        }
        (Some(_), Some(_)) => bail!("knowledge.read accepts either path or reference, not both"),
        (None, None) => bail!("knowledge.read requires path or reference"),
    }
}

pub fn expand(input: KnowledgeAgentExpandInput) -> Result<KnowledgeAgentExpandResult> {
    let note = read_note_for_agent(&input.kb_id, &input.path, true)?;
    let limit = clamp_limit(input.limit, 8, 25);
    let query = related_query(&note);
    let related_notes = service::search(Some(&input.kb_id), &query, limit + 1)?
        .into_iter()
        .filter(|hit| hit.rel_path != note.rel_path)
        .take(limit)
        .filter_map(|hit| note_hit_for_agent(hit).ok())
        .collect();
    Ok(KnowledgeAgentExpandResult {
        note,
        related_notes,
    })
}

pub fn sources(input: KnowledgeAgentSourcesInput) -> Result<KnowledgeAgentSourcesResult> {
    let limit = clamp_limit(input.limit, DEFAULT_SOURCE_LIMIT, MAX_SOURCE_LIMIT);
    if let Some(source_id) = clean_opt(input.source_id) {
        let read = service::source_read(&input.kb_id, &source_id)?;
        let item = source_item(
            read.source,
            Some(&read.content),
            input.include_content,
            None,
        );
        return Ok(KnowledgeAgentSourcesResult {
            sources: vec![item],
            truncated: false,
        });
    }

    let query = clean_opt(input.query).map(|q| q.to_lowercase());
    let mut out = Vec::new();
    let all_sources = service::source_list(&input.kb_id)?;
    let scan_truncated = all_sources.len() > MAX_SOURCE_SCAN;
    let mut result_truncated = scan_truncated;
    for source in all_sources.into_iter().take(MAX_SOURCE_SCAN) {
        if out.len() >= limit {
            result_truncated = true;
            break;
        }
        let mut snippet = None;
        if let Some(q) = query.as_deref() {
            if !source_metadata_matches(&source, q) {
                let Ok(read) = service::source_read(&input.kb_id, &source.id) else {
                    continue;
                };
                let Some(s) = snippet_for_query(&read.content, q) else {
                    continue;
                };
                snippet = Some(s);
            }
        }
        out.push(source_item(source, None, false, snippet));
    }
    Ok(KnowledgeAgentSourcesResult {
        sources: out,
        truncated: result_truncated,
    })
}

pub async fn compile_propose(input: KnowledgeAgentCompileProposeInput) -> Result<CompileRun> {
    if input.source_ids.is_empty() {
        bail!("knowledge.compile.propose requires at least one source id");
    }
    service::compile_start(
        &input.kb_id,
        CompileStartInput {
            source_ids: input.source_ids,
            strategy: input.strategy,
        },
    )
    .await
}

fn read_note_for_agent(
    kb_id: &str,
    path: &str,
    include_refs: bool,
) -> Result<KnowledgeAgentReadResult> {
    let read = service::note_read(kb_id, path)?;
    read_result_for_agent(read, include_refs)
}

fn read_result_for_agent(
    read: NoteReadResult,
    include_refs: bool,
) -> Result<KnowledgeAgentReadResult> {
    let source_refs = if include_refs {
        service::note_source_refs(&read.kb_id, &read.rel_path)?
    } else {
        Vec::new()
    };
    let kind = note_kind(&read, &source_refs);
    Ok(KnowledgeAgentReadResult {
        kind,
        kb_id: read.kb_id,
        note_id: read.note_id,
        rel_path: read.rel_path,
        title: read.title,
        content: read.content,
        content_hash: read.content_hash,
        frontmatter_json: read.frontmatter_json,
        outgoing_links: read.outgoing_links,
        backlinks: read.backlinks,
        tags: read.tags,
        source_refs,
    })
}

fn note_hit_for_agent(hit: NoteSearchHit) -> Result<KnowledgeAgentNoteHit> {
    let read = service::note_read(&hit.kb_id, &hit.rel_path)?;
    let refs = service::note_source_refs(&hit.kb_id, &hit.rel_path).unwrap_or_default();
    Ok(KnowledgeAgentNoteHit {
        kind: note_kind(&read, &refs),
        kb_id: hit.kb_id,
        kb_name: hit.kb_name,
        kb_emoji: hit.kb_emoji,
        note_id: hit.note_id,
        rel_path: hit.rel_path,
        title: hit.title,
        score: hit.score,
        snippet: hit.snippet,
        heading_path: hit.heading_path,
        start_line: hit.start_line,
    })
}

fn note_kind(read: &NoteReadResult, source_refs: &[NoteSourceRef]) -> KnowledgeAgentItemKind {
    if !source_refs.is_empty() || frontmatter_marks_compiled(read.frontmatter_json.as_deref()) {
        KnowledgeAgentItemKind::CompiledNote
    } else {
        KnowledgeAgentItemKind::Note
    }
}

fn frontmatter_marks_compiled(json: Option<&str>) -> bool {
    let Some(json) = json else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return false;
    };
    let has_key = |key: &str| value.get(key).map(json_value_present).unwrap_or(false);
    if has_key("sources") || has_key("source_id") || has_key("last_compiled") {
        return true;
    }
    value
        .get("type")
        .and_then(|v| v.as_str())
        .map(|t| matches!(t, "source_summary" | "conversation_note"))
        .unwrap_or(false)
}

fn json_value_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => true,
    }
}

fn source_item(
    source: KnowledgeSource,
    content: Option<&str>,
    include_content: bool,
    snippet: Option<String>,
) -> KnowledgeAgentSourceItem {
    let stale = source
        .compiled_at
        .map(|compiled| source.updated_at > compiled)
        .unwrap_or(false)
        || source.superseded_by_source_id.is_some();
    KnowledgeAgentSourceItem {
        kind: KnowledgeAgentItemKind::Source,
        kb_id: source.kb_id,
        source_id: source.id,
        source_kind: source.kind,
        status: source.status,
        title: source.title,
        origin_uri: source.origin_uri,
        content_hash: source.content_hash,
        compiled_at: source.compiled_at,
        stale,
        created_at: source.created_at,
        updated_at: source.updated_at,
        size: source.size,
        chunk_count: source.chunk_count,
        version_of_source_id: source.version_of_source_id,
        version_index: source.version_index,
        superseded_by_source_id: source.superseded_by_source_id,
        superseded_at: source.superseded_at,
        snippet,
        content: include_content
            .then(|| content.map(|s| s.to_string()))
            .flatten(),
    }
}

fn source_metadata_matches(source: &KnowledgeSource, query: &str) -> bool {
    source.title.to_lowercase().contains(query)
        || source
            .origin_uri
            .as_deref()
            .map(|u| u.to_lowercase().contains(query))
            .unwrap_or(false)
        || source.kind.as_str().contains(query)
        || source.status.as_str().contains(query)
}

fn snippet_for_query(content: &str, query: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.to_lowercase().contains(query))
        .map(|line| crate::truncate_utf8(line, SNIPPET_CHARS).to_string())
}

fn related_query(note: &KnowledgeAgentReadResult) -> String {
    let first_body = note
        .content
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("---")
                && !line.starts_with('#')
                && !line.starts_with("source:")
        })
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    crate::truncate_utf8(&format!("{} {}", note.title, first_body), 2_000).to_string()
}

fn normalized_query(query: &str) -> Result<String> {
    let q = query.trim();
    if q.is_empty() {
        bail!("query must not be empty");
    }
    Ok(q.to_string())
}

fn clean_opt(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn clamp_limit(value: Option<u32>, default: usize, max: usize) -> usize {
    value.map(|n| n as usize).unwrap_or(default).clamp(1, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_compiled_marker_detects_source_refs() {
        assert!(frontmatter_marks_compiled(Some(
            r#"{"sources":[{"source_id":"src-a"}]}"#
        )));
        assert!(frontmatter_marks_compiled(Some(
            r#"{"type":"source_summary","confidence":"high"}"#
        )));
        assert!(frontmatter_marks_compiled(Some(
            r#"{"last_compiled":"2026-07-01T00:00:00Z"}"#
        )));
        assert!(!frontmatter_marks_compiled(Some(r#"{"title":"Plain"}"#)));
        assert!(!frontmatter_marks_compiled(None));
    }

    #[test]
    fn source_snippet_uses_matching_line_only() {
        let content = "Intro\n\nThe durable fact is here.\nAnother line";
        assert_eq!(
            snippet_for_query(content, "durable"),
            Some("The durable fact is here.".to_string())
        );
        assert_eq!(snippet_for_query(content, "missing"), None);
    }

    #[test]
    fn limit_clamps_zero_and_large_values() {
        assert_eq!(clamp_limit(None, 10, 50), 10);
        assert_eq!(clamp_limit(Some(0), 10, 50), 1);
        assert_eq!(clamp_limit(Some(500), 10, 50), 50);
    }
}
