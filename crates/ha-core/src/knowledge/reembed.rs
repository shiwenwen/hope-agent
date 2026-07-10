//! Knowledge-base vector re-embedding background job (D7).
//!
//! Unlike memory (which re-embeds rows in place), a knowledge "reembed" is a
//! full reindex of every KB: parse → chunk → embed → `replace_note_index` under
//! the newly active model. [`crate::knowledge::set_knowledge_embedding_default`]
//! swaps the index DB's embedder first (recreating `note_vec` on a dimension
//! change), then spawns this job to repopulate every note's vectors. Note
//! vector search degrades to FTS-only until it finishes.
//!
//! This job is also reused for binding a new external KB (`create_kb_cmd`
//! passes `Some(vec![kb.id])`) and the per-KB "Reindex" context-menu action —
//! not just the settings-page full rebuild. All three are the same underlying
//! operation (`reindex_kb`), so they share progress reporting, cancellation,
//! and retry instead of each reimplementing it.
//!
//! Runs through the shared [`crate::local_model_jobs`] runner so it reports
//! progress, supports cancellation, and holds the at-most-one-overlapping-scope
//! invariant exactly like the memory reembed job. Progress granularity depends
//! on scope: a single-KB run (the common case — bind / per-KB reindex) reports
//! **file**-level progress (`PHASE_KNOWLEDGE_INDEX_FILES`); a multi-KB run (the
//! rare "rebuild everything" case) reports **KB**-level progress
//! (`PHASE_KNOWLEDGE_REEMBED`, `bytes_completed`/`bytes_total` carry done/total
//! KB counts) — pre-scanning every KB's file count to unify the two isn't
//! worth it for that cold path.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::local_model_jobs::{
    self, append_log, finish_job, spawn_job_with_target_kb_ids, update_job_with_bytes,
    LocalModelJobKind, LocalModelJobSnapshot, LocalModelJobStatus, ProgressThrottle,
};

/// Phase string for `update_job_with_bytes` when progress is KB-granular (the
/// multi-KB / full-rebuild path).
pub const PHASE_KNOWLEDGE_REEMBED: &str = "knowledge-reembed";

/// Phase string for `update_job_with_bytes` when progress is file-granular —
/// the common single-KB scope (bind a new space / per-KB "Reindex"). Kept as a
/// distinct phase string (not a boolean flag) so the frontend can branch its
/// progress-unit label purely off this value, the same way it already looks
/// up `PHASE_KNOWLEDGE_REEMBED` today.
pub const PHASE_KNOWLEDGE_INDEX_FILES: &str = "knowledge-index-files";

/// Cancel any non-terminal `KnowledgeReembed` job whose scope overlaps the
/// scope of the job about to be spawned (`new_scope`). `new_scope = None` (a
/// full rebuild) supersedes everything and cancels every active job, same as
/// a full rebuild always has. A scoped job (`Some(ids)`) only cancels jobs
/// that target an overlapping KB, or are themselves full-scope — concurrent
/// scans of disjoint KBs (e.g. binding two external vaults back to back) no
/// longer cancel each other.
pub fn cancel_active_knowledge_reembed_jobs(new_scope: Option<&[String]>) {
    if let Ok(jobs) = local_model_jobs::list_jobs() {
        for job in jobs {
            if job.kind != LocalModelJobKind::KnowledgeReembed || job.status.is_terminal() {
                continue;
            }
            let overlaps = match new_scope {
                None => true,
                Some(new_ids) => match job.target_kb_ids.as_deref() {
                    None => true,
                    Some(existing_ids) => new_ids.iter().any(|id| existing_ids.contains(id)),
                },
            };
            if overlaps {
                let _ = local_model_jobs::cancel_job(&job.job_id);
            }
        }
    }
}

