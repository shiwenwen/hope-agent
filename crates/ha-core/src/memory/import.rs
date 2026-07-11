use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::helpers::load_dedup_config;
use super::traits::MemoryBackend;
use super::types::*;

const DEFAULT_IMPORT_AGENT_ID: &str = crate::agent_loader::DEFAULT_AGENT_ID;
const DEFAULT_IMPORT_PREVIEW_SAMPLES: usize = 8;
const MAX_IMPORT_PREVIEW_SAMPLES: usize = 50;
const IMPORT_PREVIEW_CONTENT_CHARS: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryImportPreviewIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryImportPreviewSample {
    pub memory_type: MemoryType,
    pub scope: MemoryScope,
    pub content_preview: String,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_existing_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_existing_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryImportPreview {
    pub valid: bool,
    pub format: String,
    pub candidate_count: usize,
    #[serde(default)]
    pub dedup_checked: bool,
    #[serde(default)]
    pub likely_new_count: usize,
    #[serde(default)]
    pub likely_merge_count: usize,
    #[serde(default)]
    pub likely_duplicate_count: usize,
    pub by_type: BTreeMap<String, usize>,
    pub by_scope: BTreeMap<String, usize>,
    pub samples: Vec<MemoryImportPreviewSample>,
    pub issues: Vec<MemoryImportPreviewIssue>,
}

#[derive(Debug, Clone)]
struct ImportPreviewDedupOutcome {
    status: &'static str,
    existing_id: Option<i64>,
    existing_preview: Option<String>,
    score: Option<f32>,
}

// ── Import Parsers ──────────────────────────────────────────────

/// Dispatch parse to the right format handler. Accepted values for `format`:
/// `auto`, `json`, `markdown`, `md`, `text`, `txt`. Unknown formats return an
/// error whose message is stable across callers so CLI / REST / Tauri layers
/// surface the same text.
pub fn parse_import(content: &str, format: &str) -> Result<Vec<NewMemory>> {
    match normalize_import_format(format).as_str() {
        "auto" => parse_import_auto(content),
        "json" => parse_import_json(content),
        "markdown" | "md" | "text" | "txt" => parse_import_markdown(content),
        other => Err(anyhow::anyhow!("Unsupported format: {}", other)),
    }
}

/// Read-only import preview. It parses the incoming payload exactly like
/// `parse_import`, then summarizes candidates without touching the store.
pub fn preview_import(
    content: &str,
    format: &str,
    sample_limit: Option<usize>,
) -> MemoryImportPreview {
    let format = normalize_import_format(format);
    let sample_limit = sample_limit
        .unwrap_or(DEFAULT_IMPORT_PREVIEW_SAMPLES)
        .clamp(1, MAX_IMPORT_PREVIEW_SAMPLES);
    match parse_import(content, &format) {
        Ok(entries) => preview_from_entries(format, entries, sample_limit),
        Err(err) => MemoryImportPreview {
            valid: false,
            format,
            candidate_count: 0,
            dedup_checked: false,
            likely_new_count: 0,
            likely_merge_count: 0,
            likely_duplicate_count: 0,
            by_type: BTreeMap::new(),
            by_scope: BTreeMap::new(),
            samples: Vec::new(),
            issues: vec![MemoryImportPreviewIssue {
                code: "parse_error".to_string(),
                message: err.to_string(),
            }],
        },
    }
}

/// Store-aware import preview. When `dedup` is true, this estimates how many
/// candidates would be created, merged into an existing row, or skipped as
/// duplicates under the current dedup thresholds. It never writes to the store;
/// the real apply path still uses `MemoryBackend::import_entries`.
pub fn preview_import_with_backend(
    backend: &dyn MemoryBackend,
    content: &str,
    format: &str,
    sample_limit: Option<usize>,
    dedup: bool,
) -> MemoryImportPreview {
    let format = normalize_import_format(format);
    let sample_limit = sample_limit
        .unwrap_or(DEFAULT_IMPORT_PREVIEW_SAMPLES)
        .clamp(1, MAX_IMPORT_PREVIEW_SAMPLES);
    let entries = match parse_import(content, &format) {
        Ok(entries) => entries,
        Err(err) => {
            return MemoryImportPreview {
                valid: false,
                format,
                candidate_count: 0,
                dedup_checked: false,
                likely_new_count: 0,
                likely_merge_count: 0,
                likely_duplicate_count: 0,
                by_type: BTreeMap::new(),
                by_scope: BTreeMap::new(),
                samples: Vec::new(),
                issues: vec![MemoryImportPreviewIssue {
                    code: "parse_error".to_string(),
                    message: err.to_string(),
                }],
            };
        }
    };
    let mut preview = preview_from_entries(format, entries.clone(), sample_limit);
    if !dedup || entries.is_empty() {
        return preview;
    }

    let dedup_cfg = load_dedup_config();
    let mut likely_new = 0usize;
    let mut likely_merge = 0usize;
    let mut likely_duplicate = 0usize;
    let mut outcomes: Vec<ImportPreviewDedupOutcome> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let similar = match backend.find_similar(
            &entry.content,
            Some(&entry.memory_type),
            Some(&entry.scope),
            dedup_cfg.threshold_merge,
            1,
        ) {
            Ok(similar) => similar,
            Err(err) => {
                preview.issues.push(MemoryImportPreviewIssue {
                    code: "dedup_preview_failed".to_string(),
                    message: format!("Could not estimate duplicates: {err}"),
                });
                preview.dedup_checked = false;
                preview.likely_new_count = preview.candidate_count;
                preview.likely_merge_count = 0;
                preview.likely_duplicate_count = 0;
                return preview;
            }
        };
        let Some(best) = similar.first() else {
            likely_new += 1;
            outcomes.push(ImportPreviewDedupOutcome {
                status: "new",
                existing_id: None,
                existing_preview: None,
                score: None,
            });
            continue;
        };
        let score = best.relevance_score.unwrap_or(0.0);
        if score >= dedup_cfg.threshold_high {
            likely_duplicate += 1;
            outcomes.push(dedup_outcome("duplicate", best, score));
        } else {
            likely_merge += 1;
            outcomes.push(dedup_outcome("merge", best, score));
        }
    }
    preview.dedup_checked = true;
    preview.likely_new_count = likely_new;
    preview.likely_merge_count = likely_merge;
    preview.likely_duplicate_count = likely_duplicate;
    apply_sample_dedup_outcomes(&mut preview.samples, &outcomes);
    preview
}

