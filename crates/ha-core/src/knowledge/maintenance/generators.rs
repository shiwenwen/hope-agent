//! Proposal generators (WS6). Each task scans a KB and returns `NewProposal`s;
//! the orchestrator persists them as drafts (dedup by fingerprint). Deterministic
//! tasks (auto-link / orphan / frontmatter / dedup / knowledge-gap) run in one
//! `spawn_blocking` index+file scan; the LLM tasks (auto-tag / MOC / memory→note)
//! run async via the shared analysis agent. Everything is capped to keep cycles
//! cheap and the review queue small.

use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

use super::config::MaintenanceConfig;
use super::types::{NewProposal, ProposalAction, ProposalKind, ProposalStatus};
use crate::knowledge::{index, service};

/// Max notes scanned per deterministic task (bounds work on large KBs).
const SCAN_CAP: usize = 400;
/// Max proposals each individual task may emit per cycle (variety > volume).
const PER_TASK_CAP: usize = 8;
/// Body chars fed to an LLM per note.
const EXCERPT_CHARS: usize = 1200;

/// Generate proposals for one KB: deterministic scan (blocking) + LLM tasks.
/// `run_global` gates KB-independent tasks (memory→note distils the *global*
/// memory store) so they run once per cycle, not once per KB.
pub async fn generate(
    kb_id: &str,
    cfg: &MaintenanceConfig,
    run_global: bool,
) -> Result<Vec<NewProposal>> {
    let mut out: Vec<NewProposal> = Vec::new();

    let kb = kb_id.to_string();
    let cfg_clone = cfg.clone();
    let deterministic =
        tokio::task::spawn_blocking(move || generate_deterministic(&kb, &cfg_clone))
            .await
            .map_err(|e| anyhow!("maintenance scan task panicked: {e}"))??;
    out.extend(deterministic);

    // The LLM tasks cost a side_query each — skip them once the cheap deterministic
    // tasks have already filled this KB's budget (otherwise we'd pay for proposals
    // that get truncated away below).
    let budget_left = |out: &[NewProposal]| out.len() < cfg.max_proposals_per_cycle;

    if cfg.tasks.auto_tag && budget_left(&out) {
        match gen_auto_tag(kb_id, cfg).await {
            Ok(p) => out.extend(p),
            Err(e) => app_warn!("knowledge", "maintenance::auto_tag", "skipped: {}", e),
        }
    }
    if cfg.tasks.moc_upkeep && budget_left(&out) {
        match gen_moc_upkeep(kb_id, cfg).await {
            Ok(p) => out.extend(p),
            Err(e) => app_warn!("knowledge", "maintenance::moc_upkeep", "skipped: {}", e),
        }
    }
    if cfg.tasks.memory_to_note && run_global && budget_left(&out) {
        match gen_memory_to_note(kb_id, cfg).await {
            Ok(p) => out.extend(p),
            Err(e) => app_warn!("knowledge", "maintenance::memory_to_note", "skipped: {}", e),
        }
    }
    if cfg.tasks.source_conflict && budget_left(&out) {
        match gen_source_conflict(kb_id, cfg).await {
            Ok(p) => out.extend(p),
            Err(e) => app_warn!(
                "knowledge",
                "maintenance::source_conflict",
                "skipped: {}",
                e
            ),
        }
    }

    out.truncate(cfg.max_proposals_per_cycle);
    Ok(out)
}

/// Sync deterministic tasks. Runs inside `spawn_blocking`.
fn generate_deterministic(kb_id: &str, cfg: &MaintenanceConfig) -> Result<Vec<NewProposal>> {
    let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
    let mut out = Vec::new();
    if cfg.tasks.source_compile {
        out.extend(gen_source_compile(kb_id)?);
    }
    if cfg.tasks.frontmatter_fill {
        out.extend(gen_frontmatter_fill(&db, kb_id)?);
    }
    if cfg.tasks.for_agent_summary {
        out.extend(gen_for_agent_summary(&db, kb_id)?);
    }
    if cfg.tasks.open_questions_moc {
        out.extend(gen_open_questions_moc(&db, kb_id)?);
    }
    if cfg.tasks.auto_link {
        out.extend(gen_auto_link(&db, kb_id)?);
    }
    if cfg.tasks.orphan_rescue {
        out.extend(gen_orphan_rescue(&db, kb_id)?);
    }
    if cfg.tasks.knowledge_gap {
        out.extend(gen_knowledge_gap(&db, kb_id)?);
    }
    if cfg.tasks.dedup_merge {
        out.extend(gen_dedup_merge(&db, kb_id, cfg.dedup_similarity)?);
    }
    Ok(out)
}

// ── Deterministic tasks ──────────────────────────────────────────────

/// Notes whose frontmatter lacks a `title` → propose adding one.
fn gen_frontmatter_fill(db: &crate::knowledge::IndexDb, kb_id: &str) -> Result<Vec<NewProposal>> {
    let mut out = Vec::new();
    for note in db.list_notes(kb_id)?.into_iter().take(SCAN_CAP) {
        if out.len() >= PER_TASK_CAP {
            break;
        }
        if frontmatter_has_key(note.frontmatter_json.as_deref(), "title") {
            continue;
        }
        let title = note.title.trim();
        if title.is_empty() {
            continue;
        }
        let mut props = Map::new();
        props.insert("title".to_string(), Value::String(title.to_string()));
        out.push(NewProposal {
            kind: ProposalKind::FrontmatterFill,
            title: format!("Add title to frontmatter of {}", note.rel_path),
            detail: format!("Set `title: {title}` in the YAML frontmatter."),
            action: ProposalAction::SetFrontmatter {
                path: note.rel_path.clone(),
                props,
            },
            fingerprint: format!("frontmatter_fill:{}", note.rel_path),
        });
    }
    Ok(out)
}

