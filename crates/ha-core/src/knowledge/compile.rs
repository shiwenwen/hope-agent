//! Knowledge Compiler Phase 2.
//!
//! Compile runs turn raw sources into durable Review Diff proposals. Nothing in
//! this module mutates notes until the owner approves a proposal.

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::types::{
    CompileProposal, CompileProposalAction, CompileProposalKind, CompileProposalStatus, CompileRun,
    CompileRunStatus, CompileStartInput, KnowledgeSource, NewCompileProposal,
    DEFAULT_SCHEMA_SECTIONS,
};
use super::{service, source};

const DEFAULT_STRATEGY: &str = "source_summary_v1";
const MAX_SOURCE_PROMPT_CHARS: usize = 18_000;
const MAX_RELATED_NOTES: usize = 5;
const LLM_TIMEOUT_SECS: u64 = 120;
const LLM_MAX_TOKENS: u32 = 4_000;

fn registry() -> Result<&'static std::sync::Arc<super::KnowledgeRegistry>> {
    crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))
}

#[derive(Debug, Deserialize)]
struct LlmSummary {
    title: Option<String>,
    content: Option<String>,
}

pub async fn start_compile_run(kb_id: &str, input: CompileStartInput) -> Result<CompileRun> {
    let kb = registry()?
        .get(kb_id)?
        .ok_or_else(|| anyhow!("knowledge base not found: {kb_id}"))?;
    if kb.archived {
        bail!("cannot compile archived knowledge base: {kb_id}");
    }
    let strategy = input
        .strategy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_STRATEGY)
        .to_string();
    if strategy != DEFAULT_STRATEGY {
        bail!("unsupported compile strategy: {strategy}");
    }

    let mut source_ids = input.source_ids;
    source_ids.sort();
    source_ids.dedup();
    if source_ids.is_empty() {
        bail!("compile requires at least one source");
    }

    let sources = load_sources(kb_id, &source_ids)?;
    let fingerprint = compile_fingerprint(kb_id, &strategy, &sources);
    let (run, should_execute) =
        registry()?.begin_compile_run(kb_id, &source_ids, &strategy, &fingerprint)?;
    if !should_execute {
        return Ok(run);
    }

    match execute_compile_run(&run, &sources).await {
        Ok((summary, inserted, model_label)) => {
            registry()?.mark_sources_compiled(kb_id, &source_ids)?;
            registry()?.finish_compile_run(
                &run.id,
                CompileRunStatus::Completed,
                Some(&summary),
                None,
                inserted as u32,
                model_label.as_deref(),
            )?;
        }
        Err(e) => {
            let was_cancelled = registry()?
                .get_compile_run(&run.id)?
                .map(|r| r.status == CompileRunStatus::Cancelled)
                .unwrap_or(false);
            if !was_cancelled {
                registry()?.finish_compile_run(
                    &run.id,
                    CompileRunStatus::Failed,
                    None,
                    Some(&e.to_string()),
                    0,
                    None,
                )?;
            }
        }
    }

    registry()?
        .get_compile_run(&run.id)?
        .ok_or_else(|| anyhow!("compile run vanished after execution"))
}

pub fn list_runs(kb_id: &str) -> Result<Vec<CompileRun>> {
    ensure_kb_exists(kb_id)?;
    registry()?.list_compile_runs(kb_id)
}

pub fn get_run(run_id: &str) -> Result<CompileRun> {
    registry()?
        .get_compile_run(run_id)?
        .ok_or_else(|| anyhow!("compile run not found: {run_id}"))
}

pub fn cancel_run(run_id: &str) -> Result<CompileRun> {
    registry()?
        .cancel_compile_run(run_id)?
        .ok_or_else(|| anyhow!("compile run not found: {run_id}"))
}

pub fn list_proposals(
    kb_id: &str,
    run_id: Option<&str>,
    status: Option<CompileProposalStatus>,
) -> Result<Vec<CompileProposal>> {
    ensure_kb_exists(kb_id)?;
    registry()?.list_compile_proposals(kb_id, run_id, status)
}

