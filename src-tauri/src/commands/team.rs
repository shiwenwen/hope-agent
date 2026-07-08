use crate::commands::CmdError;
use crate::AppState;
use ha_core::team;
use tauri::State;

#[tauri::command]
pub async fn list_teams(
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<team::Team>, CmdError> {
    state
        .session_db
        .run(move |db| match session_id {
            Some(sid) => db.list_teams_by_session(&sid),
            None => db.list_active_teams(),
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_team(
    team_id: String,
    state: State<'_, AppState>,
) -> Result<Option<team::Team>, CmdError> {
    state
        .session_db
        .run(move |db| db.get_team(&team_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_team_members(
    team_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamMember>, CmdError> {
    state
        .session_db
        .run(move |db| db.list_team_members(&team_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_team_messages(
    team_id: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<(Vec<team::TeamMessage>, bool), CmdError> {
    state
        .session_db
        .run(move |db| db.list_team_messages_latest(&team_id, limit.unwrap_or(50)))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_team_messages_before(
    team_id: String,
    before_timestamp: String,
    before_message_id: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<(Vec<team::TeamMessage>, bool), CmdError> {
    state
        .session_db
        .run(move |db| {
            db.list_team_messages_before(
                &team_id,
                &before_timestamp,
                &before_message_id,
                limit.unwrap_or(50),
            )
        })
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_team_tasks(
    team_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamTask>, CmdError> {
    state
        .session_db
        .run(move |db| db.list_team_tasks(&team_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn send_user_team_message(
    team_id: String,
    to: Option<String>,
    content: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || {
        team::messaging::send_message(
            &db,
            &team_id,
            "*user*",
            to.as_deref(),
            &content,
            team::TeamMessageType::Chat,
        )
    })
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn list_team_templates(
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamTemplate>, CmdError> {
    let db = state.session_db.clone();
    Ok(ha_core::blocking::run_blocking(move || team::templates::all_templates(&db)).await)
}

#[tauri::command]
pub async fn create_team(
    name: String,
    description: Option<String>,
    session_id: String,
    agent_id: String,
    members: Vec<team::CreateTeamMemberSpec>,
    template: Option<String>,
    state: State<'_, AppState>,
) -> Result<team::Team, CmdError> {
    let (member_specs, resolved_template_id) = if !members.is_empty() {
        (members, template.clone())
    } else if let Some(ref tpl_name) = template {
        let templates = {
            let db = state.session_db.clone();
            ha_core::blocking::run_blocking(move || team::templates::all_templates(&db)).await
        };
        let tpl = templates
            .iter()
            .find(|t| t.template_id == *tpl_name || t.name.eq_ignore_ascii_case(tpl_name))
            .ok_or_else(|| CmdError::msg(format!("Template '{}' not found", tpl_name)))?;
        let specs = tpl
            .members
            .iter()
            .map(|m| team::CreateTeamMemberSpec {
                name: m.name.clone(),
                agent_id: m.agent_id.clone(),
                role: Some(m.role.as_str().to_string()),
                task: m
                    .default_task_template
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| {
                        format!("Work on your role '{}' as part of team '{}'.", m.name, name)
                    }),
                model: m.model_override.clone(),
                description: Some(m.description.clone()).filter(|s| !s.trim().is_empty()),
            })
            .collect();
        (specs, Some(tpl.template_id.clone()))
    } else {
        return Err(CmdError::msg("Either 'members' or 'template' is required"));
    };

    team::coordinator::create_team(
        &state.session_db,
        &name,
        description.as_deref(),
        &session_id,
        &agent_id,
        &member_specs,
        resolved_template_id.as_deref(),
        None,
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn save_team_template(
    template: team::TeamTemplate,
    state: State<'_, AppState>,
) -> Result<team::TeamTemplate, CmdError> {
    state
        .session_db
        .run(move |db| team::templates::save_template(db, template))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_team_template(
    template_id: String,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| team::templates::delete_template(db, &template_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pause_team(team_id: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    let db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || team::coordinator::pause_team(&db, &team_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_team(team_id: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    team::coordinator::resume_team(&state.session_db, &team_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn dissolve_team(team_id: String, state: State<'_, AppState>) -> Result<(), CmdError> {
    let db = state.session_db.clone();
    ha_core::blocking::run_blocking(move || team::coordinator::dissolve_team(&db, &team_id))
        .await
        .map_err(Into::into)
}