/// Raw sources that have not been compiled, or were re-extracted after compile.
fn gen_source_compile(kb_id: &str) -> Result<Vec<NewProposal>> {
    let reg = crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))?;
    let mut sources = reg.list_sources(kb_id)?;
    sources.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let covered = covered_source_compile_keys(kb_id)?;

    let mut out = Vec::new();
    let uncompiled = sources
        .iter()
        .filter(|s| s.compiled_at.is_none())
        .filter(|s| !covered.contains(&source_compile_key("uncompiled", s)))
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !uncompiled.is_empty() {
        out.push(source_compile_proposal(
            &uncompiled,
            "Compile uncompiled sources",
            "These raw sources have never produced compile-review proposals.",
            "uncompiled",
        ));
    }

    let stale = sources
        .iter()
        .filter(|s| {
            s.compiled_at
                .map(|compiled| s.updated_at > compiled)
                .unwrap_or(false)
        })
        .filter(|s| !covered.contains(&source_compile_key("stale", s)))
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !stale.is_empty() {
        out.push(source_compile_proposal(
            &stale,
            "Recompile updated sources",
            "These raw sources changed after their last compile timestamp.",
            "stale",
        ));
    }
    out.truncate(PER_TASK_CAP);
    Ok(out)
}

fn covered_source_compile_keys(kb_id: &str) -> Result<HashSet<(String, String, String)>> {
    let reg = crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))?;
    let mut covered = HashSet::new();
    for proposal in reg.list_proposals(kb_id, None)? {
        let covers_source_compile = proposal.kind == ProposalKind::SourceCompile
            && proposal.status != ProposalStatus::Failed;
        if !covers_source_compile {
            continue;
        }
        covered.extend(source_compile_fingerprint_keys(&proposal.fingerprint));
    }
    Ok(covered)
}

fn source_compile_key(
    bucket: &str,
    source: &crate::knowledge::types::KnowledgeSource,
) -> (String, String, String) {
    (
        bucket.to_string(),
        source.id.clone(),
        source.content_hash.clone(),
    )
}

fn source_compile_fingerprint_keys(fingerprint: &str) -> Vec<(String, String, String)> {
    let Some(rest) = fingerprint.strip_prefix("source_compile:") else {
        return Vec::new();
    };
    let Some((bucket, items)) = rest.split_once(':') else {
        return Vec::new();
    };
    items
        .split('|')
        .filter_map(|item| {
            let (source_id, content_hash) = item.rsplit_once(':')?;
            if source_id.is_empty() || content_hash.is_empty() {
                return None;
            }
            Some((
                bucket.to_string(),
                source_id.to_string(),
                content_hash.to_string(),
            ))
        })
        .collect()
}

fn source_compile_proposal(
    sources: &[crate::knowledge::types::KnowledgeSource],
    title_prefix: &str,
    reason: &str,
    bucket: &str,
) -> NewProposal {
    let source_ids = sources.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    let source_list = sources
        .iter()
        .map(|s| format!("`{}` ({})", s.title, s.id))
        .collect::<Vec<_>>()
        .join(", ");
    let fingerprint_input = sources
        .iter()
        .map(|s| format!("{}:{}", s.id, s.content_hash))
        .collect::<Vec<_>>()
        .join("|");
    NewProposal {
        kind: ProposalKind::SourceCompile,
        title: format!("{title_prefix} ({})", sources.len()),
        detail: format!("{reason} Sources: {source_list}."),
        action: ProposalAction::CompileSources {
            source_ids,
            reason: reason.to_string(),
        },
        fingerprint: format!("source_compile:{bucket}:{fingerprint_input}"),
    }
}

/// Schema/compiled notes missing a useful `For Agent` section.
fn gen_for_agent_summary(db: &crate::knowledge::IndexDb, kb_id: &str) -> Result<Vec<NewProposal>> {
    let mut out = Vec::new();
    for note in db.list_notes(kb_id)?.into_iter().take(SCAN_CAP) {
        if out.len() >= PER_TASK_CAP {
            break;
        }
        let schema_candidate = frontmatter_has_key(note.frontmatter_json.as_deref(), "type")
            || frontmatter_has_key(note.frontmatter_json.as_deref(), "last_compiled")
            || frontmatter_has_key(note.frontmatter_json.as_deref(), "sources");
        if !schema_candidate {
            continue;
        }
        let Ok(read) = service::note_read(kb_id, &note.rel_path) else {
            continue;
        };
        if section_body(&read.content, "For Agent")
            .map(has_actionable_section_text)
            .unwrap_or(false)
        {
            continue;
        }
        let summary = first_prose_line(&read.content)
            .unwrap_or_else(|| "No concise summary was present yet.".to_string());
        let body = format!(
            "- Scope: {}.\n- Summary: {}\n- Use the Evidence section to verify source-derived claims before acting on them.",
            note.title.trim(),
            summary
        );
        let updated = upsert_section(&read.content, "For Agent", &body);
        if updated == read.content {
            continue;
        }
        out.push(NewProposal {
            kind: ProposalKind::ForAgentSummary,
            title: format!("Add For Agent summary to {}", note.rel_path),
            detail: "Create a concise agent-facing summary section for this compiled/schema note."
                .to_string(),
            action: ProposalAction::PatchNote {
                path: note.rel_path.clone(),
                expected_hash: read.content_hash,
                content: updated,
            },
            fingerprint: format!("for_agent_summary:{}:{}", note.rel_path, note.content_hash),
        });
    }
    Ok(out)
}