pub async fn approve_proposal(id: i64) -> Result<CompileProposal> {
    let proposal = registry()?
        .get_compile_proposal(id)?
        .ok_or_else(|| anyhow!("compile proposal {id} not found"))?;
    if proposal.status != CompileProposalStatus::Draft {
        bail!(
            "compile proposal {id} is not pending (status: {})",
            proposal.status.as_str()
        );
    }
    match apply_proposal(&proposal).await {
        Ok(()) => {
            registry()?.set_compile_proposal_status(id, CompileProposalStatus::Applied, None)?
        }
        Err(e) => {
            let message = e.to_string();
            registry()?.set_compile_proposal_status(
                id,
                CompileProposalStatus::Draft,
                Some(&message),
            )?;
            bail!(message);
        }
    }
    registry()?
        .get_compile_proposal(id)?
        .ok_or_else(|| anyhow!("compile proposal {id} vanished after decision"))
}

pub fn reject_proposal(id: i64) -> Result<bool> {
    let proposal = registry()?
        .get_compile_proposal(id)?
        .ok_or_else(|| anyhow!("compile proposal {id} not found"))?;
    if proposal.status != CompileProposalStatus::Draft {
        bail!(
            "compile proposal {id} is not pending (status: {})",
            proposal.status.as_str()
        );
    }
    registry()?.set_compile_proposal_status(id, CompileProposalStatus::Rejected, None)?;
    Ok(true)
}

async fn execute_compile_run(
    run: &CompileRun,
    sources: &[(KnowledgeSource, String)],
) -> Result<(String, usize, Option<String>)> {
    let mut proposals = Vec::new();
    let mut model_label = None;
    for (source_meta, content) in sources {
        ensure_run_not_cancelled(&run.id)?;
        let related = related_notes(&run.kb_id, content);
        let generated = generate_summary(&run.kb_id, source_meta, content, &related).await;
        let (summary, label) = match generated {
            Ok((content, label)) => (content, label),
            Err(e) => {
                crate::app_warn!(
                    "knowledge",
                    "compile",
                    "LLM compile for source {} failed, using fallback summary: {}",
                    source_meta.id,
                    e
                );
                (fallback_summary(source_meta, content, &related), None)
            }
        };
        if model_label.is_none() {
            model_label = label;
        }
        proposals.push(build_summary_proposal(
            &run.kb_id,
            run,
            source_meta,
            &summary,
        )?);
    }
    ensure_run_not_cancelled(&run.id)?;
    let inserted = registry()?.insert_compile_proposals(&run.id, &run.kb_id, &proposals)?;
    let summary = format!(
        "Generated {inserted} review proposal(s) from {} source(s).",
        sources.len()
    );
    Ok((summary, inserted, model_label))
}

async fn apply_proposal(p: &CompileProposal) -> Result<()> {
    let kb = p.kb_id.as_str();
    match &p.action {
        CompileProposalAction::CreateNote {
            path,
            content,
            overwrite,
        }
        | CompileProposalAction::CreateMoc {
            path,
            content,
            overwrite,
        } => {
            service::note_save(kb, path, content, None, !*overwrite)?;
        }
        CompileProposalAction::PatchNote {
            path,
            old,
            new,
            expected_file_hash,
        } => {
            let cur = service::note_read(kb, path)?;
            if let Some(expected) = expected_file_hash {
                if cur.content_hash != *expected {
                    bail!("stale patch: '{path}' changed since the proposal was made");
                }
            }
            let matches = cur.content.matches(old).count();
            if matches != 1 {
                bail!("patch target must match exactly once in '{path}' (found {matches})");
            }
            let updated = cur.content.replacen(old, new, 1);
            service::note_save(kb, path, &updated, Some(&cur.content_hash), false)?;
        }
        CompileProposalAction::SetFrontmatter { path, props } => {
            let cur = service::note_read(kb, path)?;
            let updated = super::parser::merge_frontmatter(&cur.content, props);
            service::note_save(kb, path, &updated, Some(&cur.content_hash), false)?;
        }
        CompileProposalAction::AppendLink { from_path, to_ref } => {
            let cur = service::note_read(kb, from_path)?;
            let link = format!("[[{to_ref}]]");
            if cur.content.contains(&link) {
                return Ok(());
            }
            let mut updated = cur.content;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&format!("\n{link}\n"));
            service::note_save(kb, from_path, &updated, Some(&cur.content_hash), false)?;
        }
    }
    Ok(())
}

fn load_sources(kb_id: &str, source_ids: &[String]) -> Result<Vec<(KnowledgeSource, String)>> {
    let mut out = Vec::new();
    for source_id in source_ids {
        let read = source::read_source(kb_id, source_id)?;
        out.push((read.source, read.content));
    }
    Ok(out)
}

