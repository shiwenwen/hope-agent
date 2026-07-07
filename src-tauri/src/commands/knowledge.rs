//! Tauri commands for the Knowledge Base ("Knowledge Space") feature.
//!
//! Thin wrappers around `ha_core::knowledge` — all logic lives in ha-core. These
//! run on the **owner plane** (desktop = trusted local machine): the operator
//! sees all their knowledge bases, not gated by `effective_kb_access` (that gate
//! is for the agent `note_*` tools).

use crate::commands::CmdError;
use ha_core::filesystem::{self, ExtractedContent, FileTextContent, WorkspaceScope};
use ha_core::knowledge::{
    self, service, Backlink, BrokenLink, CompileProposal, CompileProposalStatus, CompileRun,
    CompileStartInput, CreateKnowledgeBaseInput, GraphNodePosition, KbAccess, KbAttachment,
    KbChatThread, KnowledgeAgentCompileProposeInput, KnowledgeAgentExpandInput,
    KnowledgeAgentExpandResult, KnowledgeAgentReadInput, KnowledgeAgentReadResult,
    KnowledgeAgentSearchInput, KnowledgeAgentSearchResult, KnowledgeAgentSourcesInput,
    KnowledgeAgentSourcesResult, KnowledgeBase, KnowledgeBaseMeta,
    KnowledgeBrowserSourceImportInput, KnowledgeCompileConfig, KnowledgeEvidenceClaim,
    KnowledgeEvidenceCoverage, KnowledgeEvidenceRebuildResult, KnowledgeGraph, KnowledgeSource,
    KnowledgeSourceAssetKind, KnowledgeSourceAssetLink, KnowledgeSourceDiff,
    KnowledgeSourceExternalRawSyncResult, KnowledgeSourceImportBatchInput,
    KnowledgeSourceImportInput, KnowledgeSourceImportRun, KnowledgeSourceImportRunDetail,
    KnowledgeSourceImportSessionAttachmentInput, KnowledgeSourceReadResult,
    KnowledgeSourceRefreshInput, KnowledgeSourceRefreshResult,
    KnowledgeSourceSimilarityDismissInput, KnowledgeSourceSimilarityGroup,
    KnowledgeSourceSimilarityResolveInput, KnowledgeSourceSimilarityResolveResult,
    KnowledgeSourceVersionHistory, Note, NoteReadResult, NoteSearchHit, NoteSourceRef,
    QueryFileInput, ReferenceableNote, RenameOutcome, SchemaIssue, SchemaProfile,
    UpdateKnowledgeBaseInput,
};
use ha_core::session::SessionMeta;

fn registry() -> Result<&'static std::sync::Arc<knowledge::KnowledgeRegistry>, CmdError> {
    ha_core::get_knowledge_db().ok_or_else(|| CmdError::msg("knowledge db not initialized"))
}

fn emit(name: &str, payload: serde_json::Value) {
    if let Some(bus) = ha_core::get_event_bus() {
        let _ = bus.emit(name, payload);
    }
}

// ── KB registry CRUD ────────────────────────────────────────────

#[tauri::command]
pub async fn list_kbs_cmd(
    include_archived: Option<bool>,
) -> Result<Vec<KnowledgeBaseMeta>, CmdError> {
    service::list_kb_meta(include_archived.unwrap_or(false)).map_err(Into::into)
}