/// Many notes with unresolved Open Questions -> create a reviewable MOC hub.
fn gen_open_questions_moc(db: &crate::knowledge::IndexDb, kb_id: &str) -> Result<Vec<NewProposal>> {
    let rel = "MOCs/Open Questions.md";
    if db.get_note_by_rel_path(kb_id, rel)?.is_some() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<(String, String, Vec<String>)> = Vec::new();
    for note in db.list_notes(kb_id)?.into_iter().take(SCAN_CAP) {
        let Ok(read) = service::note_read(kb_id, &note.rel_path) else {
            continue;
        };
        let Some(body) = section_body(&read.content, "Open Questions") else {
            continue;
        };
        let questions = question_lines(body);
        if !questions.is_empty() {
            entries.push((note.rel_path.clone(), note.title.clone(), questions));
        }
        if entries.len() >= 40 {
            break;
        }
    }
    if entries.len() < 3 {
        return Ok(Vec::new());
    }
    let mut list = String::new();
    for (path, title, qs) in entries.iter().take(40) {
        list.push_str(&format!("- [[{}|{}]]\n", rel_no_md(path), title));
        for q in qs.iter().take(3) {
            list.push_str(&format!("  - [ ] {}\n", q));
        }
    }
    let content = format!(
        "---\ntitle: Open Questions\nmoc: true\ntype: moc\nconfidence: medium\n---\n\n# Open Questions\n\n## For Agent\n\nUse this hub to triage unresolved questions across the knowledge space. Each item links back to the note where the question was filed.\n\n## Compiled Truth\n\n{} notes currently contain unresolved Open Questions.\n\n## Timeline\n\n## Evidence\n\nGenerated by autonomous maintenance from note Open Questions sections.\n\n## Open Questions\n\n{}\n## Related\n",
        entries.len(),
        list
    );
    Ok(vec![NewProposal {
        kind: ProposalKind::OpenQuestionsMoc,
        title: format!("Create Open Questions MOC ({} notes)", entries.len()),
        detail: "Collect recurring unresolved questions into a single MOC for review.".to_string(),
        action: ProposalAction::CreateNote {
            path: rel.to_string(),
            content,
            overwrite: false,
        },
        fingerprint: format!(
            "open_questions_moc:{}",
            entries
                .iter()
                .take(40)
                .map(|(p, _, _)| p.as_str())
                .collect::<Vec<_>>()
                .join("|")
        ),
    }])
}

