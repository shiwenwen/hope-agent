//! Apply an approved maintenance proposal through the **owner plane** (`service`).
//!
//! The owner explicitly approved the change, so this bypasses the agent-plane
//! `effective_kb_access` gate — exactly like a GUI edit. Every write re-reads the
//! note first and passes the current disk hash as the stale-write guard, so a
//! proposal generated against an out-of-date snapshot fails cleanly instead of
//! clobbering newer content. Proposals are only ever generated for internal KBs
//! (the scheduler skips every external root), so apply never touches a bound
//! vault even when it opted into external writes (WS7).

use anyhow::{bail, Result};

use super::types::{MaintenanceProposal, ProposalAction};
use crate::knowledge::types::{CompileRunStatus, CompileStartInput};
use crate::knowledge::{parser, service};

pub async fn apply_proposal(p: &MaintenanceProposal) -> Result<()> {
    let kb = p.kb_id.as_str();
    // Defense in depth (WS7): maintenance must never write an external (bound)
    // vault, even one opted into external writes. The scheduler already skips
    // external roots at generation time, but enforce it here too so the owner-
    // approve path (which bypasses `effective_kb_access`) can't reach a vault via
    // a stale proposal. Fail closed.
    if super::super::resolve_kb_dir(kb)
        .map(|r| r.is_external)
        .unwrap_or(true)
    {
        bail!("maintenance does not write external knowledge bases (kb '{kb}')");
    }
    match &p.action {
        // Non-destructive: apply to the note's *current* content (append a link to
        // whatever it now holds). `note_read` is the existence guard — a deleted
        // target errors out → proposal marked Failed.
        ProposalAction::AppendLink { from_path, to_ref } => {
            let cur = service::note_read(kb, from_path)?;
            let link = format!("[[{to_ref}]]");
            if cur.content.contains(&link) {
                return Ok(()); // idempotent: link already present
            }
            let mut content = cur.content.clone();
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&format!("\n{link}\n"));
            // Guard against a change between this read and the write.
            service::note_save(kb, from_path, &content, Some(&cur.content_hash), false)?;
            Ok(())
        }
        // Non-destructive: merge into the note's current frontmatter.
        ProposalAction::SetFrontmatter { path, props } => {
            let cur = service::note_read(kb, path)?;
            let updated = parser::merge_frontmatter(&cur.content, props);
            if updated == cur.content {
                return Ok(()); // nothing changed
            }
            service::note_save(kb, path, &updated, Some(&cur.content_hash), false)?;
            Ok(())
        }
        ProposalAction::CreateNote {
            path,
            content,
            overwrite,
        } => {
            // create_only = !overwrite: a non-overwrite create fails if the path
            // now exists (someone created it meanwhile) — surfaced to the user.
            service::note_save(kb, path, content, None, !*overwrite)?;
            Ok(())
        }
        ProposalAction::PatchNote {
            path,
            expected_hash,
            content,
        } => {
            let cur = service::note_read(kb, path)?;
            if cur.content_hash != *expected_hash {
                bail!("stale patch: '{path}' changed since the proposal was made");
            }
            if cur.content == *content {
                return Ok(());
            }
            service::note_save(kb, path, content, Some(expected_hash), false)?;
            Ok(())
        }
        ProposalAction::CompileSources {
            source_ids,
            reason: _,
        } => {
            if source_ids.is_empty() {
                bail!("compile proposal has no sources");
            }
            let run = service::compile_start(
                kb,
                CompileStartInput {
                    source_ids: source_ids.clone(),
                    strategy: None,
                },
            )
            .await?;
            if matches!(
                run.status,
                CompileRunStatus::Failed | CompileRunStatus::Cancelled
            ) {
                bail!(
                    "compile run {} ended with status {}{}",
                    run.id,
                    run.status.as_str(),
                    run.error
                        .as_deref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                );
            }
            Ok(())
        }
        // Destructive: validate EVERY note's generation-time hash before any write,
        // so a note edited since the proposal was made aborts the whole merge
        // instead of clobbering newer content / deleting a now-different note.
        ProposalAction::MergeNotes {
            keep_path,
            keep_expected_hash,
            keep_content,
            removes,
        } => {
            let keep_cur = service::note_read(kb, keep_path)?;
            if keep_cur.content_hash != *keep_expected_hash {
                bail!("stale merge: '{keep_path}' changed since the proposal was made");
            }
            for r in removes {
                if r.path.as_str() == keep_path.as_str() {
                    continue;
                }
                match service::note_read(kb, &r.path) {
                    Ok(cur) => {
                        if cur.content_hash != r.expected_hash {
                            bail!(
                                "stale merge: '{}' changed since the proposal was made",
                                r.path
                            );
                        }
                    }
                    // Already gone (deleted externally) — fine, nothing to delete.
                    Err(_) => continue,
                }
            }
            // All validated — now mutate: write the merged keep, then delete drops.
            service::note_save(kb, keep_path, keep_content, Some(keep_expected_hash), false)?;
            for r in removes {
                if r.path.as_str() == keep_path.as_str() {
                    continue;
                }
                if service::note_read(kb, &r.path).is_ok() {
                    service::note_delete(kb, &r.path)?;
                }
            }
            Ok(())
        }
    }
}