/// Spawn (or replace) the knowledge reembed / reindex job for a scope.
///
/// `kb_ids = None` rebuilds **every** KB (the full reembed on a model switch /
/// the settings "rebuild now" button); `Some(ids)` rebuilds just those KBs
/// (binding a new external vault, or the per-KB Reindex button). The
/// signature is stamped onto `last_reembedded_signature` **only on a full
/// (None) + embedding-enabled run** — a scoped rebuild does not represent full
/// coverage and must not clear `needsReembed`. When embedding is disabled this
/// is a plain FTS reindex (no embed, no stamp) and requires an explicit
/// `kb_ids` scope.
///
/// Invariant: at most one `KnowledgeReembed` job is ever non-terminal **per
/// overlapping scope** — pre-existing active jobs whose scope overlaps this
/// one are cancelled before the new one is spawned (see
/// [`cancel_active_knowledge_reembed_jobs`]).
pub fn start_knowledge_reembed_job(
    kb_ids: Option<Vec<String>>,
    source: &str,
) -> Result<LocalModelJobSnapshot> {
    let store = crate::config::cached_config();
    let enabled = store.knowledge_embedding.enabled;

    // Resolve the job's display id/name and whether to stamp the signature.
    let (model_id, display_name, stamp_signature) = if enabled {
        let id = store
            .knowledge_embedding
            .model_config_id
            .clone()
            .ok_or_else(|| anyhow!("No knowledge embedding model is currently active"))?;
        let model = store
            .embedding_models
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("Embedding model config not found: {id}"))?;
        // Only a full rebuild covers every note → only it may clear needsReembed.
        let stamp = if kb_ids.is_none() {
            Some(model.signature())
        } else {
            None
        };
        (id, model.name.clone(), stamp)
    } else {
        // Embedding off: plain index (FTS) rebuild. A full disabled rebuild has
        // no caller and no signature meaning — require an explicit KB scope.
        if kb_ids.is_none() {
            return Err(anyhow!(
                "Knowledge embedding is not enabled; nothing to reembed"
            ));
        }
        (
            "knowledge-reindex".to_string(),
            "Knowledge reindex".to_string(),
            None,
        )
    };

    cancel_active_knowledge_reembed_jobs(kb_ids.as_deref());

    crate::app_info!(
        "knowledge",
        "reembed",
        "Knowledge reembed/reindex job requested: scope={} enabled={} source={}",
        kb_ids
            .as_ref()
            .map(|v| v.len().to_string())
            .unwrap_or_else(|| "all".to_string()),
        enabled,
        source
    );

    spawn_job_with_target_kb_ids(
        LocalModelJobKind::KnowledgeReembed,
        model_id,
        display_name,
        kb_ids.clone(),
        move |job_id, token| async move {
            let result = run_knowledge_reembed(&job_id, kb_ids, stamp_signature, &token).await;
            finish_job(&job_id, result, &token);
        },
    )
}

