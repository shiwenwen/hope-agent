//! Schema profile + evidence helpers for Knowledge Compiler Phase 3.
//!
//! This is an owner-plane read layer: it parses compiled notes for source
//! references and schema lint, but never writes notes or exposes raw sources to
//! agent tools.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::types::{
    NoteSourceRef, SchemaIssue, SchemaIssueKind, SchemaProfile, DEFAULT_SCHEMA_SECTIONS,
};
use super::{service, KnowledgeRegistry};

const SCAN_CAP: usize = 500;
const ISSUE_CAP: usize = 100;

fn registry() -> Result<&'static std::sync::Arc<KnowledgeRegistry>> {
    crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))
}

pub fn profile(kb_id: &str) -> Result<SchemaProfile> {
    let kb = registry()?
        .get(kb_id)?
        .ok_or_else(|| anyhow!("knowledge base not found: {kb_id}"))?;
    let now = chrono::Utc::now().timestamp_millis();
    if let Some(profile) = registry()?.get_schema_profile(&kb.id)? {
        return Ok(profile);
    }
    let profile = SchemaProfile::default_for(&kb.id, now);
    registry()?.upsert_schema_profile(&profile)?;
    Ok(profile)
}

pub fn note_source_refs(kb_id: &str, rel_path: &str) -> Result<Vec<NoteSourceRef>> {
    let note = service::note_read(kb_id, rel_path)?;
    note_source_refs_from_content(kb_id, &note.content)
}

pub fn schema_issues(kb_id: &str) -> Result<Vec<SchemaIssue>> {
    let profile = profile(kb_id)?;
    let page_types = profile
        .page_types
        .iter()
        .map(|p| p.key.as_str())
        .collect::<HashSet<_>>();
    let notes = service::list_notes(kb_id)?;
    let mut issues = Vec::new();
    let mut scanned = 0usize;
    for note in notes {
        if issues.len() >= ISSUE_CAP {
            break;
        }
        if !indexed_schema_marker(note.frontmatter_json.as_deref()) {
            continue;
        }
        if scanned >= SCAN_CAP {
            break;
        }
        scanned += 1;
        let read = match service::note_read(kb_id, &note.rel_path) {
            Ok(read) => read,
            Err(e) => {
                issues.push(SchemaIssue {
                    kb_id: kb_id.to_string(),
                    rel_path: note.rel_path.clone(),
                    title: note.title.clone(),
                    kind: SchemaIssueKind::SchemaViolation,
                    detail: format!("Could not read note for schema lint: {e}"),
                    source_ids: Vec::new(),
                });
                continue;
            }
        };
        let snapshot = inspect_note(&read.content);
        if !is_schema_lint_candidate(&snapshot) {
            continue;
        }
        let page_type = snapshot.frontmatter.get("type").map(String::as_str);
        if page_type.map(|t| !page_types.contains(t)).unwrap_or(true) {
            issues.push(SchemaIssue {
                kb_id: kb_id.to_string(),
                rel_path: note.rel_path.clone(),
                title: note.title.clone(),
                kind: SchemaIssueKind::SchemaViolation,
                detail: page_type
                    .map(|t| format!("Unknown schema type `{t}`."))
                    .unwrap_or_else(|| "Missing frontmatter `type`.".to_string()),
                source_ids: Vec::new(),
            });
        }

        let missing_sections = DEFAULT_SCHEMA_SECTIONS
            .iter()
            .filter(|section| !snapshot.sections.contains(**section))
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>();
        if !missing_sections.is_empty() {
            issues.push(SchemaIssue {
                kb_id: kb_id.to_string(),
                rel_path: note.rel_path.clone(),
                title: note.title.clone(),
                kind: SchemaIssueKind::SchemaViolation,
                detail: format!(
                    "Missing required section(s): {}.",
                    missing_sections.join(", ")
                ),
                source_ids: Vec::new(),
            });
        }

        let refs = note_source_refs_from_content(kb_id, &read.content)?;
        let source_ids = refs.iter().map(|r| r.source_id.clone()).collect::<Vec<_>>();
        if refs.is_empty() {
            issues.push(SchemaIssue {
                kb_id: kb_id.to_string(),
                rel_path: note.rel_path.clone(),
                title: note.title.clone(),
                kind: SchemaIssueKind::MissingEvidence,
                detail: "No `sources` frontmatter or `source_id` evidence reference found."
                    .to_string(),
                source_ids: Vec::new(),
            });
        }
        for r in refs.iter().filter(|r| r.missing || r.stale) {
            issues.push(SchemaIssue {
                kb_id: kb_id.to_string(),
                rel_path: note.rel_path.clone(),
                title: note.title.clone(),
                kind: SchemaIssueKind::StaleSource,
                detail: if r.missing {
                    format!("Referenced source `{}` no longer exists.", r.source_id)
                } else {
                    format!(
                        "Referenced source `{}` changed after this note was compiled.",
                        r.source_id
                    )
                },
                source_ids: vec![r.source_id.clone()],
            });
        }

        if let Some(open_questions) = section_body(&read.content, "Open Questions") {
            if has_actionable_text(open_questions) {
                issues.push(SchemaIssue {
                    kb_id: kb_id.to_string(),
                    rel_path: note.rel_path.clone(),
                    title: note.title.clone(),
                    kind: SchemaIssueKind::UnfiledOpenQuestion,
                    detail: "Open Questions contains unresolved items.".to_string(),
                    source_ids: source_ids.clone(),
                });
            }
        }

        if contains_conflict_marker(&read.content) {
            issues.push(SchemaIssue {
                kb_id: kb_id.to_string(),
                rel_path: note.rel_path,
                title: note.title,
                kind: SchemaIssueKind::ConflictingClaim,
                detail: "Potential conflict marker found in compiled note.".to_string(),
                source_ids,
            });
        }
    }
    issues.truncate(ISSUE_CAP);
    Ok(issues)
}

