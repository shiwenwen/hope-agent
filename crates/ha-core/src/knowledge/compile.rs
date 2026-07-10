//! Knowledge Compiler Phase 2.
//!
//! Compile runs turn raw sources into durable Review Diff proposals. Nothing in
//! this module mutates notes until the owner approves a proposal.

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::types::{
    CompileProposal, CompileProposalAction, CompileProposalKind, CompileProposalStatus, CompileRun,
    CompileRunStatus, CompileStartInput, KnowledgeSource, KnowledgeSourceStatus,
    NewCompileProposal, QueryFileInput, QueryFileMode, DEFAULT_SCHEMA_SECTIONS,
};
use super::{service, source};
use crate::session::{MessageRole, SessionKind, SessionMessage};

const DEFAULT_STRATEGY: &str = "source_summary_v1";
const QUERY_FILE_STRATEGY: &str = "query_filing_v1";
const MAX_SOURCE_PROMPT_CHARS: usize = 18_000;
const MAX_FILED_ANSWER_CHARS: usize = 12_000;
const MAX_FILED_PROMPT_CHARS: usize = 3_000;
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

pub fn query_file(kb_id: &str, input: QueryFileInput) -> Result<CompileProposal> {
    ensure_kb_exists(kb_id)?;
    let session_db =
        crate::get_session_db().ok_or_else(|| anyhow!("session db not initialized"))?;
    let session = session_db
        .get_session(&input.session_id)?
        .ok_or_else(|| anyhow!("session not found: {}", input.session_id))?;
    if session.incognito {
        bail!("cannot file an incognito conversation into a knowledge base");
    }
    if session.kind != SessionKind::Knowledge && !input.confirm_conversation_source {
        bail!("filing a regular conversation requires explicit confirmation");
    }

    let message = session_db
        .get_message(input.message_id)?
        .ok_or_else(|| anyhow!("message not found: {}", input.message_id))?;
    if message.session_id != input.session_id {
        bail!("message does not belong to the requested session");
    }
    if message.role != MessageRole::Assistant {
        bail!("only assistant messages can be filed");
    }
    let answer = message.content.trim();
    if answer.is_empty() {
        bail!("assistant message is empty");
    }
    let user_prompt = previous_user_prompt(
        &session_db.load_session_messages(&input.session_id)?,
        message.id,
    );
    let mode = input.mode.unwrap_or(QueryFileMode::CreateNote);
    let title = normalize_title(
        input.title.as_deref(),
        user_prompt.as_deref().unwrap_or(answer),
    );
    let target_path = resolve_query_file_target(&input, mode, &title)?;
    let proposal = build_query_file_proposal(
        kb_id,
        &input,
        mode,
        &message,
        user_prompt,
        &title,
        &target_path,
    )?;
    let fingerprint = query_file_run_fingerprint(kb_id, &input, mode, &target_path, &proposal);
    let (run, should_execute) =
        registry()?.begin_compile_run(kb_id, &[], QUERY_FILE_STRATEGY, &fingerprint)?;
    if !should_execute {
        return existing_query_file_proposal(kb_id, &run.id, &proposal.fingerprint);
    }

    let inserted =
        registry()?.insert_compile_proposals(&run.id, kb_id, std::slice::from_ref(&proposal))?;
    registry()?.finish_compile_run(
        &run.id,
        CompileRunStatus::Completed,
        Some("Generated 1 query filing review proposal."),
        None,
        inserted as u32,
        None,
    )?;
    existing_query_file_proposal(kb_id, &run.id, &proposal.fingerprint)
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
        if read.source.status != KnowledgeSourceStatus::Ready {
            bail!(
                "source '{}' is not ready to compile (status: {}) — wait for OCR/extraction to finish, or retry failed pages, before organizing it into a note",
                read.source.title,
                read.source.status.as_str()
            );
        }
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

fn existing_query_file_proposal(
    kb_id: &str,
    run_id: &str,
    fingerprint: &str,
) -> Result<CompileProposal> {
    registry()?
        .list_compile_proposals(kb_id, Some(run_id), None)?
        .into_iter()
        .find(|p| p.fingerprint == fingerprint)
        .or_else(|| {
            registry()
                .ok()
                .and_then(|reg| reg.list_compile_proposals(kb_id, None, None).ok())
                .and_then(|items| items.into_iter().find(|p| p.fingerprint == fingerprint))
        })
        .ok_or_else(|| anyhow!("query filing proposal vanished after creation"))
}

fn previous_user_prompt(messages: &[SessionMessage], assistant_message_id: i64) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.id < assistant_message_id && m.role == MessageRole::User)
        .map(|m| m.content.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn resolve_query_file_target(
    input: &QueryFileInput,
    mode: QueryFileMode,
    title: &str,
) -> Result<String> {
    let clean = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    };
    match mode {
        QueryFileMode::CreateNote => Ok(clean(&input.target_path).unwrap_or_else(|| {
            format!(
                "Filed Conversations/{}-{}.md",
                sanitize_file_stem(title),
                input.message_id
            )
        })),
        QueryFileMode::UpdateCurrentNote => clean(&input.current_note_path)
            .ok_or_else(|| anyhow!("current note is required for update-current-note filing")),
        QueryFileMode::AppendToMoc => clean(&input.target_path)
            .ok_or_else(|| anyhow!("target MOC path is required for MOC filing")),
        QueryFileMode::AppendOpenQuestions => clean(&input.target_path)
            .or_else(|| clean(&input.current_note_path))
            .ok_or_else(|| anyhow!("target note is required for Open Questions filing")),
    }
}

