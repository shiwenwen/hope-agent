use crate::commands::CmdError;
use crate::get_memory_backend;
use crate::memory;

#[tauri::command]
pub async fn memory_add(entry: memory::NewMemory) -> Result<i64, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let id = ha_core::blocking::run_blocking(move || backend.add(entry)).await?;
    memory::emit_memory_changed("add", Some(id), None);
    Ok(id)
}

#[tauri::command]
pub async fn memory_update(id: i64, content: String, tags: Vec<String>) -> Result<(), CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.update(id, &content, &tags)).await?;
    memory::emit_memory_changed("update", Some(id), None);
    Ok(())
}

#[tauri::command]
pub async fn memory_toggle_pin(id: i64, pinned: bool) -> Result<(), CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.toggle_pin(id, pinned)).await?;
    memory::emit_memory_changed(if pinned { "pin" } else { "unpin" }, Some(id), None);
    Ok(())
}

#[tauri::command]
pub async fn memory_delete(id: i64) -> Result<(), CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.delete(id)).await?;
    memory::emit_memory_changed("delete", Some(id), None);
    Ok(())
}

#[tauri::command]
pub async fn memory_get(id: i64) -> Result<Option<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let entry = ha_core::blocking::run_blocking(move || backend.get(id)).await?;
    Ok(entry)
}

/// Read-only schema metadata for the owner UI filters. Maps to
/// `GET /api/claims/schema`.
#[tauri::command]
pub async fn claim_schema_metadata() -> Result<memory::claims::ClaimSchemaMetadata, CmdError> {
    Ok(memory::claims::claim_schema_metadata())
}

/// List structured claims (next-gen Dreaming, read-only). Optional scope
/// (`scope_type` + `scope_id` primitives — never a structured object, so the
/// HTTP query transport can't silently degrade the filter) / status /
/// claim_type; newest-updated first. Maps to `GET /api/claims`.
#[tauri::command]
pub async fn claim_list(
    scope_type: Option<String>,
    scope_id: Option<String>,
    status: Option<String>,
    claim_type: Option<String>,
    confidence_source: Option<String>,
    evidence_class: Option<String>,
    evidence_source_type: Option<String>,
    query: Option<String>,
    sort: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<memory::claims::ClaimRecord>, CmdError> {
    let scope = memory::claims::parse_claim_scope(scope_type.as_deref(), scope_id.as_deref())?;
    let claims = ha_core::blocking::run_blocking(move || {
        memory::claims::list_claims(memory::claims::ClaimListFilter {
            scope,
            status,
            claim_type,
            confidence_source,
            evidence_class,
            evidence_source_type,
            query,
            sort,
            limit,
            offset,
        })
    })
    .await?;
    Ok(claims)
}

/// Page structured claims with an exact count for the same filters. Maps to
/// `GET /api/claims/page`; the older `claim_list` array command stays for
/// compatibility.
#[tauri::command]
pub async fn claim_list_page(
    scope_type: Option<String>,
    scope_id: Option<String>,
    status: Option<String>,
    claim_type: Option<String>,
    confidence_source: Option<String>,
    evidence_class: Option<String>,
    evidence_source_type: Option<String>,
    query: Option<String>,
    sort: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<memory::claims::ClaimListPage, CmdError> {
    let scope = memory::claims::parse_claim_scope(scope_type.as_deref(), scope_id.as_deref())?;
    ha_core::blocking::run_blocking(move || {
        memory::claims::list_claims_page(memory::claims::ClaimListFilter {
            scope,
            status,
            claim_type,
            confidence_source,
            evidence_class,
            evidence_source_type,
            query,
            sort,
            limit,
            offset,
        })
    })
    .await
    .map_err(Into::into)
}

/// Fetch a single claim plus its evidence + legacy-memory links. Returns
/// `null` if the id is unknown. Maps to `GET /api/claims/{id}`.
#[tauri::command]
pub async fn claim_get(id: String) -> Result<Option<memory::claims::ClaimDetail>, CmdError> {
    let detail = ha_core::blocking::run_blocking(move || memory::claims::get_claim(&id)).await?;
    Ok(detail)
}

/// Read-only entity context graph around one claim. Owner-plane helper; not
/// exposed to agent tools.
#[tauri::command]
pub async fn claim_graph(
    id: String,
    limit: Option<usize>,
) -> Result<memory::claims::ClaimGraphProjection, CmdError> {
    ha_core::blocking::run_blocking(move || memory::claims::claim_graph(&id, limit))
        .await
        .map_err(Into::into)
}

/// List conflicting claims for one claim: same scope/type/subject/predicate,
/// different object, and effective-active or needs_review. Owner-plane helper
/// for Review Inbox; not exposed to agent tools.
#[tauri::command]
pub async fn claim_conflicts(
    id: String,
    limit: Option<usize>,
) -> Result<Vec<memory::claims::ClaimRecord>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::claims::list_claim_conflicts(&id, limit))
        .await
        .map_err(Into::into)
}