/// Notes that mention another note's title in prose but don't link it.
fn gen_auto_link(db: &crate::knowledge::IndexDb, kb_id: &str) -> Result<Vec<NewProposal>> {
    let notes = db.list_notes(kb_id)?;
    // Map lookup term → canonical ref (title), skipping short/ambiguous terms.
    // Cap the candidate-title set: the scan is O(scanned_notes × terms × body), so
    // an unbounded term list would blow up on a huge KB.
    const MAX_LINK_TERMS: usize = 2000;
    let mut terms: Vec<(String, String)> = Vec::new(); // (lowercased term, ref to insert)
    for n in &notes {
        if terms.len() >= MAX_LINK_TERMS {
            break;
        }
        let t = n.title.trim();
        if t.chars().count() >= 4 {
            terms.push((t.to_lowercase(), t.to_string()));
        }
    }
    let mut out = Vec::new();
    for note in notes.iter().take(SCAN_CAP) {
        if out.len() >= PER_TASK_CAP {
            break;
        }
        let read = match service::note_read(kb_id, &note.rel_path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let body = strip_code_and_links(&read.content);
        let already: HashSet<String> = read
            .outgoing_links
            .iter()
            .map(|l| l.target_ref.trim().to_lowercase())
            .collect();
        for (term_lc, term_ref) in &terms {
            if term_ref.eq_ignore_ascii_case(&note.title) {
                continue; // don't link a note to itself
            }
            if already.contains(term_lc) {
                continue;
            }
            if contains_word(&body, term_lc) {
                out.push(NewProposal {
                    kind: ProposalKind::AutoLink,
                    title: format!("Link “{}” → “{}”", note.title, term_ref),
                    detail: format!(
                        "“{}” is mentioned in {} but not linked. Append a `[[{}]]` link.",
                        term_ref, note.rel_path, term_ref
                    ),
                    action: ProposalAction::AppendLink {
                        from_path: note.rel_path.clone(),
                        to_ref: term_ref.clone(),
                    },
                    fingerprint: format!("auto_link:{}=>{}", note.rel_path, term_lc),
                });
                break; // one suggestion per note per cycle keeps the queue varied
            }
        }
    }
    Ok(out)
}

/// Orphan notes (no resolved in/out links) → link them from a tag-relative.
fn gen_orphan_rescue(db: &crate::knowledge::IndexDb, kb_id: &str) -> Result<Vec<NewProposal>> {
    let mut out = Vec::new();
    let kb_ids = [kb_id.to_string()];
    for orphan in db.list_orphan_notes(kb_id)?.into_iter().take(SCAN_CAP) {
        if out.len() >= PER_TASK_CAP {
            break;
        }
        let tags = db.tags_for_note(orphan.id).unwrap_or_default();
        // Path-form ref (drop `.md`) so the link resolves to THIS orphan, not some
        // other note that happens to share its title.
        let orphan_ref = rel_no_md(&orphan.rel_path);
        let mut linked = false;
        for tag in &tags {
            let Ok(relatives) = db.notes_by_tag(&kb_ids, tag) else {
                continue;
            };
            for rel in relatives {
                if rel.id == orphan.id {
                    continue;
                }
                // Link the orphan FROM a relative so it gains a backlink.
                out.push(NewProposal {
                    kind: ProposalKind::OrphanRescue,
                    title: format!("Rescue orphan “{}”", orphan.title),
                    detail: format!(
                        "“{}” has no links. Append a `[[{}]]` link from “{}” (shares #{}).",
                        orphan.title, orphan_ref, rel.title, tag
                    ),
                    action: ProposalAction::AppendLink {
                        from_path: rel.rel_path.clone(),
                        to_ref: orphan_ref.clone(),
                    },
                    fingerprint: format!("orphan_rescue:{}", orphan.rel_path),
                });
                linked = true;
                break;
            }
            if linked {
                break;
            }
        }
    }
    Ok(out)
}

/// Frequently-referenced broken `[[ ]]` targets → propose creating a stub note.
fn gen_knowledge_gap(db: &crate::knowledge::IndexDb, kb_id: &str) -> Result<Vec<NewProposal>> {
    let broken = db.list_broken_links(kb_id)?;
    // Count references per normalized target; collect referring titles.
    let mut counts: HashMap<String, (String, usize, Vec<String>)> = HashMap::new();
    for b in &broken {
        let target = clean_ref(&b.target_ref);
        if target.is_empty() {
            continue;
        }
        let key = target.to_lowercase();
        let e = counts
            .entry(key)
            .or_insert_with(|| (target.clone(), 0, Vec::new()));
        e.1 += 1;
        if e.2.len() < 8 {
            e.2.push(b.src_title.clone());
        }
    }
    let mut ranked: Vec<_> = counts.into_values().filter(|(_, n, _)| *n >= 2).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut out = Vec::new();
    // Cap AFTER the "already exists" filter, not before, so existing targets don't
    // silently eat the budget and shrink output.
    for (target, count, refs) in ranked {
        if out.len() >= PER_TASK_CAP {
            break;
        }
        let rel = gap_target_path(&target);
        // Don't propose creating something that already resolves now (skip on a
        // lookup error rather than aborting the whole task).
        if matches!(db.get_note_by_rel_path(kb_id, &rel), Ok(Some(_)) | Err(_)) {
            continue;
        }
        let refs_md = refs
            .iter()
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!(
            "---\ntitle: {}\n---\n\n# {}\n\n> Auto-created stub to resolve {} broken reference(s).\n\nReferenced by:\n\n{}\n",
            yaml_scalar(&target),
            target,
            count,
            refs_md
        );
        out.push(NewProposal {
            kind: ProposalKind::KnowledgeGap,
            title: format!("Create missing note “{target}” ({count} refs)"),
            detail: format!(
                "{count} notes link to “{target}” but it doesn't exist. Create a stub at {rel}."
            ),
            action: ProposalAction::CreateNote {
                path: rel,
                content,
                overwrite: false,
            },
            fingerprint: format!("knowledge_gap:{}", target.to_lowercase()),
        });
    }
    Ok(out)
}

/// Near-duplicate notes (identical hash or high title-token Jaccard) → merge.
fn gen_dedup_merge(
    db: &crate::knowledge::IndexDb,
    kb_id: &str,
    similarity: f32,
) -> Result<Vec<NewProposal>> {
    let notes: Vec<_> = db.list_notes(kb_id)?.into_iter().take(SCAN_CAP).collect();
    let tokens: Vec<HashSet<String>> = notes.iter().map(|n| title_tokens(&n.title)).collect();
    let mut out = Vec::new();
    let mut used: HashSet<i64> = HashSet::new();
    'outer: for i in 0..notes.len() {
        if used.contains(&notes[i].id) {
            continue;
        }
        for j in (i + 1)..notes.len() {
            if used.contains(&notes[j].id) {
                continue;
            }
            let dup = notes[i].content_hash == notes[j].content_hash
                || jaccard(&tokens[i], &tokens[j]) >= similarity;
            if !dup {
                continue;
            }
            // Keep the larger note; merge the smaller in.
            let (keep, drop) = if notes[i].size >= notes[j].size {
                (&notes[i], &notes[j])
            } else {
                (&notes[j], &notes[i])
            };
            // Reading both bodies can fail (file vanished) — skip the pair, don't
            // abort the whole task.
            let (keep_read, drop_read) = match (
                service::note_read(kb_id, &keep.rel_path),
                service::note_read(kb_id, &drop.rel_path),
            ) {
                (Ok(k), Ok(d)) => (k, d),
                _ => continue,
            };
            // Exact duplicate (identical raw bytes): keep one copy verbatim and just
            // delete the other — appending would double the content. Near-duplicate
            // (title-similar, different body): concatenate so no body is lost.
            let exact_dup = keep.content_hash == drop.content_hash;
            let (merged, detail) = if exact_dup {
                (
                    keep_read.content.clone(),
                    format!(
                        "“{}” and “{}” are identical. Delete the duplicate {} (keeping {}).",
                        keep.title, drop.title, drop.rel_path, keep.rel_path
                    ),
                )
            } else {
                (
                    format!(
                        "{}\n\n---\n\n## Merged from {}\n\n{}\n",
                        keep_read.content.trim_end(),
                        drop.rel_path,
                        strip_frontmatter(&drop_read.content).trim()
                    ),
                    format!(
                        "“{}” and “{}” look near-identical. Merge into {} and delete {}. (The merged-in note's frontmatter is dropped — review the diff.)",
                        keep.title, drop.title, keep.rel_path, drop.rel_path
                    ),
                )
            };
            out.push(NewProposal {
                kind: ProposalKind::DedupMerge,
                title: if exact_dup {
                    format!("Delete duplicate “{}”", drop.title)
                } else {
                    format!("Merge “{}” into “{}”", drop.title, keep.title)
                },
                detail,
                action: ProposalAction::MergeNotes {
                    keep_path: keep.rel_path.clone(),
                    keep_expected_hash: keep_read.content_hash.clone(),
                    keep_content: merged,
                    removes: vec![super::types::MergeRemove {
                        path: drop.rel_path.clone(),
                        expected_hash: drop_read.content_hash.clone(),
                    }],
                },
                fingerprint: {
                    let mut pair = [keep.rel_path.as_str(), drop.rel_path.as_str()];
                    pair.sort_unstable();
                    format!("dedup_merge:{}|{}", pair[0], pair[1])
                },
            });
            used.insert(keep.id);
            used.insert(drop.id);
            if out.len() >= PER_TASK_CAP.min(3) {
                break 'outer;
            }
            continue 'outer;
        }
    }
    Ok(out)
}

// ── LLM tasks ────────────────────────────────────────────────────────