/// Parse JSON when possible, otherwise fall back to Markdown/text. This is
/// deliberately owner-plane only and writes nothing; callers still decide
/// whether to apply the parsed rows.
pub fn parse_import_auto(content: &str) -> Result<Vec<NewMemory>> {
    let cleaned = strip_outer_code_fence(content);
    if looks_like_json(cleaned) {
        match parse_import_json(cleaned) {
            Ok(entries) => return Ok(entries),
            Err(json_err) => {
                let markdown_entries = parse_import_markdown(cleaned)?;
                if !markdown_entries.is_empty() {
                    return Ok(markdown_entries);
                }
                return Err(json_err);
            }
        }
    }
    parse_import_markdown(cleaned)
}

/// Parse JSON import format:
/// - array of `{ content, type?, scope?, tags? }`
/// - object with `memories` / `items` / `entries` array
/// - a single object with content-like field (`content`, `text`, `memory`, `fact`)
pub fn parse_import_json(json_str: &str) -> Result<Vec<NewMemory>> {
    let cleaned = strip_outer_code_fence(json_str);
    let value: Value =
        serde_json::from_str(cleaned).with_context(|| "Invalid JSON: expected memory object(s)")?;

    let items = json_memory_items(&value)?;

    let mut entries = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            return Err(anyhow::anyhow!("Each memory must be a JSON object"));
        };
        let content = pick_string(object, &["content", "text", "memory", "fact"])
            .ok_or_else(|| anyhow::anyhow!("Each memory must have a 'content' field"))?;
        let content = normalize_memory_content(content);
        if should_skip_content(&content) {
            continue;
        }

        let memory_type = pick_string(object, &["type", "memoryType", "category"])
            .map(infer_memory_type_from_label)
            .unwrap_or(MemoryType::User);
        let scope = parse_json_scope(item);
        let tags = parse_json_tags(item.get("tags"));

        entries.push(NewMemory {
            memory_type,
            scope,
            content,
            tags,
            source: "import".to_string(),
            source_session_id: None,
            pinned: item
                .get("pinned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            attachment_path: None,
            attachment_mime: None,
        });
    }
    Ok(entries)
}

