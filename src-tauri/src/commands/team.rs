use crate::AppState;
use ha_core::team;
use tauri::State;

#[tauri::command]
pub async fn list_teams(
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<team::Team>, String> {
    if let Some(sid) = session_id {
        state
            .session_db
            .list_teams_by_session(&sid)
            .map_err(|e| e.to_string())
    } else {
        state
            .session_db
            .list_active_teams()
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn get_team(
    team_id: String,
    state: State<'_, AppState>,
) -> Result<Option<team::Team>, String> {
    state
        .session_db
        .get_team(&team_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_team_members(
    team_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamMember>, String> {
    state
        .session_db
        .list_team_members(&team_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_team_messages(
    team_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamMessage>, String> {
    state
        .session_db
        .list_team_messages(&team_id, limit.unwrap_or(100))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_team_tasks(
    team_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamTask>, String> {
    state
        .session_db
        .list_team_tasks(&team_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_user_team_message(
    team_id: String,
    to: Option<String>,
    content: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    team::messaging::send_message(
        &state.session_db,
        &team_id,
        "*user*",
        to.as_deref(),
        &content,
        team::TeamMessageType::Chat,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn list_team_templates(
    state: State<'_, AppState>,
) -> Result<Vec<team::TeamTemplate>, String> {
    Ok(team::templates::all_templates(&state.session_db))
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
) -> Result<team::Team, String> {
    let (member_specs, resolved_template_id) = if !members.is_empty() {
        (members, template.clone())
    } else if let Some(ref tpl_name) = template {
        let templates = team::templates::all_templates(&state.session_db);
        let tpl = templates
            .iter()
            .find(|t| t.template_id == *tpl_name || t.name.eq_ignore_ascii_case(tpl_name))
            .ok_or_else(|| format!("Template '{}' not found", tpl_name))?;
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
        return Err("Either 'members' or 'template' is required".to_string());
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
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_team_template(
    template: team::TeamTemplate,
    state: State<'_, AppState>,
) -> Result<team::TeamTemplate, String> {
    team::templates::save_template(&state.session_db, template).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_team_template(
    template_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    team::templates::delete_template(&state.session_db, &template_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pause_team(team_id: String, state: State<'_, AppState>) -> Result<(), String> {
    team::coordinator::pause_team(&state.session_db, &team_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_team(team_id: String, state: State<'_, AppState>) -> Result<(), String> {
    team::coordinator::resume_team(&state.session_db, &team_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dissolve_team(team_id: String, state: State<'_, AppState>) -> Result<(), String> {
    team::coordinator::dissolve_team(&state.session_db, &team_id).map_err(|e| e.to_string())
}