fn build_query_file_proposal(
    kb_id: &str,
    input: &QueryFileInput,
    mode: QueryFileMode,
    message: &SessionMessage,
    user_prompt: Option<String>,
    title: &str,
    target_path: &str,
) -> Result<NewCompileProposal> {
    let answer = crate::truncate_utf8(message.content.trim(), MAX_FILED_ANSWER_CHARS);
    let user_prompt = user_prompt
        .as_deref()
        .map(|s| crate::truncate_utf8(s, MAX_FILED_PROMPT_CHARS).to_string());
    let filed_block = conversation_filed_block(title, user_prompt.as_deref(), answer, message);
    let (kind, action, before_text, after_text) = match mode {
        QueryFileMode::CreateNote => {
            let content = conversation_note_content(title, user_prompt.as_deref(), answer, message);
            (
                CompileProposalKind::CreateNote,
                CompileProposalAction::CreateNote {
                    path: target_path.to_string(),
                    content: content.clone(),
                    overwrite: false,
                },
                Some(String::new()),
                Some(content),
            )
        }
        QueryFileMode::UpdateCurrentNote => {
            let cur = service::note_read(kb_id, target_path)?;
            let updated = append_under_section(&cur.content, "Compiled Truth", &filed_block);
            (
                CompileProposalKind::PatchNote,
                CompileProposalAction::PatchNote {
                    path: target_path.to_string(),
                    old: cur.content.clone(),
                    new: updated.clone(),
                    expected_file_hash: Some(cur.content_hash),
                },
                Some(cur.content),
                Some(updated),
            )
        }
        QueryFileMode::AppendToMoc => {
            if let Ok(cur) = service::note_read(kb_id, target_path) {
                let updated =
                    append_under_section(&cur.content, "Filed Conversations", &filed_block);
                (
                    CompileProposalKind::PatchNote,
                    CompileProposalAction::PatchNote {
                        path: target_path.to_string(),
                        old: cur.content.clone(),
                        new: updated.clone(),
                        expected_file_hash: Some(cur.content_hash),
                    },
                    Some(cur.content),
                    Some(updated),
                )
            } else {
                let content = moc_with_filed_conversation(target_path, &filed_block);
                (
                    CompileProposalKind::CreateMoc,
                    CompileProposalAction::CreateMoc {
                        path: target_path.to_string(),
                        content: content.clone(),
                        overwrite: false,
                    },
                    Some(String::new()),
                    Some(content),
                )
            }
        }
        QueryFileMode::AppendOpenQuestions => {
            let cur = service::note_read(kb_id, target_path)?;
            let updated = append_under_section(
                &cur.content,
                "Open Questions",
                &open_question_filing(title, user_prompt.as_deref(), answer, message),
            );
            (
                CompileProposalKind::PatchNote,
                CompileProposalAction::PatchNote {
                    path: target_path.to_string(),
                    old: cur.content.clone(),
                    new: updated.clone(),
                    expected_file_hash: Some(cur.content_hash),
                },
                Some(cur.content),
                Some(updated),
            )
        }
    };
    let fingerprint = super::blake3_hex(
        format!(
            "query-file-proposal:v1:{kb_id}:{}:{}:{}:{}:{}",
            input.session_id,
            input.message_id,
            mode_key(mode),
            target_path,
            super::blake3_hex(answer.as_bytes())
        )
        .as_bytes(),
    );
    Ok(NewCompileProposal {
        kind,
        title: format!("File answer: {title}"),
        detail: format!(
            "conversation {}#{} -> {}",
            input.session_id, input.message_id, target_path
        ),
        action,
        fingerprint,
        source_ids: Vec::new(),
        before_text,
        after_text,
    })
}