/// Bounded conflict details for Review Inbox evidence matrix. Owner-plane
/// helper; not exposed to agent tools.
#[tauri::command]
pub async fn claim_conflict_details(
    id: String,
    limit: Option<usize>,
) -> Result<Vec<memory::claims::ClaimDetail>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::claims::list_claim_conflict_details(&id, limit))
        .await
        .map_err(Into::into)
}

/// Batch conflict counts for Review Inbox list grouping. Owner-plane helper;
/// not exposed to agent tools.
#[tauri::command]
pub async fn claim_conflict_summaries(
    ids: Vec<String>,
) -> Result<Vec<memory::claims::ClaimConflictSummary>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::claims::list_claim_conflict_summaries(&ids))
        .await
        .map_err(Into::into)
}

/// Batch evidence trust counts for claim list rows. Owner-plane helper; not
/// exposed to agent tools.
#[tauri::command]
pub async fn claim_evidence_summaries(
    ids: Vec<String>,
) -> Result<Vec<memory::claims::ClaimEvidenceSummary>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::claims::list_claim_evidence_summaries(&ids))
        .await
        .map_err(Into::into)
}

/// Batch Review Inbox risk summaries for claim list rows. Owner-plane helper;
/// not exposed to agent tools.
#[tauri::command]
pub async fn claim_review_summaries(
    ids: Vec<String>,
) -> Result<Vec<memory::claims::ClaimReviewSummary>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::claims::list_claim_review_summaries(&ids))
        .await
        .map_err(Into::into)
}

/// User correction (Lucid Review, design §5.2 §5.3): partial-update one claim —
/// edit content/triple/tags, change status (approve / reject / mark-outdated),
/// move scope, or pin/unpin. Writes evidence + a decision-log entry and emits
/// `memory:claim_changed`. Flat params (not a wrapped struct) so the call shape
/// matches the HTTP `PATCH /api/claims/{id}` body verbatim — `id` interpolates
/// into the path there, the rest becomes the JSON body.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn claim_update(
    id: String,
    content: Option<String>,
    subject: Option<String>,
    predicate: Option<String>,
    object: Option<String>,
    tags: Option<Vec<String>>,
    status: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    pinned: Option<bool>,
    note: Option<String>,
) -> Result<memory::claims::ClaimActionOutcome, CmdError> {
    let outcome = ha_core::blocking::run_blocking(move || {
        memory::claims::update_claim(memory::claims::ClaimUpdate {
            claim_id: id,
            content,
            subject,
            predicate,
            object,
            tags,
            status,
            scope_type,
            scope_id,
            pinned,
            note,
        })
    })
    .await?;
    Ok(outcome)
}

/// Forget a claim (design §5.3): `permanent=false` archives it (kept as an audit
/// trail, linked legacy memories stop injecting); `true` hard-deletes the claim
/// graph + any legacy memory it solely managed. Maps to
/// `POST /api/claims/{id}/forget`.
#[tauri::command]
pub async fn claim_forget(
    id: String,
    permanent: Option<bool>,
    note: Option<String>,
) -> Result<memory::claims::ClaimActionOutcome, CmdError> {
    let outcome = ha_core::blocking::run_blocking(move || {
        memory::claims::forget_claim(&id, permanent.unwrap_or(false), note.as_deref())
    })
    .await?;
    Ok(outcome)
}