/// Parse Markdown import format:
/// ## About the User / Preferences & Feedback / Project Context / References
/// ### Entry title
/// Tags: tag1, tag2
/// Scope: global | Source: user | Updated: ...
///
/// Content here...
///
/// ---
///
/// If no Hope Agent export-style entries are found, falls back to common
/// `MEMORY.md` / `USER.md` conventions: bullets, numbered lists, blockquotes,
/// and standalone paragraphs under broad section headings.
pub fn parse_import_markdown(md_str: &str) -> Result<Vec<NewMemory>> {
    let cleaned = strip_outer_code_fence(md_str);
    let entries = parse_hope_markdown(cleaned);
    if !entries.is_empty() {
        return Ok(entries);
    }
    Ok(parse_external_markdown(cleaned))
}

fn parse_hope_markdown(md_str: &str) -> Vec<NewMemory> {
    let mut entries = Vec::new();
    let mut current_type = MemoryType::User;
    let mut current_content = String::new();
    let mut current_tags: Vec<String> = Vec::new();
    let mut in_entry = false;

    for line in md_str.lines() {
        let trimmed = line.trim();

        // Type heading
        if trimmed.starts_with("## ") {
            // Flush previous entry
            if in_entry && !current_content.trim().is_empty() {
                entries.push(NewMemory {
                    memory_type: current_type.clone(),
                    scope: MemoryScope::Global,
                    content: current_content.trim().to_string(),
                    tags: std::mem::take(&mut current_tags),
                    source: "import".to_string(),
                    source_session_id: None,
                    pinned: false,
                    attachment_path: None,
                    attachment_mime: None,
                });
                current_content.clear();
                in_entry = false;
            }

            let heading = trimmed.trim_start_matches("## ").trim();
            current_type = infer_memory_type_from_heading(heading);
        } else if trimmed.starts_with("### ") {
            // Flush previous entry
            if in_entry && !current_content.trim().is_empty() {
                entries.push(NewMemory {
                    memory_type: current_type.clone(),
                    scope: MemoryScope::Global,
                    content: current_content.trim().to_string(),
                    tags: std::mem::take(&mut current_tags),
                    source: "import".to_string(),
                    source_session_id: None,
                    pinned: false,
                    attachment_path: None,
                    attachment_mime: None,
                });
                current_content.clear();
            }
            in_entry = true;
        } else if trimmed.starts_with("Tags:") {
            current_tags = trimmed
                .trim_start_matches("Tags:")
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        } else if trimmed.starts_with("Scope:") || trimmed == "---" {
            // Skip metadata lines and separators
        } else if in_entry && (!current_content.is_empty() || !trimmed.is_empty()) {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Flush last entry
    if in_entry && !current_content.trim().is_empty() {
        entries.push(NewMemory {
            memory_type: current_type,
            scope: MemoryScope::Global,
            content: current_content.trim().to_string(),
            tags: current_tags,
            source: "import".to_string(),
            source_session_id: None,
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        });
    }

    entries
}

fn preview_from_entries(
    format: String,
    entries: Vec<NewMemory>,
    sample_limit: usize,
) -> MemoryImportPreview {
    let mut by_type = BTreeMap::new();
    let mut by_scope = BTreeMap::new();
    for entry in &entries {
        *by_type
            .entry(entry.memory_type.as_str().to_string())
            .or_insert(0) += 1;
        *by_scope.entry(scope_preview_key(&entry.scope)).or_insert(0) += 1;
    }
    let samples = entries
        .iter()
        .take(sample_limit)
        .map(|entry| MemoryImportPreviewSample {
            memory_type: entry.memory_type.clone(),
            scope: entry.scope.clone(),
            content_preview: truncate_preview(&entry.content, IMPORT_PREVIEW_CONTENT_CHARS),
            tags: entry.tags.clone(),
            dedup_status: None,
            dedup_existing_id: None,
            dedup_existing_preview: None,
            dedup_score: None,
        })
        .collect();
    let mut issues = Vec::new();
    if entries.is_empty() {
        issues.push(MemoryImportPreviewIssue {
            code: "no_importable_entries".to_string(),
            message: "No importable memories found.".to_string(),
        });
    }
    MemoryImportPreview {
        valid: issues.is_empty(),
        format,
        candidate_count: entries.len(),
        dedup_checked: false,
        likely_new_count: entries.len(),
        likely_merge_count: 0,
        likely_duplicate_count: 0,
        by_type,
        by_scope,
        samples,
        issues,
    }
}

fn dedup_outcome(
    status: &'static str,
    existing: &MemoryEntry,
    score: f32,
) -> ImportPreviewDedupOutcome {
    ImportPreviewDedupOutcome {
        status,
        existing_id: Some(existing.id),
        existing_preview: Some(truncate_preview(
            &existing.content,
            IMPORT_PREVIEW_CONTENT_CHARS,
        )),
        score: Some(score),
    }
}

fn apply_sample_dedup_outcomes(
    samples: &mut [MemoryImportPreviewSample],
    outcomes: &[ImportPreviewDedupOutcome],
) {
    for (sample, outcome) in samples.iter_mut().zip(outcomes.iter()) {
        sample.dedup_status = Some(outcome.status.to_string());
        sample.dedup_existing_id = outcome.existing_id;
        sample.dedup_existing_preview = outcome.existing_preview.clone();
        sample.dedup_score = outcome.score;
    }
}

fn scope_preview_key(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_string(),
        MemoryScope::Agent { id } => format!("agent:{id}"),
        MemoryScope::Project { id } => format!("project:{id}"),
    }
}

fn parse_external_markdown(md_str: &str) -> Vec<NewMemory> {
    let mut out = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut current_type = MemoryType::User;
    let mut seen = HashSet::new();

    for raw_line in md_str.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            flush_markdown_paragraph(&mut paragraph, &mut out, &mut seen, current_type.clone());
            continue;
        }

        if is_markdown_heading(trimmed) {
            flush_markdown_paragraph(&mut paragraph, &mut out, &mut seen, current_type.clone());
            current_type = infer_memory_type_from_heading(trimmed.trim_start_matches('#').trim());
            continue;
        }

        if is_metadata_line(trimmed) || is_markdown_separator(trimmed) {
            continue;
        }

        if let Some(bullet) = strip_markdown_list_marker(trimmed) {
            flush_markdown_paragraph(&mut paragraph, &mut out, &mut seen, current_type.clone());
            push_markdown_entry(&mut out, &mut seen, current_type.clone(), bullet);
            continue;
        }

        let line = strip_blockquote_marker(trimmed);
        paragraph.push(line.to_string());
    }

    flush_markdown_paragraph(&mut paragraph, &mut out, &mut seen, current_type);
    out
}

