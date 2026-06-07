//! Tauri commands for the Knowledge Base ("Knowledge Space") feature.
//!
//! Thin wrappers around `ha_core::knowledge` — all logic lives in ha-core. These
//! run on the **owner plane** (desktop = trusted local machine): the operator
//! sees all their knowledge bases, not gated by `effective_kb_access` (that gate
//! is for the agent `note_*` tools).

use crate::commands::CmdError;
use ha_core::filesystem::{self, ExtractedContent, FileTextContent, WorkspaceScope};
use ha_core::knowledge::{
    self, service, Backlink, BrokenLink, CreateKnowledgeBaseInput, KbAccess, KbAttachment,
    KnowledgeBase, KnowledgeBaseMeta, KnowledgeGraph, Note, NoteReadResult, NoteSearchHit,
    ReferenceableNote, RenameOutcome, UpdateKnowledgeBaseInput,
};

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

/// AI-assisted note rewrite (WS9): returns rewritten Markdown for the GUI to diff;
/// nothing is written until the user confirms and the normal save runs.
#[tauri::command]
pub async fn kb_ai_rewrite_cmd(text: String, instruction: String) -> Result<String, CmdError> {
    service::ai_rewrite(&text, &instruction)
        .await
        .map_err(Into::into)
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