fn compile_fingerprint(
    kb_id: &str,
    strategy: &str,
    sources: &[(KnowledgeSource, String)],
) -> String {
    let mut parts = vec![format!("compile:v1:{kb_id}:{strategy}")];
    for (source, _) in sources {
        parts.push(format!("{}:{}", source.id, source.content_hash));
    }
    super::blake3_hex(parts.join("\n").as_bytes())
}

fn ensure_kb_exists(kb_id: &str) -> Result<()> {
    registry()?
        .get(kb_id)?
        .map(|_| ())
        .ok_or_else(|| anyhow!("knowledge base not found: {kb_id}"))
}

fn ensure_run_not_cancelled(run_id: &str) -> Result<()> {
    if registry()?
        .get_compile_run(run_id)?
        .map(|r| r.status == CompileRunStatus::Cancelled)
        .unwrap_or(false)
    {
        bail!("compile run cancelled");
    }
    Ok(())
}

async fn generate_summary(
    kb_id: &str,
    source_meta: &KnowledgeSource,
    content: &str,
    related: &[String],
) -> Result<(String, Option<String>)> {
    let config = crate::config::cached_config();
    let (agent, model) = crate::recap::report::build_analysis_agent(&config).await?;
    let prompt = summary_prompt(kb_id, source_meta, content, related);
    let fut = agent.side_query(&prompt, LLM_MAX_TOKENS);
    let res = tokio::time::timeout(std::time::Duration::from_secs(LLM_TIMEOUT_SECS), fut)
        .await
        .map_err(|_| anyhow!("compile LLM call timed out"))??;
    let parsed = parse_llm_summary(&res.text)?;
    let title = parsed
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&source_meta.title);
    let body = parsed
        .content
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("compile LLM response missing content"))?;
    Ok((
        normalize_compiled_markdown(title, source_meta, body),
        Some(model),
    ))
}

fn summary_prompt(
    kb_id: &str,
    source_meta: &KnowledgeSource,
    content: &str,
    related: &[String],
) -> String {
    let source_excerpt = crate::truncate_utf8(content, MAX_SOURCE_PROMPT_CHARS);
    let related = if related.is_empty() {
        "No related notes were found.".to_string()
    } else {
        related.join("\n")
    };
    format!(
        r#"You are compiling a raw source into a durable Markdown knowledge note for Hope Agent.

Return ONLY a valid JSON object:
{{
  "title": "short note title",
  "content": "full markdown body"
}}

Requirements for content:
- Write in the same language as the source when possible.
- Keep it factual; do not invent details absent from the source.
- Use these sections exactly: ## For Agent, ## Compiled Truth, ## Timeline, ## Evidence, ## Open Questions, ## Related.
- Every important fact in Compiled Truth should mention its source using `[source_id: {source_id}]`.
- Evidence must include at least one bullet with `source_id: "{source_id}"` and a short supporting excerpt.
- Related should use wikilink bullets only when one of the related notes below is genuinely useful.

Knowledge base id: {kb_id}
Source title: {title}
Source kind: {kind}
Source origin: {origin}

Related notes:
{related}

Raw source snapshot:
<source>
{source_excerpt}
</source>
"#,
        source_id = source_meta.id,
        title = source_meta.title,
        kind = source_meta.kind.as_str(),
        origin = source_meta.origin_uri.as_deref().unwrap_or("local import"),
    )
}

fn parse_llm_summary(text: &str) -> Result<LlmSummary> {
    let trimmed = strip_code_fence(text.trim());
    match serde_json::from_str::<LlmSummary>(&trimmed) {
        Ok(summary) => Ok(summary),
        Err(first_err) => {
            let start = trimmed
                .find('{')
                .ok_or_else(|| anyhow!("invalid compile JSON: {first_err}; no JSON object"))?;
            let end = trimmed
                .rfind('}')
                .ok_or_else(|| anyhow!("invalid compile JSON: {first_err}; no JSON object"))?;
            serde_json::from_str::<LlmSummary>(&trimmed[start..=end])
                .map_err(|e| anyhow!("invalid compile JSON: {e}"))
        }
    }
}

fn strip_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let without_start = trimmed.lines().skip(1).collect::<Vec<_>>().join("\n");
    without_start
        .trim_end()
        .strip_suffix("```")
        .unwrap_or(without_start.trim_end())
        .trim()
        .to_string()
}