fn flush_markdown_paragraph(
    buf: &mut Vec<String>,
    out: &mut Vec<NewMemory>,
    seen: &mut HashSet<String>,
    memory_type: MemoryType,
) {
    if buf.is_empty() {
        return;
    }
    let joined = buf.join(" ");
    buf.clear();
    push_markdown_entry(out, seen, memory_type, &joined);
}

fn push_markdown_entry(
    out: &mut Vec<NewMemory>,
    seen: &mut HashSet<String>,
    memory_type: MemoryType,
    content: &str,
) {
    let (memory_type, content) = split_inline_type_prefix(content, memory_type);
    let content = normalize_memory_content(content);
    if should_skip_content(&content) {
        return;
    }
    let dedup_key = content.to_ascii_lowercase();
    if !seen.insert(dedup_key) {
        return;
    }
    out.push(NewMemory {
        memory_type,
        scope: MemoryScope::Global,
        content,
        tags: Vec::new(),
        source: "import".to_string(),
        source_session_id: None,
        pinned: false,
        attachment_path: None,
        attachment_mime: None,
    });
}

fn split_inline_type_prefix(content: &str, fallback: MemoryType) -> (MemoryType, &str) {
    let Some((label, rest)) = content.split_once(':') else {
        return (fallback, content);
    };
    let label = label.trim();
    if label.is_empty() || label.len() > 40 {
        return (fallback, content);
    }
    let inferred = infer_memory_type_from_label(label);
    if inferred == MemoryType::User
        && !matches!(
            label.to_ascii_lowercase().as_str(),
            "user" | "profile" | "about user" | "用户" | "画像"
        )
    {
        return (fallback, content);
    }
    (inferred, rest.trim())
}