/// Suggest tags for untagged notes (one batched side_query).
async fn gen_auto_tag(kb_id: &str, cfg: &MaintenanceConfig) -> Result<Vec<NewProposal>> {
    // (rel_path, title, excerpt) for up to 5 untagged notes — collected off the
    // async executor (SQLite + file reads are blocking).
    let kb = kb_id.to_string();
    let collected: Vec<(String, String, String)> = tokio::task::spawn_blocking(move || {
        let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
        let mut out = Vec::new();
        for n in db.list_notes(&kb)?.into_iter().take(SCAN_CAP) {
            if db.tags_for_note(n.id)?.is_empty() {
                if let Ok(read) = service::note_read(&kb, &n.rel_path) {
                    let excerpt = crate::truncate_utf8(&read.content, EXCERPT_CHARS).to_string();
                    out.push((n.rel_path, n.title, excerpt));
                }
                if out.len() >= 5 {
                    break;
                }
            }
        }
        Ok::<_, anyhow::Error>(out)
    })
    .await
    .map_err(|e| anyhow!("auto_tag scan panicked: {e}"))??;
    if collected.is_empty() {
        return Ok(Vec::new());
    }
    let mut blocks = String::new();
    for (rel, _title, excerpt) in &collected {
        blocks.push_str(&format!("### PATH: {rel}\n{excerpt}\n\n"));
    }
    let prompt = format!(
        "Suggest 1–4 lowercase topical tags for each note below. Return ONLY a JSON object \
         mapping each note's PATH to an array of tag strings (no `#`, no commentary, no code \
         fence).\n\n{blocks}"
    );
    let resp = run_side_query(&prompt, cfg).await?;
    let map: Map<String, Value> = parse_json_object(&resp)?;
    let mut out = Vec::new();
    for (rel, title, _excerpt) in &collected {
        let Some(arr) = map.get(rel).and_then(|v| v.as_array()) else {
            continue;
        };
        let tags: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(crate::knowledge::parser::normalize_tag)
            .filter(|t| !t.is_empty())
            .take(4)
            .collect();
        if tags.is_empty() {
            continue;
        }
        let mut props = Map::new();
        props.insert(
            "tags".to_string(),
            Value::Array(tags.iter().map(|t| Value::String(t.clone())).collect()),
        );
        out.push(NewProposal {
            kind: ProposalKind::AutoTag,
            title: format!("Tag “{title}”: {}", tags.join(", ")),
            detail: format!("Add tags [{}] to {rel}.", tags.join(", ")),
            action: ProposalAction::SetFrontmatter {
                path: rel.clone(),
                props,
            },
            fingerprint: format!("auto_tag:{rel}"),
        });
    }
    Ok(out)
}

/// Build a MOC for a busy tag that has no hub yet (one side_query per candidate).
async fn gen_moc_upkeep(kb_id: &str, cfg: &MaintenanceConfig) -> Result<Vec<NewProposal>> {
    // Collect up to 2 candidate tags (busy, no existing hub) + their note lists
    // off the async executor. (tag, count, rel, list_markdown)
    let kb = kb_id.to_string();
    let candidates: Vec<(String, u32, String, String)> = tokio::task::spawn_blocking(move || {
        let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
        let kb_ids = [kb.clone()];
        let mut tags = db.all_tags(&kb_ids)?;
        tags.sort_by_key(|t| std::cmp::Reverse(t.1));
        let mut cands = Vec::new();
        for (tag, count) in tags.into_iter() {
            if cands.len() >= 2 {
                break;
            }
            if count < 5 {
                break; // sorted desc — nothing busier left
            }
            let slug = slugify(&tag);
            let rel = format!("MOCs/{slug}.md");
            if db.get_note_by_rel_path(&kb, &rel)?.is_some() {
                continue; // already has a hub
            }
            let notes = db.notes_by_tag(&kb_ids, &tag)?;
            let list = notes
                .iter()
                .take(100)
                .map(|n| format!("- [[{}]] — {}", basename_no_md(&n.rel_path), n.title))
                .collect::<Vec<_>>()
                .join("\n");
            cands.push((tag, count, rel, list));
        }
        Ok::<_, anyhow::Error>(cands)
    })
    .await
    .map_err(|e| anyhow!("moc scan panicked: {e}"))??;

    let mut out = Vec::new();
    for (tag, count, rel, list) in candidates {
        let prompt = format!(
            "Create a concise Map-of-Content (MOC) hub note for the topic “#{tag}”. Below are the \
             notes tagged with it. Write a short intro, then group them into thematic sections with \
             a one-line annotation each, linking every note with the [[wikilinks]] exactly as given. \
             Return ONLY the markdown body (no frontmatter, no code fence).\n\nNOTES:\n{list}"
        );
        let body = strip_code_fence(&run_side_query(&prompt, cfg).await?);
        if body.trim().is_empty() {
            continue;
        }
        let content = format!(
            "---\ntitle: {}\nmoc: true\n---\n\n{}\n",
            yaml_scalar(&format!("{tag} (MOC)")),
            body.trim()
        );
        out.push(NewProposal {
            kind: ProposalKind::MocUpkeep,
            title: format!("Build MOC for #{tag} ({count} notes)"),
            detail: format!(
                "Create a Map-of-Content hub at {rel} linking the {count} notes tagged #{tag}."
            ),
            action: ProposalAction::CreateNote {
                path: rel,
                content,
                overwrite: false,
            },
            fingerprint: format!("moc_upkeep:{tag}"),
        });
    }
    Ok(out)
}

/// Distil recent un-pinned memories into a single topic note (one side_query).
async fn gen_memory_to_note(kb_id: &str, cfg: &MaintenanceConfig) -> Result<Vec<NewProposal>> {
    // Memory backend list is a blocking SQLite call — off the async executor.
    let fragments: Vec<String> = tokio::task::spawn_blocking(|| {
        let Some(backend) = crate::get_memory_backend() else {
            return Ok::<_, anyhow::Error>(Vec::new());
        };
        let entries = backend.list(None, None, 40, 0)?;
        Ok(entries
            .into_iter()
            .filter(|e| !e.pinned)
            .map(|e| e.content)
            .filter(|c| !c.trim().is_empty())
            .take(20)
            .collect())
    })
    .await
    .map_err(|e| anyhow!("memory scan panicked: {e}"))??;
    if fragments.len() < 5 {
        return Ok(Vec::new()); // not enough signal to be worth a note
    }
    let joined = fragments
        .iter()
        .map(|c| format!("- {}", crate::truncate_utf8(c, 280)))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "These are scattered memory fragments about the user and their work. If they share a clear \
         theme, distil them into ONE concise permanent note. Return ONLY JSON \
         {{\"title\": \"…\", \"body\": \"markdown…\"}} (no code fence). If there's no coherent theme, \
         return {{\"title\": \"\", \"body\": \"\"}}.\n\nFRAGMENTS:\n{joined}"
    );
    let resp = run_side_query(&prompt, cfg).await?;
    let obj = parse_json_object(&resp)?;
    let title = obj
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let body = obj
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if title.is_empty() || body.is_empty() {
        return Ok(Vec::new());
    }
    let slug = slugify(title);
    let rel = format!("Memory/{slug}.md");
    // Existence check off the async executor.
    let kb = kb_id.to_string();
    let rel_check = rel.clone();
    let exists = tokio::task::spawn_blocking(move || {
        index::get_index_db()
            .and_then(|db| db.get_note_by_rel_path(&kb, &rel_check).ok().flatten())
            .is_some()
    })
    .await
    .unwrap_or(false);
    if exists {
        return Ok(Vec::new());
    }
    let content = format!("---\ntitle: {}\n---\n\n{}\n", yaml_scalar(title), body);
    Ok(vec![NewProposal {
        kind: ProposalKind::MemoryToNote,
        title: format!("Distil memories into “{title}”"),
        detail: format!(
            "Turn {} memory fragments into a permanent note at {rel}.",
            fragments.len()
        ),
        action: ProposalAction::CreateNote {
            path: rel,
            content,
            overwrite: false,
        },
        fingerprint: format!("memory_to_note:{}", slug),
    }])
}

