use crate::commands::CmdError;
use crate::session;
use crate::session::{ParentSessionFilter, ProjectFilter};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn create_session_cmd(
    agent_id: Option<String>,
    project_id: Option<String>,
    incognito: Option<bool>,
    state: State<'_, AppState>,
) -> Result<session::SessionMeta, CmdError> {
    // Caller-supplied agent_id wins. Otherwise resolve via the standard
    // precedence chain: project default → global default → hardcoded "ha-main".
    // Channel-level fallback is irrelevant here (this command is only invoked
    // from desktop / web, never from the IM channel worker path).
    let agent_id = match agent_id {
        Some(id) if !id.trim().is_empty() => id,
        _ => {
            let project = match project_id.clone() {
                Some(pid) => {
                    let project_db = state.project_db.clone();
                    ha_core::blocking::run_blocking(move || project_db.get(&pid))
                        .await
                        .ok()
                        .flatten()
                }
                None => None,
            };
            ha_core::agent::resolver::resolve_default_agent_id(project.as_ref(), None)
        }
    };
    // Reserve the Agent before the SessionDB write so a stale UI cannot create
    // a durable session for a disabled/deleted Agent or race deletion between
    // validation and insertion.
    let _agent_admission = ha_core::agent_lifecycle::begin_agent_run(&agent_id)?;
    state
        .session_db
        .run(move |db| db.create_session_with_project(&agent_id, project_id.as_deref(), incognito))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn fork_session_cmd(
    session_id: String,
    message_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<session::SessionMeta, CmdError> {
    state
        .session_db
        .run(move |db| db.fork_session(&session_id, message_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_sessions_cmd(
    agent_id: Option<String>,
    project_id: Option<String>,
    unassigned: Option<bool>,
    parent_session: Option<bool>,
    limit: Option<u32>,
    offset: Option<u32>,
    active_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMeta>, u32), CmdError> {
    let unassigned = unassigned.unwrap_or(false);
    let (mut sessions, total) = state
        .session_db
        .run(move |db| {
            // Precedence: explicit `unassigned=true` wins; then `project_id`; else All.
            let project_filter = if unassigned {
                ProjectFilter::Unassigned
            } else if let Some(ref pid) = project_id {
                ProjectFilter::InProject(pid.as_str())
            } else {
                ProjectFilter::All
            };
            let parent_filter = match parent_session {
                Some(true) => ParentSessionFilter::Child,
                Some(false) => ParentSessionFilter::Root,
                None => ParentSessionFilter::All,
            };
            db.list_sessions_paged_for_sidebar(
                agent_id.as_deref(),
                project_filter,
                parent_filter,
                limit,
                offset,
                active_session_id.as_deref(),
            )
        })
        .await?;

    session::enrich_pending_interactions(&mut sessions, &state.session_db).await?;

    Ok((sessions, total))
}

/// Authoritative count of unread regular conversations. The active session is
/// treated as read for display purposes without mutating its durable watermark.
#[tauri::command]
pub async fn regular_unread_total_cmd(
    active_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, CmdError> {
    state
        .session_db
        .run(move |db| db.regular_unread_total(active_session_id.as_deref()))
        .await
        .map_err(Into::into)
}

/// First unread regular conversation in sidebar visual order, with list offset.
#[tauri::command]
pub async fn next_unread_session_cmd(
    active_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<session::UnreadSessionTarget>, CmdError> {
    state
        .session_db
        .run(move |db| db.next_regular_unread_session(active_session_id.as_deref()))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn load_session_messages_latest_cmd(
    session_id: String,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, u32, bool), CmdError> {
    state
        .session_db
        .run(move |db| db.load_session_messages_latest(&session_id, limit))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn load_session_messages_before_cmd(
    session_id: String,
    before_id: i64,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, bool), CmdError> {
    state
        .session_db
        .run(move |db| db.load_session_messages_before(&session_id, before_id, limit))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn load_session_messages_after_cmd(
    session_id: String,
    after_id: i64,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, bool), CmdError> {
    state
        .session_db
        .run(move |db| db.load_session_messages_after(&session_id, after_id, limit))
        .await
        .map_err(Into::into)
}

/// Aggregate the session's workspace artifacts (files touched + URL sources)
/// over its FULL history. Desktop is trusted; scope/auth is enforced at the
/// HTTP boundary only.
#[tauri::command]
pub async fn load_session_artifacts_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<session::SessionArtifacts, CmdError> {
    state
        .session_db
        .run(move |db| session::aggregate_session_artifacts(db, &session_id))
        .await
        .map_err(Into::into)
}

/// Read-only environment snapshot for the workspace panel. This stays out of
/// the system prompt; it is only rendered in the UI.
#[tauri::command]
pub async fn load_session_environment_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<session::WorkspaceEnvironmentSnapshot, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || session::load_session_environment(&db, &session_id))
        .await
        .map_err(|e| CmdError::msg(format!("environment task join error: {e}")))?
        .map_err(Into::into)
}

/// Read the current Git working-tree diff for the session workspace. Desktop is
/// trusted; HTTP routes enforce the same session scope before reaching core.
#[tauri::command]
pub async fn load_session_git_diff_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<session::WorkspaceGitDiff, CmdError> {
    let db = state.session_db.clone();
    tokio::task::spawn_blocking(move || session::load_session_git_diff(&db, &session_id))
        .await
        .map_err(|e| CmdError::msg(format!("git diff task join error: {e}")))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<session::SessionMeta>, CmdError> {
    state
        .session_db
        .run(move |db| db.get_session(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_session_incognito(
    session_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.update_session_incognito(&session_id, enabled))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_memory_policy_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<session::SessionMemoryPolicy, CmdError> {
    state
        .session_db
        .run(move |db| db.get_memory_policy(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_session_memory_policy_cmd(
    session_id: String,
    policy: session::SessionMemoryPolicy,
    state: State<'_, AppState>,
) -> Result<session::SessionMemoryPolicy, CmdError> {
    let id = session_id.clone();
    let saved = state
        .session_db
        .run(move |db| db.set_memory_policy(&session_id, policy))
        .await?;
    ha_core::memory::core_repository::invalidate_session_snapshot(&id);
    if let Some(bus) = ha_core::get_event_bus() {
        let payload = serde_json::json!({ "sessionId": id, "policy": saved });
        bus.emit("memory:session_policy_changed", payload.clone());
        bus.emit("memory:policy_changed", payload);
    }
    Ok(saved)
}

/// Persist the user-selected working directory for a chat session. The core
/// layer canonicalizes the path and validates that it resolves to an existing
/// directory; `None` clears the selection.
#[tauri::command]
pub async fn set_session_working_dir(
    session_id: String,
    working_dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.update_session_working_dir(&session_id, working_dir))
        .await
        .map(|_| ())
        .map_err(Into::into)
}

/// Switch the agent bound to a session. Only valid before the session has
/// any user/assistant messages — see `SessionDB::update_session_agent`.
/// Used by the title-bar agent switcher in the UI.
#[tauri::command]
pub async fn update_session_agent_cmd(
    session_id: String,
    agent_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.update_session_agent(&session_id, &agent_id))
        .await
        .map_err(Into::into)
}

/// Pin the provider/model used by a chat session. The next chat turn will
/// resolve provider/model from this row before falling back to the agent's
/// `model.primary` and finally `config.active_model`. Replaces the legacy
/// "切模型 = 写全局" path so per-session selection no longer leaks into the
/// application-wide default.
#[tauri::command]
pub async fn set_session_model(
    session_id: String,
    provider_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    {
        let session_id = session_id.clone();
        let provider_id = provider_id.clone();
        let model_id = model_id.clone();
        state
            .session_db
            .run(move |db| {
                ha_core::session::set_session_model_preference(
                    db,
                    &session_id,
                    &provider_id,
                    &model_id,
                )
            })
            .await?;
    }
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "session:model_updated",
            serde_json::json!({
                "sessionId": session_id,
                "providerId": provider_id,
                "modelId": model_id,
            }),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn set_session_temperature(
    session_id: String,
    mode: String,
    value: Option<f64>,
    state: State<'_, AppState>,
) -> Result<Option<f64>, CmdError> {
    if !matches!(mode.as_str(), "value" | "agentDefault") {
        return Err(CmdError::msg(format!("Invalid temperature mode: {mode}")));
    }
    state
        .session_db
        .run(move |db| {
            ha_core::session::set_session_temperature_preference(
                db,
                &session_id,
                value,
                mode == "agentDefault",
            )
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_session_reasoning_effort(
    session_id: String,
    mode: String,
    value: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, CmdError> {
    if !matches!(mode.as_str(), "value" | "agentDefault") {
        return Err(CmdError::msg(format!(
            "Invalid reasoning effort mode: {mode}"
        )));
    }
    state
        .session_db
        .run(move |db| {
            ha_core::session::set_session_reasoning_effort_preference(
                db,
                &session_id,
                value.as_deref(),
                mode == "agentDefault",
            )
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_chat_runtime_defaults(
    session_id: Option<String>,
    agent_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ha_core::session::ChatRuntimeDefaults, CmdError> {
    if let Some(session_id) = session_id {
        return state
            .session_db
            .run(move |db| ha_core::session::ensure_session_runtime_defaults(db, &session_id))
            .await
            .map_err(Into::into);
    }
    let agent_id = agent_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| ha_core::agent_loader::DEFAULT_AGENT_ID.to_string());
    Ok(ha_core::session::resolve_chat_runtime_defaults(
        None, &agent_id,
    ))
}

#[tauri::command]
pub async fn delete_session_cmd(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.delete_session(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn purge_session_if_incognito(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<bool, CmdError> {
    state
        .session_db
        .run(move |db| db.purge_session_if_incognito(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rename_session_cmd(
    session_id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.update_session_title(&session_id, &title))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_session_pinned_cmd(
    session_id: String,
    pinned: bool,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.set_session_pinned(&session_id, pinned))
        .await
        .map_err(Into::into)
}

/// Mark a session read through the last message the UI actually rendered.
/// Omitting `through_message_id` preserves the explicit "mark all" behavior.
#[tauri::command]
pub async fn mark_session_read_cmd(
    session_id: String,
    through_message_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.mark_session_read_through(&session_id, through_message_id))
        .await
        .map_err(Into::into)
}

/// Mark all messages in multiple sessions as read.
#[tauri::command]
pub async fn mark_session_read_batch_cmd(
    session_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.mark_session_read_batch(&session_ids))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mark_all_sessions_read_cmd(state: State<'_, AppState>) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.mark_all_sessions_read())
        .await
        .map_err(Into::into)
}

/// Search message history (FTS5) across sessions.
///
/// `types` accepts any combination of `"regular"`, `"cron"`, `"subagent"`,
/// `"channel"`. Passing `None` or an empty vec returns results from all
/// session types.
#[tauri::command]
pub async fn search_sessions_cmd(
    query: String,
    agent_id: Option<String>,
    types: Option<Vec<String>>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionSearchResult>, CmdError> {
    let limit = limit.unwrap_or(80) as usize;

    let parsed_types: Option<Vec<session::SessionTypeFilter>> = types.map(|list| {
        list.iter()
            .filter_map(|s| session::SessionTypeFilter::parse(s))
            .collect()
    });

    state
        .session_db
        .run(move |db| {
            db.search_messages(
                &query,
                agent_id.as_deref(),
                None,
                parsed_types.as_deref(),
                limit,
            )
        })
        .await
        .map_err(Into::into)
}

/// Search message history within a single session (FTS5). Used by the
/// in-chat "find in page" search bar.
#[tauri::command]
pub async fn search_session_messages_cmd(
    session_id: String,
    query: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<session::SessionSearchResult>, CmdError> {
    let limit = limit.unwrap_or(200) as usize;
    state
        .session_db
        .run(move |db| db.search_messages(&query, None, Some(&session_id), None, limit))
        .await
        .map_err(Into::into)
}

/// Load a window of messages centred on a target message id (used by search
/// result "jump to message" flow).
#[tauri::command]
pub async fn load_session_messages_around_cmd(
    session_id: String,
    target_message_id: i64,
    before: u32,
    after: u32,
    state: State<'_, AppState>,
) -> Result<(Vec<session::SessionMessage>, u32, bool, bool), CmdError> {
    state
        .session_db
        .run(move |db| {
            db.load_session_messages_around(&session_id, target_message_id, before, after)
        })
        .await
        .map_err(Into::into)
}

/// Report whether a session currently has an active chat stream running in
/// the backend. Used by the frontend on session switch to decide whether to
/// attach the EventBus-backed stream listener for a reloaded window that
/// lost its per-call Channel.
#[tauri::command]
pub async fn get_session_stream_state(
    session_id: String,
) -> Result<ha_core::chat_engine::SessionStreamState, CmdError> {
    Ok(ha_core::chat_engine::session_stream_state(&session_id))
}

/// Return the durable journal snapshot used by reload handshakes. Unlike the
/// lightweight state endpoint, this contains the full visible stream prefix.
#[tauri::command]
pub async fn get_session_stream_snapshot(
    session_id: String,
) -> Result<Option<ha_core::chat_engine::SessionStreamSnapshot>, CmdError> {
    tokio::task::spawn_blocking(move || ha_core::chat_engine::session_stream_snapshot(&session_id))
        .await
        .map_err(|error| CmdError::msg(format!("stream snapshot task failed: {error}")))?
        .map_err(Into::into)
}

/// Serialize a session to disk in Markdown / JSON / HTML.
///
/// `output_path` is supplied by the frontend's native save dialog
/// (`@tauri-apps/plugin-dialog`); this command only writes the bytes to that
/// path. The HTTP twin is `GET /api/sessions/{id}/export` which returns the
/// payload as a binary response for the browser to download — they share the
/// same [`ha_core::session::export::export_session`] serializer.
#[tauri::command]
pub async fn export_session_cmd(
    session_id: String,
    format: String,
    include_thinking: bool,
    include_tools: bool,
    output_path: String,
    state: State<'_, AppState>,
) -> Result<String, CmdError> {
    let fmt = ha_core::session::export::ExportFormat::parse(&format).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid export format: `{}` (expected md / json / html)",
            format
        )
    })?;
    let opts = ha_core::session::export::ExportOptions {
        format: fmt,
        include_thinking,
        include_tools,
    };
    {
        let output_path = output_path.clone();
        state
            .session_db
            .run(move |db| -> Result<(), anyhow::Error> {
                let payload = ha_core::session::export::export_session(db, &session_id, opts)?;
                std::fs::write(&output_path, &payload.body)
                    .map_err(|e| anyhow::anyhow!("failed to write {}: {}", output_path, e))?;
                Ok(())
            })
            .await?;
    }
    Ok(output_path)
}