fn json_memory_items(value: &Value) -> Result<Vec<&Value>> {
    match value {
        Value::Array(items) => Ok(items.iter().collect()),
        Value::Object(object) => {
            for key in ["memories", "memory", "items", "entries"] {
                if let Some(Value::Array(items)) = object.get(key) {
                    return Ok(items.iter().collect());
                }
            }
            if pick_string(object, &["content", "text", "memory", "fact"]).is_some() {
                return Ok(vec![value]);
            }
            Err(anyhow::anyhow!(
                "Invalid JSON: expected an array or an object with a memories/items/entries array"
            ))
        }
        _ => Err(anyhow::anyhow!(
            "Invalid JSON: expected an array of memory objects"
        )),
    }
}

fn pick_string<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(value) = object.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

fn parse_json_tags(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .flat_map(split_tags)
            .collect(),
        Some(Value::String(raw)) => split_tags(raw).collect(),
        _ => Vec::new(),
    }
}

fn split_tags(raw: &str) -> impl Iterator<Item = String> + '_ {
    raw.split([',', '#'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(|tag| tag.to_ascii_lowercase())
}

fn parse_json_scope(item: &Value) -> MemoryScope {
    let Some(object) = item.as_object() else {
        return MemoryScope::Global;
    };
    let agent_id = pick_string(object, &["agentId", "agent_id"]).unwrap_or(DEFAULT_IMPORT_AGENT_ID);
    let project_id = pick_string(object, &["projectId", "project_id"]);
    match object.get("scope") {
        Some(Value::String(scope)) => parse_scope_string(scope, agent_id, project_id),
        Some(Value::Object(scope)) => parse_scope_object(scope, agent_id, project_id),
        _ => MemoryScope::Global,
    }
}

fn parse_scope_object(
    scope: &serde_json::Map<String, Value>,
    fallback_agent_id: &str,
    fallback_project_id: Option<&str>,
) -> MemoryScope {
    let kind = pick_string(scope, &["kind", "type", "scope"]).unwrap_or("global");
    let id = pick_string(
        scope,
        &["id", "agentId", "agent_id", "projectId", "project_id"],
    );
    parse_scope_string(
        kind,
        id.unwrap_or(fallback_agent_id),
        id.or(fallback_project_id),
    )
}

fn parse_scope_string(
    scope: &str,
    fallback_agent_id: &str,
    fallback_project_id: Option<&str>,
) -> MemoryScope {
    let normalized = scope.trim().to_ascii_lowercase();
    if normalized == "agent" {
        return MemoryScope::Agent {
            id: fallback_agent_id.to_string(),
        };
    }
    if let Some(id) = normalized.strip_prefix("agent:") {
        let id = id.trim();
        if !id.is_empty() {
            return MemoryScope::Agent { id: id.to_string() };
        }
    }
    if normalized == "project" {
        if let Some(id) = fallback_project_id.filter(|id| !id.trim().is_empty()) {
            return MemoryScope::Project { id: id.to_string() };
        }
        return MemoryScope::Global;
    }
    if let Some(id) = normalized.strip_prefix("project:") {
        let id = id.trim();
        if !id.is_empty() {
            return MemoryScope::Project { id: id.to_string() };
        }
    }
    MemoryScope::Global
}

fn infer_memory_type_from_heading(heading: &str) -> MemoryType {
    infer_memory_type_from_label(&heading.to_ascii_lowercase())
}

fn infer_memory_type_from_label(label: &str) -> MemoryType {
    let normalized = label.trim().to_ascii_lowercase();
    if normalized.contains("feedback")
        || normalized.contains("preference")
        || normalized.contains("instruction")
        || normalized.contains("rule")
        || normalized.contains("style")
        || normalized.contains("纠正")
        || normalized.contains("偏好")
        || normalized.contains("规则")
        || normalized.contains("要求")
    {
        return MemoryType::Feedback;
    }
    if normalized.contains("project")
        || normalized.contains("context")
        || normalized.contains("work")
        || normalized.contains("task")
        || normalized.contains("项目")
        || normalized.contains("工作")
        || normalized.contains("计划")
    {
        return MemoryType::Project;
    }
    if normalized.contains("reference")
        || normalized.contains("link")
        || normalized.contains("resource")
        || normalized.contains("document")
        || normalized.contains("tool")
        || normalized.contains("参考")
        || normalized.contains("链接")
        || normalized.contains("资料")
        || normalized.contains("文档")
        || normalized.contains("系统")
    {
        return MemoryType::Reference;
    }
    MemoryType::from_str(normalized.as_str())
}

fn strip_markdown_list_marker(line: &str) -> Option<&str> {
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(strip_checkbox(rest.trim()));
        }
    }
    strip_ordered_list_marker(line)
}