fn query_file_run_fingerprint(
    kb_id: &str,
    input: &QueryFileInput,
    mode: QueryFileMode,
    target_path: &str,
    proposal: &NewCompileProposal,
) -> String {
    super::blake3_hex(
        format!(
            "query-file-run:v1:{kb_id}:{}:{}:{}:{}:{}",
            input.session_id,
            input.message_id,
            mode_key(mode),
            target_path,
            proposal.fingerprint
        )
        .as_bytes(),
    )
}

fn mode_key(mode: QueryFileMode) -> &'static str {
    match mode {
        QueryFileMode::CreateNote => "create_note",
        QueryFileMode::UpdateCurrentNote => "update_current_note",
        QueryFileMode::AppendToMoc => "append_to_moc",
        QueryFileMode::AppendOpenQuestions => "append_open_questions",
    }
}

fn normalize_title(requested: Option<&str>, fallback: &str) -> String {
    let raw = requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            fallback
                .lines()
                .next()
                .unwrap_or("Filed conversation")
                .trim()
        });
    let raw = raw
        .trim_start_matches('#')
        .trim_start_matches(['-', '*'])
        .trim();
    let title = crate::truncate_utf8(raw, 80).trim().to_string();
    if title.is_empty() {
        "Filed conversation".to_string()
    } else {
        title
    }
}

fn conversation_note_content(
    title: &str,
    user_prompt: Option<&str>,
    answer: &str,
    message: &SessionMessage,
) -> String {
    let filed_at = chrono::Utc::now().to_rfc3339();
    let prompt = user_prompt.unwrap_or("No user prompt captured.");
    format!(
        r#"---
type: conversation_note
source: conversation
conversation_session_id: "{}"
conversation_message_id: {}
filed_at: "{}"
confidence: medium
---

# {}

## For Agent

This note was filed from a knowledge conversation. Treat it as a reviewed conversation-derived synthesis and follow the Evidence section back to the original turn when needed.

## Compiled Truth

{}

## Timeline

- {}: Filed from conversation message `{}`.

## Evidence

- source: conversation
- session_id: `{}`
- message_id: `{}`
- assistant_timestamp: `{}`
- user_prompt: {}

## Open Questions

## Related
"#,
        yaml_escape(&message.session_id),
        message.id,
        yaml_escape(&filed_at),
        title.trim(),
        answer.trim(),
        filed_at,
        message.id,
        message.session_id,
        message.id,
        message.timestamp,
        yaml_inline(prompt),
    )
}