#[derive(Debug, Clone)]
struct SourceConflictCandidate {
    path: String,
    title: String,
    expected_hash: String,
    content: String,
    source_id: String,
    source_title: String,
    note_excerpt: String,
    source_excerpt: String,
}

/// Compare likely source/note pairs and file potential contradictions as review
/// questions, never as automatic fact rewrites.
async fn gen_source_conflict(kb_id: &str, cfg: &MaintenanceConfig) -> Result<Vec<NewProposal>> {
    let kb = kb_id.to_string();
    let candidates: Vec<SourceConflictCandidate> = tokio::task::spawn_blocking(move || {
        let db = index::get_index_db().ok_or_else(|| anyhow!("knowledge index not initialized"))?;
        let reg =
            crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))?;
        let notes = db
            .list_notes(&kb)?
            .into_iter()
            .filter(|n| !n.rel_path.starts_with("MOCs/"))
            .take(SCAN_CAP)
            .collect::<Vec<_>>();
        let note_tokens = notes
            .iter()
            .map(|n| title_tokens(&n.title))
            .collect::<Vec<_>>();

        let mut out = Vec::new();
        for source in reg.list_sources(&kb)?.into_iter().take(SCAN_CAP) {
            if out.len() >= 3 {
                break;
            }
            if !matches!(
                source.status,
                crate::knowledge::types::KnowledgeSourceStatus::Ready
            ) {
                continue;
            }
            let src_tokens = title_tokens(&source.title);
            if src_tokens.is_empty() {
                continue;
            }
            let Some((best_idx, best_score)) = note_tokens
                .iter()
                .enumerate()
                .map(|(idx, tokens)| (idx, jaccard(&src_tokens, tokens)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            else {
                continue;
            };
            if best_score < 0.34 {
                continue;
            }
            let note = &notes[best_idx];
            let Ok(note_read) = service::note_read(&kb, &note.rel_path) else {
                continue;
            };
            let Ok(source_read) = service::source_read(&kb, &source.id) else {
                continue;
            };
            out.push(SourceConflictCandidate {
                path: note.rel_path.clone(),
                title: note.title.clone(),
                expected_hash: note_read.content_hash,
                content: note_read.content.clone(),
                source_id: source.id.clone(),
                source_title: source.title.clone(),
                note_excerpt: crate::truncate_utf8(
                    &strip_frontmatter(&note_read.content),
                    EXCERPT_CHARS,
                )
                .to_string(),
                source_excerpt: crate::truncate_utf8(&source_read.content, EXCERPT_CHARS)
                    .to_string(),
            });
        }
        Ok::<_, anyhow::Error>(out)
    })
    .await
    .map_err(|e| anyhow!("source conflict scan panicked: {e}"))??;

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut blocks = String::new();
    for (idx, c) in candidates.iter().enumerate() {
        blocks.push_str(&format!(
            "### CANDIDATE {idx}\nNOTE_PATH: {}\nNOTE_TITLE: {}\nSOURCE_ID: {}\nSOURCE_TITLE: {}\n\nNOTE_EXCERPT:\n{}\n\nSOURCE_EXCERPT:\n{}\n\n",
            c.path, c.title, c.source_id, c.source_title, c.note_excerpt, c.source_excerpt
        ));
    }
    let prompt = format!(
        "You are auditing a personal knowledge base. For each candidate, decide whether the raw \
         SOURCE_EXCERPT likely contradicts the compiled NOTE_EXCERPT. Only flag concrete factual \
         conflicts, not missing details or style differences. Return ONLY JSON: \
         {{\"conflicts\":[{{\"candidate\":0,\"summary\":\"...\",\"evidence\":\"...\"}}]}}. \
         If none are real conflicts, return {{\"conflicts\":[]}}.\n\n{blocks}"
    );
    let resp = run_side_query(&prompt, cfg).await?;
    let obj = parse_json_object(&resp)?;
    let Some(items) = obj.get("conflicts").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for item in items.iter().take(PER_TASK_CAP) {
        let Some(idx) = item
            .get("candidate")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
        else {
            continue;
        };
        let Some(c) = candidates.get(idx) else {
            continue;
        };
        let summary = item
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(summary) = summary else {
            continue;
        };
        let evidence = item
            .get("evidence")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Review the source and compiled note before updating the claim.");
        let block = format!(
            "- [ ] Possible conflict with source `{}` ({}): {}\n  - evidence: {}\n",
            c.source_id,
            c.source_title,
            one_line(summary),
            one_line(evidence),
        );
        let updated = append_under_section(&c.content, "Open Questions", &block);
        out.push(NewProposal {
            kind: ProposalKind::SourceConflict,
            title: format!("Review possible source conflict in {}", c.path),
            detail: format!(
                "Source `{}` may contradict compiled note `{}`. Apply to file a review item under Open Questions.",
                c.source_title, c.title
            ),
            action: ProposalAction::PatchNote {
                path: c.path.clone(),
                expected_hash: c.expected_hash.clone(),
                content: updated,
            },
            fingerprint: format!(
                "source_conflict:{}:{}:{}",
                c.path,
                c.source_id,
                crate::knowledge::blake3_hex(summary.as_bytes())
            ),
        });
    }
    Ok(out)
}