fn strip_ordered_list_marker(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 || idx + 1 >= bytes.len() {
        return None;
    }
    if (bytes[idx] == b'.' || bytes[idx] == b')') && bytes[idx + 1].is_ascii_whitespace() {
        return Some(strip_checkbox(line[idx + 2..].trim()));
    }
    None
}

fn strip_checkbox(line: &str) -> &str {
    let trimmed = line.trim_start();
    for prefix in ["[ ]", "[x]", "[X]"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    trimmed
}

fn strip_blockquote_marker(line: &str) -> &str {
    line.strip_prefix('>').map(str::trim).unwrap_or(line)
}

fn is_markdown_heading(line: &str) -> bool {
    line.starts_with('#')
}

fn is_markdown_separator(line: &str) -> bool {
    line.len() >= 3 && line.chars().all(|ch| ch == '-' || ch == '*' || ch == '_')
}

fn is_metadata_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("tags:")
        || lower.starts_with("scope:")
        || lower.starts_with("source:")
        || lower.starts_with("updated:")
        || lower.starts_with("created:")
        || lower.starts_with("type:")
}

fn normalize_memory_content(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['-', '*', ' ', '\t'])
        .trim()
        .to_string()
}

fn should_skip_content(content: &str) -> bool {
    if content.len() < 2 {
        return true;
    }
    let lower = content.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "no memories stored."
            | "no memories stored"
            | "no memories found."
            | "no memories found"
            | "none"
            | "n/a"
            | "null"
            | "[]"
    )
}