fn conversation_filed_block(
    title: &str,
    user_prompt: Option<&str>,
    answer: &str,
    message: &SessionMessage,
) -> String {
    let prompt = user_prompt.unwrap_or("No user prompt captured.");
    format!(
        "### {}\n\n> source: conversation · session `{}` · message `{}` · assistant `{}`\n\n**User prompt**\n\n> {}\n\n**Filed answer**\n\n{}\n",
        title.trim(),
        message.session_id,
        message.id,
        message.timestamp,
        quote_markdown(prompt),
        answer.trim(),
    )
}

fn open_question_filing(
    title: &str,
    user_prompt: Option<&str>,
    answer: &str,
    message: &SessionMessage,
) -> String {
    let prompt = user_prompt.unwrap_or(title);
    format!(
        "- [ ] {}\n  - source: conversation `{}` / message `{}`\n  - prompt: {}\n  - filed answer: {}\n",
        title.trim(),
        message.session_id,
        message.id,
        one_line(prompt),
        one_line(answer),
    )
}

fn moc_with_filed_conversation(path: &str, filed_block: &str) -> String {
    let title = path
        .rsplit('/')
        .next()
        .unwrap_or("Conversation MOC")
        .trim_end_matches(".md")
        .trim();
    format!(
        "---\ntype: moc\nconfidence: medium\n---\n\n# {}\n\n## For Agent\n\nThis MOC collects conversation filings.\n\n## Compiled Truth\n\n## Timeline\n\n## Evidence\n\n## Open Questions\n\n## Related\n\n## Filed Conversations\n\n{}",
        title,
        filed_block.trim(),
    )
}

fn append_under_section(content: &str, section: &str, addition: &str) -> String {
    let heading = format!("## {section}");
    let mut pos = 0usize;
    let mut section_start = None;
    let mut section_end = content.len();
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == heading {
            section_start = Some(pos + line.len());
        } else if section_start.is_some() && trimmed.starts_with("## ") {
            section_end = pos;
            break;
        }
        pos += line.len();
    }

    let addition = addition.trim();
    let mut out = String::new();
    if section_start.is_some() {
        out.push_str(&content[..section_end].trim_end());
        out.push_str("\n\n");
        out.push_str(addition);
        out.push('\n');
        out.push_str(&content[section_end..]);
    } else {
        out.push_str(content.trim_end());
        out.push_str("\n\n");
        out.push_str(&heading);
        out.push_str("\n\n");
        out.push_str(addition);
        out.push('\n');
    }
    out
}

fn quote_markdown(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn one_line(text: &str) -> String {
    crate::truncate_utf8(&text.split_whitespace().collect::<Vec<_>>().join(" "), 240).to_string()
}

fn yaml_inline(text: &str) -> String {
    format!("\"{}\"", yaml_escape(&one_line(text)))
}

async fn generate_summary(
    kb_id: &str,
    source_meta: &KnowledgeSource,
    content: &str,
    related: &[String],
) -> Result<(String, Option<String>)> {
    let config = crate::config::cached_config();
    let compile_cfg = config.knowledge_compile.clone().normalized();
    let override_chain = compile_cfg.model_override.clone().or_else(|| {
        compile_cfg
            .agent_id
            .as_deref()
            .and_then(|id| crate::automation::resolve_legacy_agent_chain(&config, id))
    });
    let chain = crate::automation::effective_chain(&config, override_chain);
    let prompt = summary_prompt(kb_id, source_meta, content, related);
    let fut = crate::automation::run(crate::automation::ModelTaskSpec {
        purpose: "knowledge.compile",
        chain,
        session_key: "automation:knowledge_compile",
        instruction: &prompt,
        max_tokens: LLM_MAX_TOKENS,
    });
    let res = tokio::time::timeout(std::time::Duration::from_secs(LLM_TIMEOUT_SECS), fut)
        .await
        .map_err(|_| anyhow!("compile LLM call timed out"))??;
    // Label the candidate that actually produced `res.text` — not
    // necessarily `chain[0]` if a fallback fired mid-call.
    let model = crate::automation::model_label(&config, &res.model);
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