/// Run the reembed: full-reindex every KB in scope under the active model.
///
/// v1 simplification (perf follow-up, correctness-first for now): this
/// re-reads + re-parses + re-chunks every note via `reindex_kb(full=true)`
/// purely to regenerate vectors, and processes KBs sequentially. A future
/// optimization could (a) iterate existing `note_chunk` rows and re-embed bodies
/// in place — skipping parse/chunk/FTS churn, like memory's
/// `reembed_all_with_progress` — and (b) pipeline embed batches across KBs.
async fn run_knowledge_reembed(
    job_id: &str,
    kb_ids: Option<Vec<String>>,
    stamp_signature: Option<String>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<serde_json::Value> {
    let kb_ids = match kb_ids {
        Some(ids) => ids,
        None => {
            let registry =
                crate::get_knowledge_db().ok_or_else(|| anyhow!("knowledge db not initialized"))?;
            // Propagate a registry error instead of unwrap_or_default → []: an
            // empty list from a failed query must NOT look like "0 KBs, all done"
            // and stamp the signature (see the cancel/had_error guard below).
            registry.list_all_ids()?
        }
    };
    let total = kb_ids.len();

    append_log(job_id, "step", "Re-embedding knowledge notes");

    let job_id_owned = job_id.to_string();
    let cancel_clone = cancel.clone();
    let kb_ids_for_task = kb_ids.clone();

    let (reindexed, failed_files, had_error) = if total == 1 {
        // Single-KB scope (bind a new space / per-KB "Reindex"): report
        // file-level progress. `reindex_kb` already knows the total file
        // count upfront (the initial disk scan), so unlike the multi-KB path
        // below there's no "stuck at 0/1 for minutes" window while a slow
        // (remote) embedding call runs per file.
        update_job_with_bytes(
            job_id,
            LocalModelJobStatus::Running,
            PHASE_KNOWLEDGE_INDEX_FILES,
            Some(0),
            Some(0),
            None,
            None,
            None,
        );
        let kb_id = kb_ids_for_task[0].clone();
        let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
        tokio::task::spawn_blocking(move || -> (usize, usize, bool) {
            let mut on_progress = |done: usize, total_files: usize| -> bool {
                let percent = if total_files == 0 {
                    100u8
                } else {
                    ((done as u64 * 100) / total_files as u64).min(100) as u8
                };
                let terminal = done >= total_files;
                let should_emit = terminal
                    || throttle
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .should_emit(
                            PHASE_KNOWLEDGE_INDEX_FILES,
                            Some(percent),
                            Some(done as u64),
                        );
                if should_emit {
                    update_job_with_bytes(
                        &job_id_owned,
                        LocalModelJobStatus::Running,
                        PHASE_KNOWLEDGE_INDEX_FILES,
                        Some(percent),
                        Some(done as u64),
                        Some(total_files as u64),
                        None,
                        None,
                    );
                }
                !cancel_clone.is_cancelled()
            };
            match crate::knowledge::index::reindex_kb_with_progress(&kb_id, true, &mut on_progress)
            {
                Ok(report) => (report.changed, report.failed, false),
                Err(e) => {
                    crate::app_warn!("knowledge", "reembed", "reindex kb {} failed: {}", kb_id, e);
                    (0, 0, true)
                }
            }
        })
        .await
        .map_err(|e| anyhow!("Knowledge reembed task join failed: {e}"))?
    } else {
        // Multi-KB scope (rare: settings "rebuild everything"): keep KB-
        // granular progress — a unified cross-KB file total would need a
        // second full directory walk of every KB up front, not worth it for
        // this cold path.
        update_job_with_bytes(
            job_id,
            LocalModelJobStatus::Running,
            PHASE_KNOWLEDGE_REEMBED,
            Some(0),
            Some(0),
            Some(total as u64),
            None,
            None,
        );
        let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
        tokio::task::spawn_blocking(move || -> (usize, usize, bool) {
            let mut done = 0usize;
            let mut changed_total = 0usize;
            let mut failed_total = 0usize;
            let mut had_error = false;
            for kb_id in &kb_ids_for_task {
                if cancel_clone.is_cancelled() {
                    break;
                }
                match crate::knowledge::index::reindex_kb(kb_id, true) {
                    Ok(report) => {
                        changed_total += report.changed;
                        failed_total += report.failed;
                    }
                    Err(e) => {
                        had_error = true;
                        crate::app_warn!(
                            "knowledge",
                            "reembed",
                            "reindex kb {} failed: {}",
                            kb_id,
                            e
                        )
                    }
                }
                done += 1;
                let percent = if total == 0 {
                    100u8
                } else {
                    ((done as u64 * 100) / total as u64).min(100) as u8
                };
                let terminal = done >= total;
                let should_emit = terminal
                    || throttle
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .should_emit(PHASE_KNOWLEDGE_REEMBED, Some(percent), Some(done as u64));
                if should_emit {
                    update_job_with_bytes(
                        &job_id_owned,
                        LocalModelJobStatus::Running,
                        PHASE_KNOWLEDGE_REEMBED,
                        Some(percent),
                        Some(done as u64),
                        Some(total as u64),
                        None,
                        None,
                    );
                }
            }
            (changed_total, failed_total, had_error)
        })
        .await
        .map_err(|e| anyhow!("Knowledge reembed task join failed: {e}"))?
    };

    // Only stamp last_reembedded_signature when the run actually covered every
    // note under this model. On cancellation or any per-KB failure return Err
    // (job → Failed) and leave the signature untouched, so needs_reembed stays
    // true and the UI keeps prompting a rebuild. This mirrors memory's reembed,
    // which propagates Err on cancel *before* its signature write — without this
    // guard a cancelled/partial run would falsely report "up to date" while
    // notes have no vectors.
    if cancel.is_cancelled() {
        return Err(anyhow!("Knowledge reembed cancelled"));
    }
    if had_error {
        return Err(anyhow!(
            "Knowledge reembed finished with errors; some KBs were not re-embedded"
        ));
    }

    // Stamp only on a full (None scope) + enabled run — `stamp_signature` is
    // None for scoped rebuilds and disabled reindexes, which must not clear
    // `needsReembed` (they don't cover every note).
    if let Some(signature_for_save) = stamp_signature {
        crate::config::mutate_config(
            ("knowledge_embedding.reembedded", "knowledge_reembed_job"),
            move |store| {
                store.knowledge_embedding.last_reembedded_signature =
                    Some(signature_for_save.clone());
                Ok(())
            },
        )?;
    }

    // Refresh every KB in scope so the note list / counts update without a
    // manual reload. `spawn_reindex_kb`'s fire-and-forget path already does
    // this on completion; this job-tracked path historically didn't, which
    // left a completed bind/rebuild silently stuck showing stale contents.
    if let Some(bus) = crate::get_event_bus() {
        for kb_id in &kb_ids {
            bus.emit(
                "knowledge:changed",
                json!({ "kbId": kb_id, "op": "reindex" }),
            );
        }
    }

    crate::app_info!(
        "knowledge",
        "reembed",
        "Knowledge reembed completed: {} notes reindexed ({} failed) across {} KBs",
        reindexed,
        failed_files,
        total
    );

    Ok(json!({ "reindexed": reindexed, "failedFiles": failed_files, "kbCount": total }))
}