// ── LLM helper ───────────────────────────────────────────────────────

async fn run_side_query(prompt: &str, cfg: &MaintenanceConfig) -> Result<String> {
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    let fut = agent.side_query(prompt, cfg.llm_max_tokens);
    let res = tokio::time::timeout(std::time::Duration::from_secs(cfg.llm_timeout_secs), fut)
        .await
        .map_err(|_| anyhow!("maintenance LLM call timed out"))??;
    Ok(res.text.trim().to_string())
}

// ── Text helpers ─────────────────────────────────────────────────────

/// Does `frontmatter_json` (a serialized object) contain a non-empty `key`?
fn frontmatter_has_key(json: Option<&str>, key: &str) -> bool {
    let Some(s) = json else { return false };
    serde_json::from_str::<Value>(s)
        .ok()
        .and_then(|v| v.get(key).cloned())
        .map(|v| !matches!(v, Value::Null) && v.as_str() != Some(""))
        .unwrap_or(false)
}

/// Lowercased alphanumeric word set of a title for Jaccard similarity.
fn title_tokens(title: &str) -> HashSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    inter / union
}

/// ASCII word-boundary match for `needle` (lowercased) in `haystack`; CJK falls
/// back to substring (no word boundaries). Mirrors the suggest-links contract.
fn contains_word(haystack_lower: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return false;
    }
    let is_cjk = needle_lower
        .chars()
        .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if is_cjk {
        return haystack_lower.contains(needle_lower);
    }
    let bytes = haystack_lower.as_bytes();
    let nb = needle_lower.as_bytes();
    let mut from = 0;
    while let Some(pos) = haystack_lower[from..].find(needle_lower) {
        let start = from + pos;
        let end = start + nb.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Lowercased body with fenced + inline code and `[[wikilinks]]` removed — what we
/// scan for unlinked mentions. Markdown `[text](url)` link *text* is left as prose
/// (a title appearing only inside a URL is a rare false positive the review catches).
fn strip_code_and_links(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Drop inline code spans and `[[wikilink]]` targets.
    let mut s = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                for n in chars.by_ref() {
                    if n == '`' {
                        break;
                    }
                }
            }
            '[' if chars.peek() == Some(&'[') => {
                // wikilink: skip until ]]
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == ']' && n == ']' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => s.push(c),
        }
    }
    s.to_lowercase()
}

/// Drop a leading YAML frontmatter block.
fn strip_frontmatter(content: &str) -> String {
    let mut lines = content.lines();
    if lines.next().map(|l| l.trim_end()) != Some("---") {
        return content.to_string();
    }
    let mut body = Vec::new();
    let mut closed = false;
    for l in lines {
        if !closed && l.trim_end() == "---" {
            closed = true;
            continue;
        }
        if closed {
            body.push(l);
        }
    }
    if !closed {
        return content.to_string();
    }
    body.join("\n").trim_start().to_string()
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

fn has_actionable_section_text(body: &str) -> bool {
    body.lines().any(|line| {
        let t = line
            .trim()
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim();
        !t.is_empty()
            && !matches!(
                t,
                "None"
                    | "none"
                    | "N/A"
                    | "n/a"
                    | "无"
                    | "暂无"
                    | "No concise summary was present yet."
            )
    })
}

fn question_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            if line
                .chars()
                .next()
                .map(char::is_whitespace)
                .unwrap_or(false)
            {
                return None;
            }
            let raw = line.trim();
            let lower = raw.to_ascii_lowercase();
            if lower.starts_with("- [x]") || lower.starts_with("* [x]") {
                return None;
            }
            let t = raw
                .trim_start_matches("- [ ]")
                .trim_start_matches("* [ ]")
                .trim_start_matches("- [x]")
                .trim_start_matches("* [x]")
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim();
            if t.is_empty() || matches!(t, "None" | "none" | "N/A" | "n/a" | "无" | "暂无") {
                None
            } else {
                Some(crate::truncate_utf8(t, 180).to_string())
            }
        })
        .take(5)
        .collect()
}

fn first_prose_line(content: &str) -> Option<String> {
    strip_frontmatter(content)
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("---")
                && !line.starts_with("source:")
                && !line.starts_with("Source:")
        })
        .map(|line| {
            crate::truncate_utf8(
                line.trim_start_matches("- ")
                    .trim_start_matches("* ")
                    .trim_start_matches("> ")
                    .trim(),
                220,
            )
            .to_string()
        })
        .find(|line| !line.is_empty())
}

fn upsert_section(content: &str, section: &str, body: &str) -> String {
    let heading = format!("## {section}");
    let body = body.trim();
    let mut pos = 0usize;
    let mut section_start = None;
    let mut section_end = content.len();
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == heading {
            section_start = Some(pos);
        } else if section_start.is_some() && trimmed.starts_with("## ") {
            section_end = pos;
            break;
        }
        pos += line.len();
    }
    if let Some(start) = section_start {
        let mut out = String::new();
        out.push_str(content[..start].trim_end());
        out.push_str("\n\n");
        out.push_str(&heading);
        out.push_str("\n\n");
        out.push_str(body);
        out.push('\n');
        out.push_str(content[section_end..].trim_start_matches('\n'));
        return out;
    }

    let insert_at = insertion_after_frontmatter_and_h1(content);
    let mut out = String::new();
    out.push_str(content[..insert_at].trim_end());
    out.push_str("\n\n");
    out.push_str(&heading);
    out.push_str("\n\n");
    out.push_str(body);
    out.push_str("\n\n");
    out.push_str(content[insert_at..].trim_start());
    out
}

fn append_under_section(content: &str, section: &str, addition: &str) -> String {
    let current = section_body(content, section).unwrap_or("");
    let body = if has_actionable_section_text(current) {
        format!("{}\n\n{}", current.trim_end(), addition.trim())
    } else {
        addition.trim().to_string()
    };
    upsert_section(content, section, &body)
}