fn note_source_refs_from_content(kb_id: &str, content: &str) -> Result<Vec<NoteSourceRef>> {
    let snapshot = inspect_note(content);
    let mut out = Vec::new();
    for (source_id, cited_in) in snapshot.source_refs {
        let source = registry()?.get_source(kb_id, &source_id)?;
        let (title, origin_uri, source_updated_at, missing) = match source {
            Some(s) => (Some(s.title), s.origin_uri, Some(s.updated_at), false),
            None => (None, None, None, true),
        };
        let stale = source_updated_at
            .zip(snapshot.last_compiled_at)
            .map(|(source_updated, last_compiled)| source_updated > last_compiled)
            .unwrap_or(false);
        out.push(NoteSourceRef {
            source_id,
            title,
            origin_uri,
            missing,
            stale,
            source_updated_at,
            note_last_compiled_at: snapshot.last_compiled_at,
            cited_in,
        });
    }
    Ok(out)
}

fn is_schema_lint_candidate(snapshot: &NoteSchemaSnapshot) -> bool {
    snapshot.frontmatter.contains_key("type")
        || snapshot.frontmatter.contains_key("last_compiled")
        || !snapshot.source_refs.is_empty()
}

fn indexed_schema_marker(json: Option<&str>) -> bool {
    let Some(json) = json else { return false };
    serde_json::from_str::<Value>(json)
        .ok()
        .map(|value| {
            ["type", "last_compiled", "sources"].iter().any(|key| {
                value
                    .get(*key)
                    .map(json_value_is_present)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn json_value_is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        _ => true,
    }
}

#[derive(Debug, Default)]
struct NoteSchemaSnapshot {
    frontmatter: HashMap<String, String>,
    sections: HashSet<String>,
    source_refs: Vec<(String, Vec<String>)>,
    last_compiled_at: Option<i64>,
}

fn inspect_note(content: &str) -> NoteSchemaSnapshot {
    let frontmatter = frontmatter_map(content);
    let last_compiled_at = frontmatter
        .get("last_compiled")
        .and_then(|v| parse_rfc3339_ms(v));
    let sections = section_titles(content);
    let source_refs = extract_source_refs(content);
    NoteSchemaSnapshot {
        frontmatter,
        sections,
        source_refs,
        last_compiled_at,
    }
}

fn frontmatter_map(content: &str) -> HashMap<String, String> {
    let Some(fm) = frontmatter_block(content) else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for line in fm.lines() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || value.trim().is_empty() {
            continue;
        }
        map.insert(key.to_string(), clean_scalar(value));
    }
    map
}

fn frontmatter_block(content: &str) -> Option<&str> {
    let rest = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))?;
    let mut offset = content.len() - rest.len();
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            return Some(&content[content.len() - rest.len()..offset]);
        }
        offset += line.len();
    }
    None
}

fn section_titles(content: &str) -> HashSet<String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let title = trimmed.strip_prefix("## ")?;
            Some(title.trim().trim_matches('#').trim().to_string())
        })
        .collect()
}

fn section_body<'a>(content: &'a str, section: &str) -> Option<&'a str> {
    let mut start = None;
    let mut end = content.len();
    let mut pos = 0usize;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(title) = trimmed.strip_prefix("## ") {
            let title = title.trim().trim_matches('#').trim();
            if title == section {
                start = Some(pos + line.len());
            } else if start.is_some() {
                end = pos;
                break;
            }
        }
        pos += line.len();
    }
    start.map(|s| &content[s..end])
}

