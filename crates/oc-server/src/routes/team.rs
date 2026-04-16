use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use oc_core::team;

use crate::error::AppError;
use crate::routes::helpers::app_state as state;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTeamsQuery {
    pub session_id: Option<String>,
}

/// `GET /api/teams`
pub async fn list_teams(
    Query(q): Query<ListTeamsQuery>,
) -> Result<Json<Vec<team::Team>>, AppError> {
    let s = state()?;
    if let Some(sid) = q.session_id {
        Ok(Json(s.session_db.list_teams_by_session(&sid)?))
    } else {
        Ok(Json(s.session_db.list_active_teams()?))
    }
}

/// `GET /api/teams/:id`
pub async fn get_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    Ok(Json(serde_json::to_value(
        state()?.session_db.get_team(&team_id)?,
    )?))
}

/// `GET /api/teams/:id/members`
pub async fn get_team_members(
    Path(team_id): Path<String>,
) -> Result<Json<Vec<team::TeamMember>>, AppError> {
    Ok(Json(state()?.session_db.list_team_members(&team_id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesQuery {
    pub limit: Option<usize>,
}

/// `GET /api/teams/:id/messages`
pub async fn get_team_messages(
    Path(team_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<Vec<team::TeamMessage>>, AppError> {
    Ok(Json(
        state()?
            .session_db
            .list_team_messages(&team_id, q.limit.unwrap_or(100))?,
    ))
}

/// `GET /api/teams/:id/tasks`
pub async fn get_team_tasks(
    Path(team_id): Path<String>,
) -> Result<Json<Vec<team::TeamTask>>, AppError> {
    Ok(Json(state()?.session_db.list_team_tasks(&team_id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageBody {
    pub to: Option<String>,
    pub content: String,
}

/// `POST /api/teams/:id/messages`
pub async fn send_user_team_message(
    Path(team_id): Path<String>,
    Json(body): Json<SendMessageBody>,
) -> Result<Json<Value>, AppError> {
    let s = state()?;
    let msg = team::messaging::send_message(
        &s.session_db,
        &team_id,
        "*user*",
        body.to.as_deref(),
        &body.content,
        team::TeamMessageType::Chat,
    )?;
    Ok(Json(json!({ "messageId": msg.message_id })))
}

/// `GET /api/team-templates`
pub async fn list_team_templates() -> Result<Json<Vec<team::TeamTemplate>>, AppError> {
    Ok(Json(team::templates::all_templates(&state()?.session_db)))
}

/// `POST /api/teams/:id/pause`
pub async fn pause_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    team::coordinator::pause_team(&state()?.session_db, &team_id)?;
    Ok(Json(json!({ "status": "paused" })))
}

/// `POST /api/teams/:id/resume`
pub async fn resume_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    team::coordinator::resume_team(&state()?.session_db, &team_id).await?;
    Ok(Json(json!({ "status": "resumed" })))
}

/// `POST /api/teams/:id/dissolve`
pub async fn dissolve_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    team::coordinator::dissolve_team(&state()?.session_db, &team_id)?;
    Ok(Json(json!({ "status": "dissolved" })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamBody {
    pub name: String,
    pub description: Option<String>,
    pub session_id: String,
    pub agent_id: String,
    pub members: Vec<team::CreateTeamMemberSpec>,
    pub template: Option<String>,
}

/// `POST /api/teams`
pub async fn create_team(Json(body): Json<CreateTeamBody>) -> Result<Json<team::Team>, AppError> {
    let s = state()?;
    let member_specs = if !body.members.is_empty() {
        body.members
    } else if let Some(ref tpl_name) = body.template {
        let templates = team::templates::all_templates(&s.session_db);
        let tpl = templates
            .iter()
            .find(|t| t.template_id == *tpl_name || t.name.eq_ignore_ascii_case(tpl_name))
            .ok_or_else(|| AppError::bad_request(format!("Template '{}' not found", tpl_name)))?;
        tpl.members
            .iter()
            .map(|m| team::CreateTeamMemberSpec {
                name: m.name.clone(),
                agent_id: m.agent_id.clone(),
                role: Some(m.role.as_str().to_string()),
                task: m.description.clone(),
                model: None,
            })
            .collect()
    } else {
        return Err(AppError::bad_request("Either 'members' or 'template' required"));
    };

    let created = team::coordinator::create_team(
        &s.session_db,
        &body.name,
        body.description.as_deref(),
        &body.session_id,
        &body.agent_id,
        &member_specs,
        body.template.as_deref(),
        None,
    )
    .await?;

    Ok(Json(created))
}
