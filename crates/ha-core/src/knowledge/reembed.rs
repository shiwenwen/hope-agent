//! Knowledge-base vector re-embedding background job (D7).
//!
//! Unlike memory (which re-embeds rows in place), a knowledge "reembed" is a
//! full reindex of every KB: parse → chunk → embed → `replace_note_index` under
//! the newly active model. [`crate::knowledge::set_knowledge_embedding_default`]
//! swaps the index DB's embedder first (recreating `note_vec` on a dimension
//! change), then spawns this job to repopulate every note's vectors. Note
//! vector search degrades to FTS-only until it finishes.
//!
//! Runs through the shared [`crate::local_model_jobs`] runner so it reports
//! progress, supports cancellation, and holds the at-most-one-running invariant
//! exactly like the memory reembed job. Progress is KB-granular
//! (`bytes_completed`/`bytes_total` carry done/total KB counts).

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::local_model_jobs::{
    self, append_log, finish_job, spawn_job, update_job_with_bytes, LocalModelJobKind,
    LocalModelJobSnapshot, LocalModelJobStatus, ProgressThrottle,
};

/// Phase string for `update_job_with_bytes` (looked up by the frontend for the
/// localized phase label — keep as the single Rust source of truth).
pub const PHASE_KNOWLEDGE_REEMBED: &str = "knowledge-reembed";

/// Cancel any non-terminal `KnowledgeReembed` jobs (at most one runs globally).
pub fn cancel_active_knowledge_reembed_jobs() {
    if let Ok(jobs) = local_model_jobs::list_jobs() {
        for job in jobs {
            if job.kind == LocalModelJobKind::KnowledgeReembed && !job.status.is_terminal() {
                let _ = local_model_jobs::cancel_job(&job.job_id);
            }
        }
    }
}

/// Spawn (or replace) the global knowledge reembed / reindex job.
///
/// `kb_ids = None` rebuilds **every** KB (the full reembed on a model switch /
/// the settings "rebuild now" button); `Some(ids)` rebuilds just those KBs (the
/// per-KB Reindex button). The signature is stamped onto
/// `last_reembedded_signature` **only on a full (None) + embedding-enabled run**
/// — a single-KB rebuild does not represent full coverage and must not clear
/// `needsReembed`. When embedding is disabled this is a plain FTS reindex (no
/// embed, no stamp) and requires an explicit `kb_ids` scope.
///
/// Invariant: at most one `KnowledgeReembed` job is ever non-terminal —
/// pre-existing active jobs are cancelled before the new one is spawned.
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

    cancel_active_knowledge_reembed_jobs();

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

    spawn_job(
        LocalModelJobKind::KnowledgeReembed,
        model_id,
        display_name,
        move |job_id, token| async move {
            let result = run_knowledge_reembed(&job_id, kb_ids, stamp_signature, &token).await;
            finish_job(&job_id, result, &token);
        },
    )
}

/// Run the reembed: full-reindex every KB under the active model.
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
    append_log(job_id, "step", "Re-embedding knowledge notes");

    let job_id_owned = job_id.to_string();
    let cancel_clone = cancel.clone();
    let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));

    // Hop the blocking reindex (file IO + embedding calls) to spawn_blocking so
    // the runner future stays cooperative.
    let (reindexed, had_error) = tokio::task::spawn_blocking(move || -> (usize, bool) {
        let mut done = 0usize;
        let mut changed_total = 0usize;
        let mut had_error = false;
        for kb_id in &kb_ids {
            if cancel_clone.is_cancelled() {
                break;
            }
            match crate::knowledge::index::reindex_kb(kb_id, true) {
                Ok(report) => changed_total += report.changed,
                Err(e) => {
                    had_error = true;
                    crate::app_warn!("knowledge", "reembed", "reindex kb {} failed: {}", kb_id, e)
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
        (changed_total, had_error)
    })
    .await
    .map_err(|e| anyhow!("Knowledge reembed task join failed: {e}"))?;

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
    // None for single-KB rebuilds and disabled reindexes, which must not clear
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

    crate::app_info!(
        "knowledge",
        "reembed",
        "Knowledge reembed completed: {} notes reindexed across {} KBs",
        reindexed,
        total
    );

    Ok(json!({ "reindexed": reindexed, "kbCount": total }))
}