fn insertion_after_frontmatter_and_h1(content: &str) -> usize {
    let mut offset = 0usize;
    let mut in_frontmatter = content.trim_start().starts_with("---");
    let mut seen_frontmatter_open = false;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        offset += line.len();
        if in_frontmatter {
            if !seen_frontmatter_open && trimmed == "---" {
                seen_frontmatter_open = true;
                continue;
            }
            if seen_frontmatter_open && trimmed == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        if trimmed.starts_with("# ") {
            return offset;
        }
        if !trimmed.is_empty() {
            break;
        }
    }
    offset.min(content.len())
}

fn one_line(value: &str) -> String {
    crate::truncate_utf8(&value.split_whitespace().collect::<Vec<_>>().join(" "), 240).to_string()
}

fn clean_ref(raw: &str) -> String {
    raw.split('|')
        .next()
        .unwrap_or(raw)
        .split('#')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_string()
}

/// KB-relative path for a knowledge-gap target ref: keep an explicit path, else
/// drop a stub at the KB root.
fn gap_target_path(target: &str) -> String {
    let t = target.replace('\\', "/");
    if t.to_lowercase().ends_with(".md") || t.to_lowercase().ends_with(".markdown") {
        t
    } else if t.contains('/') {
        format!("{t}.md")
    } else {
        format!("{}.md", t.trim())
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "note".to_string()
    } else {
        s
    }
}

fn basename_no_md(rel: &str) -> String {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    base.strip_suffix(".md")
        .or_else(|| base.strip_suffix(".markdown"))
        .unwrap_or(base)
        .to_string()
}

/// Full KB-relative path with the `.md` extension dropped — a path-form wikilink
/// ref that resolves to exactly one note (resolver: path-form > basename).
fn rel_no_md(rel: &str) -> String {
    rel.strip_suffix(".md")
        .or_else(|| rel.strip_suffix(".markdown"))
        .unwrap_or(rel)
        .to_string()
}

/// Quote a YAML scalar if it could be misparsed.
fn yaml_scalar(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.starts_with(|c: char| c.is_whitespace())
        || s.ends_with(|c: char| c.is_whitespace())
        || s.contains(['#', ':', '"', '\'', '\n', '[', ']', '{', '}']);
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn strip_code_fence(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some((info, body)) = rest.split_once('\n') {
            let info = info.trim().to_ascii_lowercase();
            if (info.is_empty() || info == "markdown" || info == "md") && body.ends_with("```") {
                return body.strip_suffix("```").unwrap_or(body).trim().to_string();
            }
        }
    }
    t.to_string()
}

/// Parse the outermost JSON object from an LLM reply (fence/prose tolerant).
fn parse_json_object(text: &str) -> Result<Map<String, Value>> {
    let t = text.trim();
    let start = t
        .find('{')
        .ok_or_else(|| anyhow!("no JSON object in reply"))?;
    let end = t
        .rfind('}')
        .ok_or_else(|| anyhow!("no JSON object in reply"))?;
    if end <= start {
        return Err(anyhow!("malformed JSON object in reply"));
    }
    let v: Value = serde_json::from_str(&t[start..=end])?;
    match v {
        Value::Object(m) => Ok(m),
        _ => Err(anyhow!("expected a JSON object")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_word_respects_boundaries() {
        assert!(contains_word("see the foo bar", "foo"));
        assert!(!contains_word("food court", "foo"));
        assert!(contains_word("项目笔记很多", "项目")); // CJK substring
    }

    #[test]
    fn jaccard_basic() {
        let a = title_tokens("Project Alpha Notes");
        let b = title_tokens("Project Alpha Note");
        assert!(jaccard(&a, &b) > 0.4);
        let c = title_tokens("Totally Different");
        assert_eq!(jaccard(&a, &c), 0.0);
    }

    #[test]
    fn strip_frontmatter_drops_block() {
        assert_eq!(strip_frontmatter("---\ntitle: A\n---\n\nbody"), "body");
        assert_eq!(strip_frontmatter("no fm\nbody"), "no fm\nbody");
    }

    #[test]
    fn slugify_handles_unicode_and_symbols() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  "), "note");
    }

    #[test]
    fn frontmatter_has_key_detects_title() {
        assert!(frontmatter_has_key(Some("{\"title\":\"X\"}"), "title"));
        assert!(!frontmatter_has_key(Some("{\"title\":\"\"}"), "title"));
        assert!(!frontmatter_has_key(None, "title"));
    }

    #[test]
    fn upsert_section_inserts_after_frontmatter_and_h1() {
        let updated = upsert_section(
            "---\ntitle: A\n---\n\n# A\n\nBody",
            "For Agent",
            "- Summary",
        );
        assert!(updated.contains("# A\n\n## For Agent\n\n- Summary\n\nBody"));
        assert!(updated.starts_with("---\ntitle: A\n---"));
    }

    #[test]
    fn question_lines_skips_completed_items() {
        let questions = question_lines("- [ ] Open one\n- [x] Done\n* [ ] Open two\nNone");
        assert_eq!(
            questions,
            vec!["Open one".to_string(), "Open two".to_string()]
        );
    }

    #[test]
    fn question_lines_skips_nested_metadata() {
        let questions = question_lines(
            "- [ ] Should this become a claim?\n  - source: conversation `s1`\n  - prompt: What happened?\n  - filed answer: Maybe.\n- [ ] Second question\n  - evidence: raw source excerpt",
        );
        assert_eq!(
            questions,
            vec![
                "Should this become a claim?".to_string(),
                "Second question".to_string()
            ]
        );
    }

    #[test]
    fn source_compile_fingerprint_keys_parse_bucket_and_hash() {
        let keys =
            source_compile_fingerprint_keys("source_compile:uncompiled:src-a:hash1|src-b:hash2");
        assert_eq!(
            keys,
            vec![
                (
                    "uncompiled".to_string(),
                    "src-a".to_string(),
                    "hash1".to_string()
                ),
                (
                    "uncompiled".to_string(),
                    "src-b".to_string(),
                    "hash2".to_string()
                )
            ]
        );
    }

    #[test]
    fn strip_code_fence_only_unwraps_markdown() {
        assert_eq!(strip_code_fence("```markdown\n# A\n```"), "# A");
        let py = "```python\nx=1\n```";
        assert_eq!(strip_code_fence(py), py);
    }
}