fn truncate_preview(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn normalize_import_format(format: &str) -> String {
    format.trim().to_ascii_lowercase()
}

fn looks_like_json(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with('[') || trimmed.starts_with('{')
}

fn strip_outer_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let Some(first_newline) = rest.find('\n') else {
        return trimmed;
    };
    let body = &rest[first_newline + 1..];
    let Some(end) = body.rfind("```") else {
        return trimmed;
    };
    body[..end].trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auto_json_wrapped_in_code_fence() {
        let entries = parse_import(
            r#"```json
{"memories":[{"content":"The user prefers concise replies.","type":"feedback","tags":"style,short"}]}
```"#,
            "auto",
        )
        .expect("parse");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].memory_type, MemoryType::Feedback);
        assert_eq!(entries[0].content, "The user prefers concise replies.");
        assert_eq!(entries[0].tags, vec!["style", "short"]);
    }

    #[test]
    fn parses_hope_markdown_export_without_splitting_body_bullets() {
        let entries = parse_import_markdown(
            r#"# Memories

## Preferences & Feedback

### Reply style
Tags: style, language
Scope: global | Source: user | Updated: 2026-01-01T00:00:00Z

The user wants Chinese replies.
- Preserve this body bullet as part of the same memory.

---
"#,
        )
        .expect("parse");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].memory_type, MemoryType::Feedback);
        assert_eq!(entries[0].tags, vec!["style", "language"]);
        assert!(entries[0].content.contains("Preserve this body bullet"));
    }

    #[test]
    fn parses_external_markdown_bullets_and_paragraphs() {
        let entries = parse_import_markdown(
            r#"# USER.md

## Preferences
- [x] The user wants Chinese replies by default.
1. The user prefers concise status updates.

## Projects
The user is upgrading Hope Agent's memory system.

## References
> The user archived the memory-system research plan in iCloud Plans.
"#,
        )
        .expect("parse");

        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].memory_type, MemoryType::Feedback);
        assert_eq!(
            entries[0].content,
            "The user wants Chinese replies by default."
        );
        assert_eq!(entries[1].memory_type, MemoryType::Feedback);
        assert_eq!(entries[2].memory_type, MemoryType::Project);
        assert_eq!(entries[3].memory_type, MemoryType::Reference);
    }

    #[test]
    fn parses_inline_type_prefixes_in_external_markdown() {
        let entries = parse_import_markdown(
            r#"- Preference: The user likes calm direct answers.
- Project: The user is planning a memory-system upgrade.
- Reference: The user tracks the archived memory-system plan.
"#,
        )
        .expect("parse");

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].memory_type, MemoryType::Feedback);
        assert_eq!(entries[0].content, "The user likes calm direct answers.");
        assert_eq!(entries[1].memory_type, MemoryType::Project);
        assert_eq!(entries[2].memory_type, MemoryType::Reference);
    }

    #[test]
    fn previews_import_without_writing() {
        let preview = preview_import(
            r#"- Preference: The user likes calm direct answers.
- Project: The user is planning a memory-system upgrade.
"#,
            "auto",
            Some(1),
        );

        assert!(preview.valid);
        assert_eq!(preview.candidate_count, 2);
        assert!(!preview.dedup_checked);
        assert_eq!(preview.likely_new_count, 2);
        assert_eq!(preview.by_type.get("feedback"), Some(&1));
        assert_eq!(preview.by_type.get("project"), Some(&1));
        assert_eq!(preview.by_scope.get("global"), Some(&2));
        assert_eq!(preview.samples.len(), 1);
    }

    #[test]
    fn previews_empty_import_as_invalid() {
        let preview = preview_import("# Memories\n\nNo memories stored.\n", "markdown", None);

        assert!(!preview.valid);
        assert_eq!(preview.candidate_count, 0);
        assert_eq!(preview.issues[0].code, "no_importable_entries");
    }

    #[test]
    fn ignores_empty_export_placeholder() {
        let entries = parse_import_markdown("# Memories\n\nNo memories stored.\n").expect("parse");
        assert!(entries.is_empty());
    }
}