fn extract_source_refs(content: &str) -> Vec<(String, Vec<String>)> {
    let mut refs: Vec<(String, Vec<String>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut in_frontmatter = false;
    let mut at_start = true;
    let mut current_section = "body".to_string();
    let mut in_sources_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if at_start {
            at_start = false;
            if trimmed == "---" {
                in_frontmatter = true;
                current_section = "frontmatter".to_string();
                continue;
            }
        }
        if in_frontmatter && trimmed == "---" {
            in_frontmatter = false;
            current_section = "body".to_string();
            in_sources_block = false;
            continue;
        }
        if let Some(title) = trimmed.strip_prefix("## ") {
            current_section = title.trim().trim_matches('#').trim().to_string();
            in_sources_block = false;
        }

        if in_frontmatter && trimmed.starts_with("sources:") {
            in_sources_block = true;
            if let Some((_, value)) = trimmed.split_once(':') {
                for id in parse_inline_list(value) {
                    push_source_ref(&mut refs, &mut index, id, &current_section);
                }
            }
            continue;
        }
        if in_sources_block && (line.starts_with(' ') || line.starts_with('\t')) {
            if let Some(id) = parse_source_line(trimmed) {
                push_source_ref(&mut refs, &mut index, id, &current_section);
            }
            continue;
        } else if in_sources_block && !trimmed.is_empty() {
            in_sources_block = false;
        }

        if let Some(id) = parse_source_line(trimmed) {
            push_source_ref(&mut refs, &mut index, id, &current_section);
        }
    }
    refs
}

fn parse_source_line(trimmed: &str) -> Option<String> {
    let line = trimmed.trim_start_matches("- ").trim();
    if let Some((key, value)) = line.split_once(':') {
        if key.trim() == "source_id" {
            return clean_source_id(value);
        }
    }
    None
}

fn parse_inline_list(value: &str) -> Vec<String> {
    let value = value.trim();
    if !(value.starts_with('[') && value.ends_with(']')) {
        return Vec::new();
    }
    value[1..value.len() - 1]
        .split(',')
        .filter_map(clean_source_id)
        .collect()
}

fn push_source_ref(
    refs: &mut Vec<(String, Vec<String>)>,
    index: &mut HashMap<String, usize>,
    id: String,
    cited_in: &str,
) {
    if let Some(i) = index.get(&id).copied() {
        if !refs[i].1.iter().any(|s| s == cited_in) {
            refs[i].1.push(cited_in.to_string());
        }
        return;
    }
    index.insert(id.clone(), refs.len());
    refs.push((id, vec![cited_in.to_string()]));
}

fn clean_source_id(raw: &str) -> Option<String> {
    let cleaned = clean_scalar(raw)
        .trim_matches([',', '[', ']'])
        .trim()
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn clean_scalar(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .trim()
        .to_string()
}

fn parse_rfc3339_ms(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn has_actionable_text(body: &str) -> bool {
    body.lines().any(|line| {
        let t = line
            .trim()
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim();
        !t.is_empty()
            && !matches!(
                t,
                "暂无"
                    | "无"
                    | "None"
                    | "none"
                    | "N/A"
                    | "n/a"
                    | "未从资料中稳定抽取时间线。"
                    | "需要人工复核并补充更细粒度的结构化事实。"
            )
    })
}

fn contains_conflict_marker(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("conflict")
        || lower.contains("contradict")
        || content.contains("矛盾")
        || content.contains("冲突")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sources_from_frontmatter_and_evidence() {
        let doc = r#"---
type: source_summary
sources:
  - source_id: "src-a"
last_compiled: "2026-07-01T00:00:00Z"
confidence: medium
---

## Evidence

- source_id: `src-b`
"#;
        let refs = extract_source_refs(doc);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].0, "src-a");
        assert_eq!(refs[0].1, vec!["frontmatter"]);
        assert_eq!(refs[1].0, "src-b");
        assert_eq!(refs[1].1, vec!["Evidence"]);
        let snapshot = inspect_note(doc);
        assert_eq!(
            snapshot.frontmatter.get("type").map(String::as_str),
            Some("source_summary")
        );
        assert!(snapshot.last_compiled_at.is_some());
    }

    #[test]
    fn detects_required_sections() {
        let sections = section_titles("## For Agent\n\nx\n## Evidence\n\n- source_id: `s`\n");
        assert!(sections.contains("For Agent"));
        assert!(sections.contains("Evidence"));
        assert!(!sections.contains("Compiled Truth"));
    }

    #[test]
    fn skips_plain_notes_without_schema_markers() {
        let snapshot = inspect_note("# Plain note\n\nfree-form text");
        assert!(!is_schema_lint_candidate(&snapshot));

        let compiled = inspect_note(
            r#"---
type: source_summary
last_compiled: "2026-07-01T00:00:00Z"
---

## Evidence

- source_id: "src-a"
"#,
        );
        assert!(is_schema_lint_candidate(&compiled));
    }

    #[test]
    fn indexed_schema_marker_uses_compiler_frontmatter() {
        assert!(!indexed_schema_marker(None));
        assert!(!indexed_schema_marker(Some(r#"{"title":"Plain"}"#)));
        assert!(indexed_schema_marker(Some(r#"{"type":"source_summary"}"#)));
        assert!(indexed_schema_marker(Some(r#"{"sources":[{"source_id":"src-a"}]}"#)));
    }
}
