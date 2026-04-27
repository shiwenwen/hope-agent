//! Memory vector re-embedding background job.
//!
//! Spawns a [`crate::local_model_jobs`] runner that walks every memory entry,
//! re-computing its embedding under the currently active model. Designed to:
//!
//! - report progress (`bytes_completed`/`bytes_total` carry processed/total
//!   entry counts so the existing local-model-job UI shows a real progress bar)
//! - support cancellation via the standard `local_model_job_cancel` plumbing
//! - guarantee at most one running reembed at a time (a new spawn cancels any
//!   pre-existing active reembed first)
//! - persist `last_reembedded_signature` on success so the
//!   `needsReembed` indicator clears
//!
//! `KeepExisting` mode leaves rows in place and overwrites their `embedding`
//! field as it goes — searches keep working against the old vectors during the
//! rebuild. `DeleteAll` mode wipes every `memories.embedding` /
//! `embedding_signature` first, so partial failures cannot leave a mix of old
//! and new vectors. The `embedding_cache` table is left alone in either mode
//! (it is keyed by `(hash, model, signature)` and `prune_embedding_cache_to_signature`
//! has already been called by `set_memory_embedding_default`).

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::local_model_jobs::{
    self, append_log, finish_job, spawn_job, update_job_with_bytes, LocalModelJobKind,
    LocalModelJobSnapshot, LocalModelJobStatus, ProgressThrottle,
};

/// Phase strings used in `update_job_with_bytes` and looked up by
/// `src/types/local-model-jobs.ts::PHASE_KEY` for i18n display. Drift between
/// the two sides silently breaks the localized phase label, so keep these as
/// the single Rust source of truth.
pub const PHASE_REEMBED_KEEP: &str = "reembed-keep";
pub const PHASE_REEMBED_FRESH: &str = "reembed-fresh";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReembedMode {
    #[default]
    KeepExisting,
    DeleteAll,
}

impl ReembedMode {
    fn phase(self) -> &'static str {
        match self {
            Self::KeepExisting => PHASE_REEMBED_KEEP,
            Self::DeleteAll => PHASE_REEMBED_FRESH,
        }
    }

    fn step_message(self) -> &'static str {
        match self {
            Self::KeepExisting => "Re-embedding memories (keep existing)",
            Self::DeleteAll => "Re-embedding memories (fresh)",
        }
    }
}

/// Spawn (or replace) the global memory reembed job.
///
/// Invariant: at most one `MemoryReembed` job is ever in a non-terminal state.
/// Pre-existing active jobs are cancelled before the new one is spawned. The
/// old runner's per-batch `cancel.is_cancelled()` check exits at the next
/// boundary; the SQLite write connection mutex serialises any overlap.
pub fn start_memory_reembed_job(
    model_config_id: &str,
    mode: ReembedMode,
) -> Result<LocalModelJobSnapshot> {
    let store = crate::config::cached_config();
    let model = store
        .embedding_models
        .iter()
        .find(|item| item.id == model_config_id)
        .cloned()
        .ok_or_else(|| anyhow!("Embedding model config not found: {model_config_id}"))?;

    if !store.memory_embedding.enabled
        || store.memory_embedding.model_config_id.as_deref() != Some(model_config_id)
    {
        return Err(anyhow!(
            "Cannot reembed: '{model_config_id}' is not the active memory embedding model"
        ));
    }

    if let Ok(jobs) = local_model_jobs::list_jobs() {
        for job in jobs {
            if job.kind == LocalModelJobKind::MemoryReembed && !job.status.is_terminal() {
                let _ = local_model_jobs::cancel_job(&job.job_id);
            }
        }
    }

    if mode == ReembedMode::DeleteAll {
        if let Some(backend) = crate::get_memory_backend() {
            let cleared = backend.clear_all_embeddings()?;
            app_info!(
                "memory",
                "reembed_job",
                "Cleared embeddings on {} memory rows before fresh reembed",
                cleared
            );
        }
    }

    let signature = model.signature();

    spawn_job(
        LocalModelJobKind::MemoryReembed,
        model_config_id.to_string(),
        model.name.clone(),
        move |job_id, token| async move {
            let final_result = run_reembed(&job_id, mode, &signature, &token).await;
            finish_job(&job_id, final_result, &token);
        },
    )
}

async fn run_reembed(
    job_id: &str,
    mode: ReembedMode,
    signature: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<serde_json::Value> {
    let backend =
        crate::get_memory_backend().ok_or_else(|| anyhow!("Memory backend not initialized"))?;
    let phase = mode.phase();

    update_job_with_bytes(
        job_id,
        LocalModelJobStatus::Running,
        phase,
        Some(0),
        Some(0),
        None,
        None,
        None,
    );
    append_log(job_id, "step", mode.step_message());

    // Hop the blocking sqlite work to spawn_blocking so the runner future stays
    // cooperative. Wrap the per-batch progress callback in the same
    // `ProgressThrottle` the other jobs use so a fast batch API doesn't flood
    // the EventBus + jobs DB at full chunk-cadence.
    let job_id_owned = job_id.to_string();
    let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
    let cancel_clone = cancel.clone();
    let backend_clone = backend.clone();

    let count_result = tokio::task::spawn_blocking(move || {
        let mut on_progress = |done: usize, total: usize| {
            let percent = if total == 0 {
                100u8
            } else {
                ((done as u64 * 100) / total as u64).min(100) as u8
            };
            let bytes_completed = done as u64;
            // Always emit the terminal frame; otherwise let the throttle
            // coalesce mid-flight bursts.
            let terminal = total > 0 && done >= total;
            let should_emit = terminal
                || throttle
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .should_emit(phase, Some(percent), Some(bytes_completed));
            if !should_emit {
                return;
            }
            update_job_with_bytes(
                &job_id_owned,
                LocalModelJobStatus::Running,
                phase,
                Some(percent),
                Some(bytes_completed),
                Some(total as u64),
                None,
                None,
            );
        };
        backend_clone.reembed_all_with_progress(&cancel_clone, &mut on_progress, 16)
    })
    .await
    .map_err(|e| anyhow!("Reembed task join failed: {e}"))?;

    let count = count_result?;

    let signature_for_save = signature.to_string();
    crate::config::mutate_config(
        ("memory_embedding.reembedded", "memory_reembed_job"),
        move |store| {
            store.memory_embedding.last_reembedded_signature = Some(signature_for_save.clone());
            Ok(())
        },
    )?;

    app_info!(
        "memory",
        "reembed_job",
        "Memory reembed completed: count={} mode={:?}",
        count,
        mode
    );

    Ok(json!({
        "reembedded": count,
        "mode": mode,
    }))
}