#[tauri::command]
pub async fn get_kb_cmd(id: String) -> Result<Option<KnowledgeBase>, CmdError> {
    registry()?.get(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn create_kb_cmd(input: CreateKnowledgeBaseInput) -> Result<KnowledgeBase, CmdError> {
    let kb = registry()?.create(input)?;
    // Index + watch the new KB (external vaults get a full scan).
    knowledge::index::spawn_reindex_kb(kb.id.clone(), true);
    let _ = knowledge::watcher::start_watcher(&kb.id);
    emit("knowledge:created", serde_json::json!({ "kbId": kb.id }));
    Ok(kb)
}

#[tauri::command]
pub async fn update_kb_cmd(
    id: String,
    patch: UpdateKnowledgeBaseInput,
) -> Result<KnowledgeBase, CmdError> {
    let kb = registry()?.update(&id, patch)?;
    emit(
        "knowledge:changed",
        serde_json::json!({ "kbId": kb.id, "op": "meta" }),
    );
    Ok(kb)
}

#[tauri::command]
pub async fn delete_kb_cmd(id: String) -> Result<bool, CmdError> {
    knowledge::watcher::stop_watcher(&id);
    let removed = knowledge::delete_kb_cascade(&id)?;
    emit(
        "knowledge:changed",
        serde_json::json!({ "kbId": id, "op": "delete" }),
    );
    Ok(removed)
}

#[tauri::command]
pub async fn reindex_kb_cmd(id: String) -> Result<(), CmdError> {
    // Run through the KnowledgeReembed job so the UI gets progress. Single-KB
    // scope → no signature stamp (doesn't represent full coverage). Rebuilds
    // FTS + (if knowledge embedding is enabled) vectors.
    knowledge::start_knowledge_reembed_job(Some(vec![id]), "manual-reindex")?;
    Ok(())
}

/// Rebuild the index for a single note (per-note context-menu action). Runs
/// synchronously (one file → fast); rebuilds FTS + vectors (if enabled).
#[tauri::command]
pub async fn reindex_note_cmd(kb_id: String, path: String) -> Result<(), CmdError> {
    tokio::task::spawn_blocking(move || knowledge::index::reindex_note_by_path(&kb_id, &path))
        .await
        .map_err(|e| CmdError::msg(format!("reindex note task failed: {e}")))??;
    Ok(())
}

/// Rebuild the index for every note under a folder (per-folder context-menu
/// action). Synchronous; rebuilds FTS + vectors (if enabled). Does not prune.
#[tauri::command]
pub async fn reindex_dir_cmd(kb_id: String, path: String) -> Result<(), CmdError> {
    tokio::task::spawn_blocking(move || knowledge::index::reindex_dir(&kb_id, &path))
        .await
        .map_err(|e| CmdError::msg(format!("reindex dir task failed: {e}")))??;
    Ok(())
}

// ── Raw source inbox (Knowledge Compiler Phase 1) ────────────────

#[tauri::command]
pub async fn kb_source_import_cmd(
    kb_id: String,
    input: KnowledgeSourceImportInput,
) -> Result<KnowledgeSource, CmdError> {
    service::source_import(&kb_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_import_browser_cmd(
    kb_id: String,
    input: KnowledgeBrowserSourceImportInput,
) -> Result<KnowledgeSource, CmdError> {
    service::source_import_browser(&kb_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_import_session_attachment_cmd(
    kb_id: String,
    input: KnowledgeSourceImportSessionAttachmentInput,
) -> Result<KnowledgeSource, CmdError> {
    service::source_import_session_attachment(&kb_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_import_batch_cmd(
    kb_id: String,
    input: KnowledgeSourceImportBatchInput,
) -> Result<KnowledgeSourceImportRunDetail, CmdError> {
    service::source_import_batch(&kb_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_list_cmd(kb_id: String) -> Result<Vec<KnowledgeSource>, CmdError> {
    service::source_list(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_import_runs_list_cmd(
    kb_id: String,
    limit: Option<usize>,
) -> Result<Vec<KnowledgeSourceImportRun>, CmdError> {
    service::source_import_runs_list(&kb_id, limit).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_import_run_detail_cmd(
    kb_id: String,
    run_id: String,
) -> Result<KnowledgeSourceImportRunDetail, CmdError> {
    service::source_import_run_detail(&kb_id, &run_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_import_retry_failed_cmd(
    kb_id: String,
    run_id: String,
) -> Result<KnowledgeSourceImportRunDetail, CmdError> {
    service::source_import_retry_failed(&kb_id, &run_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub fn kb_source_similarity_groups_cmd(
    kb_id: String,
) -> Result<Vec<KnowledgeSourceSimilarityGroup>, CmdError> {
    service::source_similarity_groups(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub fn kb_source_similarity_dismiss_cmd(
    kb_id: String,
    input: KnowledgeSourceSimilarityDismissInput,
) -> Result<Vec<KnowledgeSourceSimilarityGroup>, CmdError> {
    service::source_similarity_dismiss(&kb_id, input).map_err(Into::into)
}

#[tauri::command]
pub fn kb_source_similarity_resolve_cmd(
    kb_id: String,
    input: KnowledgeSourceSimilarityResolveInput,
) -> Result<KnowledgeSourceSimilarityResolveResult, CmdError> {
    service::source_similarity_resolve(&kb_id, input).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_read_cmd(
    kb_id: String,
    source_id: String,
) -> Result<KnowledgeSourceReadResult, CmdError> {
    service::source_read(&kb_id, &source_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_asset_link_cmd(
    kb_id: String,
    source_id: String,
    kind: KnowledgeSourceAssetKind,
) -> Result<Option<KnowledgeSourceAssetLink>, CmdError> {
    service::source_asset_link(&kb_id, &source_id, kind).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_refresh_cmd(
    kb_id: String,
    source_id: String,
    input: KnowledgeSourceRefreshInput,
) -> Result<KnowledgeSourceRefreshResult, CmdError> {
    service::source_refresh(&kb_id, &source_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_versions_cmd(
    kb_id: String,
    source_id: String,
) -> Result<KnowledgeSourceVersionHistory, CmdError> {
    service::source_versions(&kb_id, &source_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_diff_cmd(
    kb_id: String,
    source_id: String,
    to_source_id: String,
) -> Result<KnowledgeSourceDiff, CmdError> {
    service::source_diff(&kb_id, &source_id, &to_source_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_reextract_cmd(
    kb_id: String,
    source_id: String,
) -> Result<KnowledgeSource, CmdError> {
    service::source_reextract(&kb_id, &source_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_delete_cmd(kb_id: String, source_id: String) -> Result<bool, CmdError> {
    service::source_delete(&kb_id, &source_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_source_sync_external_raw_cmd(
    kb_id: String,
) -> Result<KnowledgeSourceExternalRawSyncResult, CmdError> {
    service::source_sync_external_raw(&kb_id).map_err(Into::into)
}

// ── Knowledge Compiler (Phase 2) ─────────────────────────────────

#[tauri::command]
pub async fn kb_compile_start_cmd(
    kb_id: String,
    input: CompileStartInput,
) -> Result<CompileRun, CmdError> {
    service::compile_start(&kb_id, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_compile_status_cmd(run_id: String) -> Result<CompileRun, CmdError> {
    service::compile_status(&run_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_compile_runs_list_cmd(kb_id: String) -> Result<Vec<CompileRun>, CmdError> {
    service::compile_runs_list(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_compile_proposals_list_cmd(
    kb_id: String,
    run_id: Option<String>,
    status: Option<CompileProposalStatus>,
) -> Result<Vec<CompileProposal>, CmdError> {
    service::compile_proposals_list(&kb_id, run_id.as_deref(), status).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_compile_proposal_approve_cmd(id: i64) -> Result<CompileProposal, CmdError> {
    service::compile_proposal_approve(id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_compile_proposal_reject_cmd(id: i64) -> Result<bool, CmdError> {
    service::compile_proposal_reject(id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_compile_run_cancel_cmd(run_id: String) -> Result<CompileRun, CmdError> {
    service::compile_run_cancel(&run_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_query_file_cmd(
    kb_id: String,
    input: QueryFileInput,
) -> Result<CompileProposal, CmdError> {
    service::query_file(&kb_id, input).map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_compile_config_get_cmd() -> Result<KnowledgeCompileConfig, CmdError> {
    Ok(service::get_compile_config())
}

#[tauri::command]
pub async fn knowledge_compile_config_set_cmd(
    config: KnowledgeCompileConfig,
) -> Result<KnowledgeCompileConfig, CmdError> {
    service::set_compile_config(config, "desktop").map_err(Into::into)
}

// ── Schema profile + evidence refs (Phase 3) ─────────────────────

#[tauri::command]
pub async fn kb_schema_profile_cmd(kb_id: String) -> Result<SchemaProfile, CmdError> {
    service::schema_profile(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_schema_issues_cmd(kb_id: String) -> Result<Vec<SchemaIssue>, CmdError> {
    service::schema_issues(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_note_source_refs_cmd(
    kb_id: String,
    path: String,
) -> Result<Vec<NoteSourceRef>, CmdError> {
    service::note_source_refs(&kb_id, &path).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_evidence_coverage_cmd(
    kb_id: String,
) -> Result<KnowledgeEvidenceCoverage, CmdError> {
    service::evidence_coverage(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_evidence_source_claims_cmd(
    kb_id: String,
    source_id: String,
) -> Result<Vec<KnowledgeEvidenceClaim>, CmdError> {
    service::evidence_source_claims(&kb_id, &source_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_evidence_rebuild_cmd(
    kb_id: String,
) -> Result<KnowledgeEvidenceRebuildResult, CmdError> {
    service::evidence_rebuild(&kb_id).map_err(Into::into)
}

// ── Phase 6 external-agent API ──────────────────────────────────

#[tauri::command]
pub async fn knowledge_agent_search_cmd(
    input: KnowledgeAgentSearchInput,
) -> Result<KnowledgeAgentSearchResult, CmdError> {
    knowledge::agent_api::search(input).map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_agent_read_cmd(
    input: KnowledgeAgentReadInput,
) -> Result<KnowledgeAgentReadResult, CmdError> {
    knowledge::agent_api::read(input).map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_agent_expand_cmd(
    input: KnowledgeAgentExpandInput,
) -> Result<KnowledgeAgentExpandResult, CmdError> {
    knowledge::agent_api::expand(input).map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_agent_sources_cmd(
    input: KnowledgeAgentSourcesInput,
) -> Result<KnowledgeAgentSourcesResult, CmdError> {
    knowledge::agent_api::sources(input).map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_agent_compile_propose_cmd(
    input: KnowledgeAgentCompileProposeInput,
) -> Result<CompileRun, CmdError> {
    knowledge::agent_api::compile_propose(input)
        .await
        .map_err(Into::into)
}

// ── Access bindings ─────────────────────────────────────────────

#[tauri::command]
pub async fn attach_session_kb_cmd(
    session_id: String,
    kb_id: String,
    access: Option<String>,
) -> Result<(), CmdError> {
    let access = KbAccess::from_str_lenient(access.as_deref().unwrap_or("read"));
    registry()?.attach_session(&session_id, &kb_id, access)?;
    emit(
        "knowledge:changed",
        serde_json::json!({ "kbId": kb_id, "op": "attach" }),
    );
    Ok(())
}

#[tauri::command]
pub async fn detach_session_kb_cmd(session_id: String, kb_id: String) -> Result<(), CmdError> {
    registry()?.detach_session(&session_id, &kb_id)?;
    emit(
        "knowledge:changed",
        serde_json::json!({ "kbId": kb_id, "op": "detach" }),
    );
    Ok(())
}

#[tauri::command]
pub async fn attach_project_kb_cmd(
    project_id: String,
    kb_id: String,
    access: Option<String>,
) -> Result<(), CmdError> {
    let access = KbAccess::from_str_lenient(access.as_deref().unwrap_or("read"));
    registry()?.attach_project(&project_id, &kb_id, access)?;
    emit(
        "knowledge:changed",
        serde_json::json!({ "kbId": kb_id, "op": "attach" }),
    );
    Ok(())
}

#[tauri::command]
pub async fn detach_project_kb_cmd(project_id: String, kb_id: String) -> Result<(), CmdError> {
    registry()?.detach_project(&project_id, &kb_id)?;
    emit(
        "knowledge:changed",
        serde_json::json!({ "kbId": kb_id, "op": "detach" }),
    );
    Ok(())
}

/// Effective (session ∪ project) KB attachments for the "currently active
/// knowledge bases" UI list. Owner view: shows attachments regardless of source
/// caps (the agent plane applies the IM/incognito/external caps separately).
#[tauri::command]
pub async fn list_session_kbs_cmd(
    session_id: String,
    project_id: Option<String>,
) -> Result<Vec<KbAttachment>, CmdError> {
    let reg = registry()?;
    let mut out: Vec<KbAttachment> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (kb_id, access) in reg.list_session_attachments(&session_id)? {
        if let Ok(Some(kb)) = reg.get(&kb_id) {
            seen.insert(kb_id);
            out.push(KbAttachment {
                kb,
                access,
                via: "session".into(),
            });
        }
    }
    if let Some(pid) = project_id {
        for (kb_id, access) in reg.list_project_attachments(&pid)? {
            if seen.contains(&kb_id) {
                continue;
            }
            if let Ok(Some(kb)) = reg.get(&kb_id) {
                out.push(KbAttachment {
                    kb,
                    access,
                    via: "project".into(),
                });
            }
        }
    }
    Ok(out)
}

/// Project-scoped KB attachments for the project settings UI (owner plane).
#[tauri::command]
pub async fn list_project_kbs_cmd(project_id: String) -> Result<Vec<KbAttachment>, CmdError> {
    let reg = registry()?;
    let mut out: Vec<KbAttachment> = Vec::new();
    for (kb_id, access) in reg.list_project_attachments(&project_id)? {
        if let Ok(Some(kb)) = reg.get(&kb_id) {
            out.push(KbAttachment {
                kb,
                access,
                via: "project".into(),
            });
        }
    }
    Ok(out)
}

// ── Notes (owner plane) ─────────────────────────────────────────

#[tauri::command]
pub async fn list_kb_notes_cmd(kb_id: String) -> Result<Vec<Note>, CmdError> {
    service::list_notes(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_note_read_cmd(kb_id: String, path: String) -> Result<NoteReadResult, CmdError> {
    service::note_read(&kb_id, &path).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_note_save_cmd(
    kb_id: String,
    path: String,
    content: String,
    expected_file_hash: Option<String>,
    create_only: Option<bool>,
) -> Result<String, CmdError> {
    service::note_save(
        &kb_id,
        &path,
        &content,
        expected_file_hash.as_deref(),
        create_only.unwrap_or(false),
    )
    .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_note_delete_cmd(kb_id: String, path: String) -> Result<(), CmdError> {
    service::note_delete(&kb_id, &path).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_note_rename_cmd(
    kb_id: String,
    path: String,
    new_path: String,
) -> Result<RenameOutcome, CmdError> {
    service::note_rename(&kb_id, &path, &new_path)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_list_dirs_cmd(kb_id: String) -> Result<Vec<String>, CmdError> {
    service::list_dirs(&kb_id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn kb_list_tags_cmd(kb_id: String) -> Result<Vec<String>, CmdError> {
    service::list_tags(&kb_id).map_err(Into::into)
}

// ── Knowledge embedding selection (D7, owner plane) ─────────────
// Independent of memory_embedding; draws from the shared `embedding_models`
// library. Set/disable carry a background reindex side effect, so they live
// behind dedicated commands rather than generic `update_settings`.

#[tauri::command]
pub async fn knowledge_embedding_get_cmd(
) -> Result<ha_core::memory::EmbeddingSelectionState, CmdError> {
    Ok(knowledge::get_knowledge_embedding_state())
}

#[tauri::command]
pub async fn knowledge_embedding_set_default_cmd(
    model_config_id: String,
) -> Result<ha_core::memory::EmbeddingSelectionState, CmdError> {
    knowledge::set_knowledge_embedding_default(&model_config_id, "settings-ui").map_err(Into::into)
}

#[tauri::command]
pub async fn knowledge_embedding_disable_cmd(
) -> Result<ha_core::memory::EmbeddingSelectionState, CmdError> {
    knowledge::disable_knowledge_embedding("settings-ui").map_err(Into::into)
}

/// Force a full rebuild of every KB under the active model. Unlike
/// `set_default`, this never short-circuits on a same signature — it's the
/// explicit "rebuild now" action. Progress comes through the KnowledgeReembed
/// job stream.
#[tauri::command]
pub async fn knowledge_embedding_rebuild_cmd() -> Result<(), CmdError> {
    knowledge::start_knowledge_reembed_job(None, "manual-rebuild")?;
    Ok(())
}

/// Current knowledge chunking parameters (advanced; GUI-only like embedding).
#[tauri::command]
pub async fn knowledge_chunk_get_cmd() -> Result<ha_core::knowledge::ChunkConfig, CmdError> {
    Ok(knowledge::get_chunk_config())
}

/// Save chunking parameters and trigger a full reindex of every KB. Values are
/// clamped server-side; the clamped result is returned.
#[tauri::command]
pub async fn knowledge_chunk_set_cmd(
    max_chars: usize,
    overlap_chars: usize,
) -> Result<ha_core::knowledge::ChunkConfig, CmdError> {
    knowledge::set_chunk_config(max_chars, overlap_chars, "settings-ui").map_err(Into::into)
}

/// Current knowledge hybrid-search ranking parameters (clamped).
#[tauri::command]
pub async fn knowledge_search_config_get_cmd(
) -> Result<ha_core::knowledge::KnowledgeSearchConfig, CmdError> {
    Ok(knowledge::get_search_config())
}

/// Save search ranking parameters (clamped server-side, no reindex). The clamped
/// result is returned. Send default values to restore defaults.
#[tauri::command]
pub async fn knowledge_search_config_set_cmd(
    config: ha_core::knowledge::KnowledgeSearchConfig,
) -> Result<ha_core::knowledge::KnowledgeSearchConfig, CmdError> {
    knowledge::set_search_config(config, "settings-ui").map_err(Into::into)
}

/// Flat list of notes the chat composer can reference via `[[ ]]`. Pass
/// `draft_kb_ids` for a brand-new chat whose attaches aren't persisted yet.
#[tauri::command]
pub async fn list_referenceable_notes_cmd(
    session_id: Option<String>,
    project_id: Option<String>,
    draft_kb_ids: Option<Vec<String>>,
) -> Result<Vec<ReferenceableNote>, CmdError> {
    service::list_referenceable_notes(
        session_id.as_deref(),
        project_id.as_deref(),
        &draft_kb_ids.unwrap_or_default(),
    )
    .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_mkdir_cmd(kb_id: String, path: String) -> Result<String, CmdError> {
    service::mkdir(&kb_id, &path).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_rename_dir_cmd(
    kb_id: String,
    path: String,
    new_path: String,
) -> Result<RenameOutcome, CmdError> {
    service::rename_dir(&kb_id, &path, &new_path)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn kb_delete_dir_cmd(kb_id: String, path: String) -> Result<(), CmdError> {
    service::delete_dir(&kb_id, &path).await.map_err(Into::into)
}

#[tauri::command]
pub async fn kb_backlinks_cmd(kb_id: String, path: String) -> Result<Vec<Backlink>, CmdError> {
    service::backlinks(&kb_id, &path).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_broken_links_cmd(kb_id: String) -> Result<Vec<BrokenLink>, CmdError> {
    service::broken_links(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_orphans_cmd(kb_id: String) -> Result<Vec<Note>, CmdError> {
    service::orphans(&kb_id).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_graph_cmd(kb_id: String) -> Result<KnowledgeGraph, CmdError> {
    service::graph(&kb_id).map_err(Into::into)
}

/// Read the user-pinned graph layout (Batch J).
#[tauri::command]
pub async fn kb_graph_layout_get_cmd(kb_id: String) -> Result<Vec<GraphNodePosition>, CmdError> {
    service::graph_layout(&kb_id).map_err(Into::into)
}

/// Replace the user-pinned graph layout (Batch J). Empty `positions` resets it.
#[tauri::command]
pub async fn kb_graph_layout_save_cmd(
    kb_id: String,
    positions: Vec<GraphNodePosition>,
) -> Result<(), CmdError> {
    service::save_graph_layout(&kb_id, &positions).map_err(Into::into)
}

// ── Knowledge-space sidebar chat threads ────────────────────────

/// Default-load target: the most recent chat thread anchored to `note` in this
/// KB. `None` when the note has no prior conversation (panel shows empty state).
#[tauri::command]
pub async fn kb_chat_thread_get_cmd(
    kb_id: String,
    note: Option<String>,
) -> Result<Option<SessionMeta>, CmdError> {
    service::kb_chat_thread_latest(&kb_id, note.as_deref()).map_err(Into::into)
}

/// History picker: a page of chat threads in a KB, newest-active first. `query`
/// FTS-filters by message content when non-empty; `limit`/`offset` paginate.
#[tauri::command]
pub async fn kb_chat_threads_list_cmd(
    kb_id: String,
    query: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<KbChatThread>, CmdError> {
    service::kb_chat_threads_list(&kb_id, query.as_deref(), limit, offset).map_err(Into::into)
}

/// Quick-rewrite of a text selection: returns rewritten Markdown for the GUI to
/// diff in the floating bar; nothing is written until the user applies it and the
/// normal save runs. `model_override` (`"providerId::modelId"`) pins the model —
/// the panel defaults it to the current conversation's model.
#[tauri::command]
pub async fn kb_ai_rewrite_cmd(
    text: String,
    instruction: String,
    model_override: Option<String>,
) -> Result<String, CmdError> {
    service::ai_rewrite(&text, &instruction, model_override.as_deref())
        .await
        .map_err(Into::into)
}

/// Record a quick-rewrite outcome (applied / discarded) for statistics. Called by
/// the floating bar after the user decides. Best-effort; never blocks the action.
#[tauri::command]
pub async fn kb_rewrite_log_cmd(
    kb_id: String,
    note_path: Option<String>,
    instruction: String,
    model: Option<String>,
    chars_before: i64,
    chars_after: i64,
    accepted: bool,
) -> Result<(), CmdError> {
    service::log_quick_rewrite(
        &kb_id,
        note_path.as_deref(),
        &instruction,
        model.as_deref(),
        chars_before,
        chars_after,
        accepted,
    );
    Ok(())
}

// ── Layer-2 autonomous maintenance (WS6) ────────────────────────

/// Manually run one maintenance cycle (generate proposals across all KBs).
#[tauri::command]
pub async fn kb_maintenance_run_cmd() -> Result<knowledge::maintenance::MaintenanceReport, CmdError>
{
    Ok(
        knowledge::maintenance::manual_run(knowledge::maintenance::MaintenanceTrigger::Manual)
            .await,
    )
}

/// Running flag + last cycle report.
#[tauri::command]
pub async fn kb_maintenance_status_cmd(
) -> Result<knowledge::maintenance::MaintenanceStatus, CmdError> {
    Ok(knowledge::maintenance::status())
}

/// List proposals for a KB (optionally filtered by status: draft/applied/rejected/failed).
#[tauri::command]
pub async fn kb_maintenance_list_cmd(
    kb_id: String,
    status: Option<String>,
) -> Result<Vec<knowledge::maintenance::MaintenanceProposal>, CmdError> {
    let st = status
        .as_deref()
        .and_then(knowledge::maintenance::ProposalStatus::from_str);
    knowledge::maintenance::list_proposals(&kb_id, st).map_err(Into::into)
}

/// Pending (draft) proposal count for a KB (review-queue badge).
#[tauri::command]
pub async fn kb_maintenance_pending_count_cmd(kb_id: String) -> Result<usize, CmdError> {
    knowledge::maintenance::pending_count(&kb_id).map_err(Into::into)
}

/// Approve a proposal — applies it through the owner plane.
#[tauri::command]
pub async fn kb_maintenance_approve_cmd(
    id: i64,
) -> Result<knowledge::maintenance::MaintenanceProposal, CmdError> {
    knowledge::maintenance::approve_proposal(id)
        .await
        .map_err(Into::into)
}

/// Reject a single proposal.
#[tauri::command]
pub async fn kb_maintenance_reject_cmd(id: i64) -> Result<(), CmdError> {
    knowledge::maintenance::reject_proposal(id).map_err(Into::into)
}

/// Reject every pending proposal for a KB. Returns how many were cleared.
#[tauri::command]
pub async fn kb_maintenance_reject_all_cmd(kb_id: String) -> Result<usize, CmdError> {
    knowledge::maintenance::reject_all(&kb_id).map_err(Into::into)
}

/// Read the maintenance config (GUI panel).
#[tauri::command]
pub async fn kb_maintenance_config_get_cmd(
) -> Result<knowledge::maintenance::MaintenanceConfig, CmdError> {
    Ok(service::get_maintenance_config())
}

/// Persist the maintenance config (GUI panel). Returns the clamped value saved.
#[tauri::command]
pub async fn kb_maintenance_config_set_cmd(
    config: knowledge::maintenance::MaintenanceConfig,
) -> Result<knowledge::maintenance::MaintenanceConfig, CmdError> {
    service::set_maintenance_config(config, "gui").map_err(Into::into)
}

// ── Sprite / inspiration mode ───────────────────────────────────

/// Edit-idle trigger from the knowledge panel. Fire-and-forget: throttling +
/// side_query happen in the background; a suggestion (if any) arrives via the
/// `sprite:suggestion` event.
#[tauri::command]
pub async fn kb_sprite_observe_cmd(
    params: ha_core::sprite::SpriteObserveParams,
) -> Result<(), CmdError> {
    tokio::spawn(async move {
        let _ = ha_core::sprite::observe_and_maybe_speak(params).await;
    });
    Ok(())
}

/// Read the sprite config (GUI panel).
#[tauri::command]
pub async fn sprite_config_get_cmd() -> Result<ha_core::sprite::SpriteConfig, CmdError> {
    Ok(ha_core::sprite::get_config())
}

/// Persist the sprite config (GUI panel). Returns the clamped value saved.
#[tauri::command]
pub async fn sprite_config_set_cmd(
    config: ha_core::sprite::SpriteConfig,
) -> Result<ha_core::sprite::SpriteConfig, CmdError> {
    ha_core::sprite::set_config(config, "gui").map_err(Into::into)
}

/// Read the passive related-notes config (GUI panel, read bridge ③).
#[tauri::command]
pub async fn kb_passive_recall_config_get_cmd() -> Result<knowledge::PassiveRecallConfig, CmdError>
{
    Ok(service::get_passive_recall_config())
}

/// Persist the passive related-notes config (GUI panel). Returns the clamped value.
#[tauri::command]
pub async fn kb_passive_recall_config_set_cmd(
    config: knowledge::PassiveRecallConfig,
) -> Result<knowledge::PassiveRecallConfig, CmdError> {
    service::set_passive_recall_config(config, "gui").map_err(Into::into)
}

/// Read optional original-media retention config (GUI panel).
#[tauri::command]
pub async fn knowledge_media_retention_config_get_cmd(
) -> Result<knowledge::KnowledgeMediaRetentionConfig, CmdError> {
    Ok(service::get_media_retention_config())
}

/// Persist optional original-media retention config (GUI panel).
#[tauri::command]
pub async fn knowledge_media_retention_config_set_cmd(
    config: knowledge::KnowledgeMediaRetentionConfig,
) -> Result<knowledge::KnowledgeMediaRetentionConfig, CmdError> {
    service::set_media_retention_config(config, "gui").map_err(Into::into)
}

/// Resolve a `[[ ]]` reference to a note and return its full read result (for
/// `![[ ]]` transclusion preview). `Ok(None)` = broken embed.
#[tauri::command]
pub async fn kb_note_read_ref_cmd(
    kb_id: String,
    reference: String,
) -> Result<Option<NoteReadResult>, CmdError> {
    service::note_read_ref(&kb_id, &reference).map_err(Into::into)
}

#[tauri::command]
pub async fn kb_search_cmd(
    query: String,
    kb_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<NoteSearchHit>, CmdError> {
    let limit = limit.unwrap_or(20).clamp(1, 100) as usize;
    service::search(kb_id.as_deref(), &query, limit).map_err(Into::into)
}

// ── KB file preview plane (owner) ───────────────────────────────

async fn blocking<T, F>(f: F) -> Result<T, CmdError>
where
    F: FnOnce() -> filesystem::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::msg(format!("fs task join error: {e}")))?
        .map_err(|e| CmdError::msg(e.message().to_string()))
}

#[tauri::command]
pub async fn kb_file_read_cmd(kb_id: String, path: String) -> Result<FileTextContent, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::for_knowledge(&kb_id)?;
        filesystem::project_read_text(&s, &path)
    })
    .await
}

#[tauri::command]
pub async fn kb_file_extract_cmd(
    kb_id: String,
    path: String,
) -> Result<ExtractedContent, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::for_knowledge(&kb_id)?;
        filesystem::project_fs_extract(&s, &path)
    })
    .await
}

/// Desktop "raw": resolve to an absolute path for `convertFileSrc` (mirrors
/// `project_fs_resolve`). The HTTP transport uses the `/raw` endpoint instead.
#[tauri::command]
pub async fn kb_file_resolve_cmd(kb_id: String, path: String) -> Result<String, CmdError> {
    blocking(move || {
        let s = WorkspaceScope::for_knowledge(&kb_id)?;
        let abs = s.resolve_existing(&path)?;
        Ok(abs.to_string_lossy().to_string())
    })
    .await
}