/// Dry-run: scan legacy memories and return a backfill plan (exact summary +
/// capped candidate preview), writing nothing. Maps to
/// `GET /api/memory/backfill/plan`. Full-table scan runs on a blocking thread.
#[tauri::command]
pub async fn memory_backfill_plan() -> Result<memory::claims::BackfillPlan, CmdError> {
    tokio::task::spawn_blocking(memory::claims::plan_backfill)
        .await
        .map_err(|e| CmdError::msg(format!("backfill plan task failed: {e}")))?
        .map_err(Into::into)
}

/// Apply the backfill deterministically (re-scan, NOT trusting any client-sent
/// candidate list): write a claim + memory evidence + detached link for each
/// not-yet-linked memory. Maps to `POST /api/memory/backfill/apply`.
#[tauri::command]
pub async fn memory_backfill_apply() -> Result<memory::claims::BackfillApplyResult, CmdError> {
    tokio::task::spawn_blocking(memory::claims::apply_backfill)
        .await
        .map_err(|e| CmdError::msg(format!("backfill apply task failed: {e}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_list(
    scope: Option<memory::MemoryScope>,
    types: Option<Vec<memory::MemoryType>>,
    sources: Option<Vec<String>>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let entries = ha_core::blocking::run_blocking(move || {
        backend.list_filtered(
            scope.as_ref(),
            types.as_deref(),
            sources.as_deref(),
            limit.unwrap_or(50),
            offset.unwrap_or(0),
        )
    })
    .await?;
    Ok(entries)
}

#[tauri::command]
pub async fn memory_history(
    limit: Option<usize>,
    offset: Option<usize>,
    query: Option<String>,
    actions: Option<Vec<memory::MemoryHistoryAction>>,
    memory_types: Option<Vec<memory::MemoryType>>,
    sources: Option<Vec<String>>,
) -> Result<Vec<memory::MemoryHistoryRecord>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let query = memory::MemoryHistoryQuery {
        query,
        actions,
        memory_types,
        sources,
        limit,
        offset,
    };
    ha_core::blocking::run_blocking(move || backend.history_filtered(&query))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_history_page(
    limit: Option<usize>,
    offset: Option<usize>,
    query: Option<String>,
    actions: Option<Vec<memory::MemoryHistoryAction>>,
    memory_types: Option<Vec<memory::MemoryType>>,
    sources: Option<Vec<String>>,
) -> Result<memory::MemoryHistoryListResponse, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let query = memory::MemoryHistoryQuery {
        query,
        actions,
        memory_types,
        sources,
        limit,
        offset,
    };
    ha_core::blocking::run_blocking(move || backend.history_filtered_page(&query))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_audit_page(
    limit: Option<usize>,
    offset: Option<usize>,
    query: Option<String>,
    action: Option<String>,
) -> Result<memory::MemoryAuditPageResponse, CmdError> {
    let query = memory::MemoryAuditPageQuery {
        query,
        action,
        limit,
        offset,
    };
    ha_core::blocking::run_blocking(move || memory::memory_audit_page(query))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_search(
    query: memory::MemorySearchQuery,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let results = ha_core::blocking::run_blocking(move || backend.search(&query)).await?;
    Ok(results)
}

#[tauri::command]
pub async fn memory_count(
    scope: Option<memory::MemoryScope>,
    sources: Option<Vec<String>>,
) -> Result<usize, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let count = ha_core::blocking::run_blocking(move || {
        backend.count_filtered(scope.as_ref(), sources.as_deref())
    })
    .await?;
    Ok(count)
}

#[tauri::command]
pub async fn memory_episode_add(
    episode: memory::NewMemoryEpisode,
) -> Result<memory::MemoryEpisodeRecord, CmdError> {
    let record = ha_core::blocking::run_blocking(move || memory::add_episode(episode)).await?;
    memory::emit_memory_changed("episode_add", None, Some(1));
    Ok(record)
}

#[tauri::command]
pub async fn memory_episode_list_page(
    query: memory::MemoryEpisodeQuery,
) -> Result<memory::MemoryEpisodeListPage, CmdError> {
    ha_core::blocking::run_blocking(move || memory::list_episodes_page(query))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_episode_get(
    id: String,
) -> Result<Option<memory::MemoryEpisodeRecord>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::get_episode(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_episode_update(
    id: String,
    patch: memory::MemoryEpisodePatch,
) -> Result<Option<memory::MemoryEpisodeRecord>, CmdError> {
    let record =
        ha_core::blocking::run_blocking(move || memory::update_episode(&id, patch)).await?;
    if record.is_some() {
        memory::emit_memory_changed("episode_update", None, Some(1));
    }
    Ok(record)
}

#[tauri::command]
pub async fn memory_episode_archive(id: String) -> Result<bool, CmdError> {
    let changed = ha_core::blocking::run_blocking(move || memory::archive_episode(&id)).await?;
    if changed {
        memory::emit_memory_changed("episode_archive", None, Some(1));
    }
    Ok(changed)
}

#[tauri::command]
pub async fn memory_episode_restore(id: String) -> Result<bool, CmdError> {
    let changed = ha_core::blocking::run_blocking(move || memory::restore_episode(&id)).await?;
    if changed {
        memory::emit_memory_changed("episode_restore", None, Some(1));
    }
    Ok(changed)
}

#[tauri::command]
pub async fn memory_procedure_add(
    procedure: memory::NewMemoryProcedure,
) -> Result<memory::MemoryProcedureRecord, CmdError> {
    let record = ha_core::blocking::run_blocking(move || memory::add_procedure(procedure)).await?;
    memory::emit_memory_changed("procedure_add", None, Some(1));
    Ok(record)
}

#[tauri::command]
pub async fn memory_episode_promote_procedure(
    id: String,
    options: Option<memory::PromoteEpisodeOptions>,
) -> Result<memory::MemoryProcedureRecord, CmdError> {
    let record = ha_core::blocking::run_blocking(move || {
        memory::promote_episode_to_procedure(&id, options.unwrap_or_default())
    })
    .await?;
    memory::emit_memory_changed("procedure_add", None, Some(1));
    Ok(record)
}

#[tauri::command]
pub async fn memory_procedure_list_page(
    query: memory::MemoryProcedureQuery,
) -> Result<memory::MemoryProcedureListPage, CmdError> {
    ha_core::blocking::run_blocking(move || memory::list_procedures_page(query))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_procedure_get(
    id: String,
) -> Result<Option<memory::MemoryProcedureRecord>, CmdError> {
    ha_core::blocking::run_blocking(move || memory::get_procedure(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_procedure_update(
    id: String,
    patch: memory::MemoryProcedurePatch,
) -> Result<Option<memory::MemoryProcedureRecord>, CmdError> {
    let record =
        ha_core::blocking::run_blocking(move || memory::update_procedure(&id, patch)).await?;
    if record.is_some() {
        memory::emit_memory_changed("procedure_update", None, Some(1));
    }
    Ok(record)
}

#[tauri::command]
pub async fn memory_procedure_archive(id: String) -> Result<bool, CmdError> {
    let changed = ha_core::blocking::run_blocking(move || memory::archive_procedure(&id)).await?;
    if changed {
        memory::emit_memory_changed("procedure_archive", None, Some(1));
    }
    Ok(changed)
}

#[tauri::command]
pub async fn memory_procedure_restore(id: String) -> Result<bool, CmdError> {
    let changed = ha_core::blocking::run_blocking(move || memory::restore_procedure(&id)).await?;
    if changed {
        memory::emit_memory_changed("procedure_restore", None, Some(1));
    }
    Ok(changed)
}

#[tauri::command]
pub async fn memory_experience_history_page(
    query: memory::MemoryExperienceHistoryQuery,
) -> Result<memory::MemoryExperienceHistoryListPage, CmdError> {
    ha_core::blocking::run_blocking(move || memory::list_experience_history_page(query))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_export(scope: Option<memory::MemoryScope>) -> Result<String, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let md =
        ha_core::blocking::run_blocking(move || backend.export_markdown(scope.as_ref())).await?;
    Ok(md)
}

#[tauri::command]
pub async fn memory_backup_export() -> Result<memory::MemoryBackupBundle, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || memory::export_backup_bundle(backend.as_ref()))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_backup_export_encrypted(
    passphrase: String,
) -> Result<memory::MemoryEncryptedBackupBundle, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || {
        memory::export_encrypted_backup_bundle(backend.as_ref(), &passphrase)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_backup_export_archive(output_path: String) -> Result<String, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || -> anyhow::Result<String> {
        let archive = memory::export_backup_archive(backend.as_ref())?;
        ha_core::platform::write_atomic(std::path::Path::new(&output_path), &archive)?;
        Ok(output_path)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_backup_preview(
    content: String,
    passphrase: Option<String>,
) -> Result<memory::MemoryBackupImportPreview, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || {
        memory::preview_backup_bundle_with_passphrase(
            backend.as_ref(),
            &content,
            passphrase.as_deref(),
        )
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_backup_preview_archive(
    data: Vec<u8>,
) -> Result<memory::MemoryBackupImportPreview, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || memory::preview_backup_archive(backend.as_ref(), &data))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_backup_restore_legacy(
    content: String,
    options: Option<memory::MemoryBackupRestoreOptions>,
    passphrase: Option<String>,
) -> Result<memory::MemoryBackupRestoreResult, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let result = ha_core::blocking::run_blocking(move || {
        memory::restore_backup_legacy_memories_with_passphrase(
            backend.as_ref(),
            &content,
            options.unwrap_or_default(),
            passphrase.as_deref(),
        )
    })
    .await?;
    if result.import_result.created > 0 {
        memory::emit_memory_changed(
            "backup_restore_legacy",
            None,
            Some(result.import_result.created),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn memory_backup_restore_legacy_archive(
    data: Vec<u8>,
    options: Option<memory::MemoryBackupRestoreOptions>,
) -> Result<memory::MemoryBackupRestoreResult, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let result = ha_core::blocking::run_blocking(move || {
        memory::restore_backup_legacy_memories_from_archive(
            backend.as_ref(),
            &data,
            options.unwrap_or_default(),
        )
    })
    .await?;
    if result.import_result.created > 0 {
        memory::emit_memory_changed(
            "backup_restore_legacy",
            None,
            Some(result.import_result.created),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn memory_backup_restore_structured(
    content: String,
    options: Option<memory::MemoryBackupStructuredRestoreOptions>,
    passphrase: Option<String>,
) -> Result<memory::MemoryBackupStructuredRestoreResult, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let result = ha_core::blocking::run_blocking(move || {
        memory::restore_backup_structured_memory_with_passphrase(
            backend.as_ref(),
            &content,
            options.unwrap_or_default(),
            passphrase.as_deref(),
        )
    })
    .await?;
    if result.restored_claims > 0 {
        memory::emit_claim_changed(
            "backup_restore_structured",
            None,
            Some(result.restored_claims),
        );
    }
    if result.restored_profile_snapshots > 0 {
        memory::emit_memory_changed(
            "backup_restore_profile",
            None,
            Some(result.restored_profile_snapshots),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn memory_backup_restore_structured_archive(
    data: Vec<u8>,
    options: Option<memory::MemoryBackupStructuredRestoreOptions>,
) -> Result<memory::MemoryBackupStructuredRestoreResult, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let result = ha_core::blocking::run_blocking(move || {
        memory::restore_backup_structured_memory_from_archive(
            backend.as_ref(),
            &data,
            options.unwrap_or_default(),
        )
    })
    .await?;
    if result.restored_claims > 0 {
        memory::emit_claim_changed(
            "backup_restore_structured",
            None,
            Some(result.restored_claims),
        );
    }
    if result.restored_profile_snapshots > 0 {
        memory::emit_memory_changed(
            "backup_restore_profile",
            None,
            Some(result.restored_profile_snapshots),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn memory_find_similar(
    content: String,
    threshold: Option<f32>,
    limit: Option<usize>,
) -> Result<Vec<memory::MemoryEntry>, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let dedup_cfg = memory::load_dedup_config();
    let threshold = threshold.unwrap_or(dedup_cfg.threshold_merge);
    let limit = limit.unwrap_or(5);
    let results = ha_core::blocking::run_blocking(move || {
        backend.find_similar(&content, None, None, threshold, limit)
    })
    .await?;
    Ok(results)
}

#[tauri::command]
pub async fn memory_delete_batch(ids: Vec<i64>) -> Result<usize, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let deleted = ha_core::blocking::run_blocking(move || backend.delete_batch(&ids)).await?;
    memory::emit_memory_changed("delete_batch", None, Some(deleted));
    Ok(deleted)
}

#[tauri::command]
pub async fn memory_get_import_from_ai_prompt(locale: Option<String>) -> Result<String, CmdError> {
    let locale = locale.as_deref().unwrap_or("en");
    Ok(memory::import_prompt::import_from_ai_prompt(locale).to_string())
}

#[tauri::command]
pub async fn memory_import(
    content: String,
    format: String,
    dedup: bool,
) -> Result<memory::ImportResult, CmdError> {
    let entries =
        ha_core::blocking::run_blocking(move || memory::parse_import(&content, &format)).await?;
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let result =
        ha_core::blocking::run_blocking(move || backend.import_entries(entries, dedup)).await?;
    if result.created > 0 {
        memory::emit_memory_changed("import", None, Some(result.created));
    }
    Ok(result)
}

#[tauri::command]
pub async fn memory_import_preview(
    content: String,
    format: String,
    limit: Option<usize>,
    dedup: Option<bool>,
) -> Result<memory::MemoryImportPreview, CmdError> {
    if let Some(backend) = get_memory_backend() {
        let backend = backend.clone();
        Ok(ha_core::blocking::run_blocking(move || {
            memory::preview_import_with_backend(
                backend.as_ref(),
                &content,
                &format,
                limit,
                dedup.unwrap_or(false),
            )
        })
        .await)
    } else {
        Ok(ha_core::blocking::run_blocking(move || {
            memory::preview_import(&content, &format, limit)
        })
        .await)
    }
}

#[tauri::command]
pub async fn memory_reembed(ids: Option<Vec<i64>>) -> Result<usize, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let count = ha_core::blocking::run_blocking(move || match ids {
        Some(ids) => backend.reembed_batch(&ids),
        None => backend.reembed_all(),
    })
    .await?;
    Ok(count)
}

#[tauri::command]
pub async fn memory_stats(
    scope: Option<memory::MemoryScope>,
) -> Result<memory::MemoryStats, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let stats = ha_core::blocking::run_blocking(move || backend.stats(scope.as_ref())).await?;
    Ok(stats)
}

#[tauri::command]
pub async fn memory_health() -> Result<memory::MemoryHealth, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.health())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_repair(
    action: memory::MemoryRepairAction,
) -> Result<memory::MemoryRepairReport, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let report = ha_core::blocking::run_blocking(move || backend.repair(action)).await?;
    memory::emit_memory_changed("repair", None, None);
    Ok(report)
}

#[tauri::command]
pub async fn memory_db_snapshot_restore_preview(
    snapshot_path: String,
) -> Result<memory::MemoryDbSnapshotRestorePreview, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    ha_core::blocking::run_blocking(move || backend.db_snapshot_restore_preview(&snapshot_path))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_db_snapshot_restore(
    snapshot_path: String,
) -> Result<memory::MemoryDbSnapshotRestoreReport, CmdError> {
    let backend = get_memory_backend()
        .ok_or_else(|| CmdError::msg("Memory backend not initialized"))?
        .clone();
    let report =
        ha_core::blocking::run_blocking(move || backend.db_snapshot_restore(&snapshot_path))
            .await?;
    memory::emit_memory_changed("db_snapshot_restore", None, None);
    Ok(report)
}

#[tauri::command]
pub async fn get_extract_config() -> Result<memory::MemoryExtractConfig, CmdError> {
    Ok(ha_core::config::cached_config().memory_extract.clone())
}

#[tauri::command]
pub async fn save_extract_config(config: memory::MemoryExtractConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_extract", "settings-ui"), move |store| {
        store.memory_extract = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_memory_selection_config() -> Result<memory::MemorySelectionConfig, CmdError> {
    Ok(ha_core::config::cached_config().memory_selection.clone())
}

#[tauri::command]
pub async fn save_memory_selection_config(
    config: memory::MemorySelectionConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_selection", "settings-ui"), move |store| {
        store.memory_selection = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_memory_budget_config() -> Result<memory::MemoryBudgetConfig, CmdError> {
    Ok(ha_core::config::cached_config().memory_budget.clone())
}

#[tauri::command]
pub async fn save_memory_budget_config(config: memory::MemoryBudgetConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_budget", "settings-ui"), move |store| {
        store.memory_budget = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_external_memory_providers_config(
) -> Result<memory::ExternalMemoryProvidersConfig, CmdError> {
    Ok(ha_core::config::cached_config().memory_providers.clone())
}

#[tauri::command]
pub async fn get_external_memory_providers_preflight(
) -> Result<memory::ExternalMemoryProviderPreflightReport, CmdError> {
    Ok(ha_core::blocking::run_blocking(memory::get_external_memory_provider_preflight).await)
}

#[tauri::command]
pub async fn run_external_memory_provider_sync(
) -> Result<memory::ExternalMemoryProviderSyncReport, CmdError> {
    Ok(memory::run_external_memory_provider_sync().await)
}

#[tauri::command]
pub async fn get_external_memory_provider_credential_status(
    provider_id: String,
) -> Result<memory::ExternalMemoryProviderCredentialStatus, CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::get_external_memory_provider_credential_status(&provider_id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn save_external_memory_provider_credentials(
    provider_id: String,
    mut credentials: memory::ExternalMemoryProviderCredentialInput,
) -> Result<memory::ExternalMemoryProviderCredentialStatus, CmdError> {
    if credentials.provider_id != provider_id {
        return Err(CmdError::from(anyhow::anyhow!(
            "provider id and credential body must match"
        )));
    }
    credentials.provider_id = provider_id;
    memory::save_external_memory_provider_credentials(credentials)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_external_memory_provider_credentials(
    provider_id: String,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::clear_external_memory_provider_credentials(&provider_id)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn save_external_memory_providers_config(
    config: memory::ExternalMemoryProvidersConfig,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::save_external_memory_providers_config(config, "settings-ui")
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_dedup_config() -> Result<memory::DedupConfig, CmdError> {
    Ok(ha_core::config::cached_config().dedup.clone())
}

#[tauri::command]
pub async fn save_dedup_config(config: memory::DedupConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_dedup", "settings-ui"), move |store| {
        store.dedup = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── Search Tuning Configs ──────────────────────────────────────

#[tauri::command]
pub async fn get_hybrid_search_config() -> Result<memory::HybridSearchConfig, CmdError> {
    Ok(ha_core::config::cached_config().hybrid_search.clone())
}

#[tauri::command]
pub async fn save_hybrid_search_config(config: memory::HybridSearchConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("hybrid_search", "settings-ui"), move |store| {
        store.hybrid_search = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_temporal_decay_config() -> Result<memory::TemporalDecayConfig, CmdError> {
    Ok(ha_core::config::cached_config().temporal_decay.clone())
}

#[tauri::command]
pub async fn save_temporal_decay_config(
    config: memory::TemporalDecayConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("temporal_decay", "settings-ui"), move |store| {
        store.temporal_decay = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_mmr_config() -> Result<memory::MmrConfig, CmdError> {
    Ok(ha_core::config::cached_config().mmr.clone())
}

#[tauri::command]
pub async fn save_mmr_config(config: memory::MmrConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("memory_mmr", "settings-ui"), move |store| {
        store.mmr = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_embedding_cache_config() -> Result<memory::EmbeddingCacheConfig, CmdError> {
    Ok(ha_core::config::cached_config().embedding_cache.clone())
}

#[tauri::command]
pub async fn save_embedding_cache_config(
    config: memory::EmbeddingCacheConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("embedding_cache", "settings-ui"), move |store| {
        store.embedding_cache = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_multimodal_config() -> Result<memory::MultimodalConfig, CmdError> {
    Ok(ha_core::config::cached_config().multimodal.clone())
}

#[tauri::command]
pub async fn save_multimodal_config(config: memory::MultimodalConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("multimodal", "settings-ui"), move |store| {
        store.multimodal = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_embedding_config() -> Result<memory::EmbeddingConfig, CmdError> {
    let store = ha_core::config::cached_config();
    let resolved =
        memory::resolve_memory_embedding_config(&store.memory_embedding, &store.embedding_models)?;
    Ok(resolved
        .map(|(_, config, _)| config)
        .unwrap_or_else(memory::EmbeddingConfig::default))
}

#[tauri::command]
pub async fn save_embedding_config(config: memory::EmbeddingConfig) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::save_legacy_embedding_config(config, "settings-ui")
    })
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn get_embedding_presets() -> Result<Vec<memory::EmbeddingPreset>, CmdError> {
    Ok(memory::embedding_presets())
}

#[tauri::command]
pub async fn list_local_embedding_models() -> Result<Vec<memory::LocalEmbeddingModel>, CmdError> {
    Ok(ha_core::blocking::run_blocking(memory::list_local_models_with_status).await)
}

#[tauri::command]
pub async fn embedding_model_config_list() -> Result<Vec<memory::EmbeddingModelConfig>, CmdError> {
    Ok(memory::list_embedding_model_configs())
}

#[tauri::command]
pub async fn embedding_model_config_templates(
) -> Result<Vec<memory::EmbeddingModelTemplate>, CmdError> {
    Ok(memory::embedding_model_config_templates())
}

#[tauri::command]
pub async fn embedding_model_config_save(
    config: memory::EmbeddingModelConfig,
) -> Result<memory::EmbeddingModelConfig, CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::save_embedding_model_config(config, "settings-ui")
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn embedding_model_config_delete(id: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::delete_embedding_model_config(&id, "settings-ui")
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn embedding_model_config_test(
    config: memory::EmbeddingModelConfig,
) -> Result<String, CmdError> {
    let config = config.normalize_for_save();
    config.validate()?;
    ha_core::provider::test::test_embedding(config.to_runtime_config(true))
        .await
        .map_err(CmdError::msg)
}

#[tauri::command]
pub async fn memory_embedding_get() -> Result<memory::EmbeddingSelectionState, CmdError> {
    Ok(memory::get_memory_embedding_state())
}

#[tauri::command]
pub async fn memory_embedding_set_default(
    model_config_id: String,
    mode: memory::ReembedMode,
) -> Result<memory::EmbeddingSetDefaultResult, CmdError> {
    ha_core::blocking::run_blocking(move || {
        memory::set_memory_embedding_default(&model_config_id, mode, "settings-ui", None)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn memory_reembed_start(
    mode: memory::ReembedMode,
) -> Result<ha_core::local_model_jobs::LocalModelJobSnapshot, CmdError> {
    let model_id = ha_core::config::cached_config()
        .memory_embedding
        .model_config_id
        .clone()
        .ok_or_else(|| {
            CmdError::msg("No memory embedding model is currently active".to_string())
        })?;
    let snapshot = ha_core::blocking::run_blocking(move || {
        memory::start_memory_reembed_job(&model_id, mode, None)
    })
    .await?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn memory_embedding_disable() -> Result<memory::EmbeddingSelectionState, CmdError> {
    ha_core::blocking::run_blocking(move || memory::disable_memory_embedding("settings-ui"))
        .await
        .map_err(Into::into)
}

// ── Core Memory (memory.md) commands ────────────────────────────

#[tauri::command]
pub async fn get_global_memory_md() -> Result<Option<String>, CmdError> {
    let path = crate::paths::root_dir()?.join("memory.md");
    ha_core::blocking::run_blocking(move || {
        if path.exists() {
            std::fs::read_to_string(&path).map(Some).map_err(Into::into)
        } else {
            Ok(None)
        }
    })
    .await
}

#[tauri::command]
pub async fn save_global_memory_md(content: String) -> Result<(), CmdError> {
    let path = crate::paths::root_dir()?.join("memory.md");
    ha_core::blocking::run_blocking(move || {
        ha_core::platform::write_atomic(&path, content.as_bytes()).map_err(Into::into)
    })
    .await
}

#[tauri::command]
pub async fn get_agent_memory_md(id: String) -> Result<Option<String>, CmdError> {
    let path = crate::paths::agent_dir(&id)?.join("memory.md");
    ha_core::blocking::run_blocking(move || {
        if path.exists() {
            std::fs::read_to_string(&path).map(Some).map_err(Into::into)
        } else {
            Ok(None)
        }
    })
    .await
}

#[tauri::command]
pub async fn save_agent_memory_md(id: String, content: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::agent_lifecycle::save_agent_memory_md(&id, &content).map_err(Into::into)
    })
    .await
}