fn normalize_compiled_markdown(title: &str, source_meta: &KnowledgeSource, body: &str) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("type: source_summary\n");
    out.push_str("sources:\n");
    out.push_str(&format!(
        "  - source_id: \"{}\"\n",
        yaml_escape(&source_meta.id)
    ));
    out.push_str(&format!(
        "last_compiled: \"{}\"\n",
        chrono::Utc::now().to_rfc3339()
    ));
    out.push_str("confidence: medium\n");
    out.push_str("---\n\n");
    let body = ensure_default_sections(body.trim());
    if body.starts_with('#') {
        out.push_str(&body);
    } else {
        out.push_str(&format!("# {}\n\n{}", title.trim(), body));
    }
    out.push('\n');
    out
}

fn ensure_default_sections(body: &str) -> String {
    let mut out = body.trim().to_string();
    for section in DEFAULT_SCHEMA_SECTIONS {
        let heading = format!("## {section}");
        if !out.lines().any(|line| line.trim() == heading) {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("\n{heading}\n\n"));
        }
    }
    out
}

fn fallback_summary(source_meta: &KnowledgeSource, content: &str, related: &[String]) -> String {
    let title = source_meta.title.trim();
    let excerpt = crate::truncate_utf8(content.trim(), 6_000);
    let related = if related.is_empty() {
        "- 暂无\n".to_string()
    } else {
        related
            .iter()
            .map(|r| format!("- {r}\n"))
            .collect::<String>()
    };
    normalize_compiled_markdown(
        title,
        source_meta,
        &format!(
            r#"# {title}

## For Agent

这是一份由原始资料编译得到的 source summary。优先把它当作来源摘要使用，关键事实仍应回看 Evidence 中的 source id。

## Compiled Truth

> 以下内容全部来自 source_id: `{source_id}`。

{excerpt}

## Timeline

- 未从资料中稳定抽取时间线。

## Evidence

- source_id: `{source_id}`
- source_title: {source_title}

## Open Questions

- 需要人工复核并补充更细粒度的结构化事实。

## Related

{related}"#,
            source_id = source_meta.id,
            source_title = source_meta.title,
        ),
    )
}

fn build_summary_proposal(
    kb_id: &str,
    run: &CompileRun,
    source_meta: &KnowledgeSource,
    content: &str,
) -> Result<NewCompileProposal> {
    let path = summary_path(&source_meta.title);
    let current = service::note_read(kb_id, &path).ok();
    let (kind, action, before_text) = if let Some(cur) = current {
        (
            CompileProposalKind::PatchNote,
            CompileProposalAction::PatchNote {
                path: path.clone(),
                old: cur.content.clone(),
                new: content.to_string(),
                expected_file_hash: Some(cur.content_hash),
            },
            Some(cur.content),
        )
    } else {
        (
            CompileProposalKind::CreateNote,
            CompileProposalAction::CreateNote {
                path: path.clone(),
                content: content.to_string(),
                overwrite: false,
            },
            Some(String::new()),
        )
    };
    let fingerprint = super::blake3_hex(
        format!(
            "compile-proposal:v1:{}:{}:{}:{}",
            run.fingerprint, source_meta.id, path, source_meta.content_hash
        )
        .as_bytes(),
    );
    Ok(NewCompileProposal {
        kind,
        title: format!("Compile {}", source_meta.title),
        detail: format!("{} -> {}", source_meta.id, path),
        action,
        fingerprint,
        source_ids: vec![source_meta.id.clone()],
        before_text,
        after_text: Some(content.to_string()),
    })
}

fn related_notes(kb_id: &str, content: &str) -> Vec<String> {
    service::search(
        Some(kb_id),
        crate::truncate_utf8(content, 2_000),
        MAX_RELATED_NOTES,
    )
    .unwrap_or_default()
    .into_iter()
    .take(MAX_RELATED_NOTES)
    .map(|hit| format!("- [[{}]] — {}", hit.rel_path, hit.title))
    .collect()
}

fn summary_path(title: &str) -> String {
    let stem = sanitize_file_stem(title);
    format!("Source Summaries/{stem}.md")
}

fn sanitize_file_stem(title: &str) -> String {
    let mut out = String::new();
    for ch in title.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ' ' {
            out.push(ch);
        } else if ch.is_alphanumeric() {
            out.push(ch);
        } else {
            out.push(' ');
        }
    }
    let compact = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = crate::truncate_utf8(compact.trim(), 80).trim().to_string();
    if compact.is_empty() {
        "Untitled Source".to_string()
    } else {
        compact
    }
}

fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
