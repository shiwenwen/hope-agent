use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use ha_core::team;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTeamsQuery {
    pub session_id: Option<String>,
}

/// `GET /api/teams`
pub async fn list_teams(
    Query(q): Query<ListTeamsQuery>,
) -> Result<Json<Vec<team::Team>>, AppError> {
    let db = session_db()?;
    if let Some(sid) = q.session_id {
        Ok(Json(db.list_teams_by_session(&sid)?))
    } else {
        Ok(Json(db.list_active_teams()?))
    }
}

/// `GET /api/teams/:id`
pub async fn get_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    Ok(Json(serde_json::to_value(
        session_db()?.get_team(&team_id)?,
    )?))
}

/// `GET /api/teams/:id/members`
pub async fn get_team_members(
    Path(team_id): Path<String>,
) -> Result<Json<Vec<team::TeamMember>>, AppError> {
    Ok(Json(session_db()?.list_team_members(&team_id)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesQuery {
    pub limit: Option<u32>,
}

/// `GET /api/teams/:id/messages?limit=N` — load latest team messages.
///
/// Returns JSON tuple `[messages, hasMore]` (same shape as Tauri
/// `get_team_messages`). Default limit is 50.
pub async fn get_team_messages(
    Path(team_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<Value>, AppError> {
    let (messages, has_more) =
        session_db()?.list_team_messages_latest(&team_id, q.limit.unwrap_or(50))?;
    Ok(Json(json!([messages, has_more])))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesBeforeQuery {
    pub before_timestamp: String,
    pub before_message_id: String,
    pub limit: Option<u32>,
}

/// `GET /api/teams/:id/messages/before?beforeTimestamp=...&beforeMessageId=...&limit=N`
///
/// Load team messages strictly older than the composite cursor, in ASC order.
/// Returns JSON tuple `[messages, hasMore]`.
pub async fn get_team_messages_before(
    Path(team_id): Path<String>,
    Query(q): Query<MessagesBeforeQuery>,
) -> Result<Json<Value>, AppError> {
    let (messages, has_more) = session_db()?.list_team_messages_before(
        &team_id,
        &q.before_timestamp,
        &q.before_message_id,
        q.limit.unwrap_or(50),
    )?;
    Ok(Json(json!([messages, has_more])))
}

/// `GET /api/teams/:id/tasks`
pub async fn get_team_tasks(
    Path(team_id): Path<String>,
) -> Result<Json<Vec<team::TeamTask>>, AppError> {
    Ok(Json(session_db()?.list_team_tasks(&team_id)?))
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
    let msg = team::messaging::send_message(
        session_db()?,
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
    Ok(Json(team::templates::all_templates(session_db()?)))
}

/// `POST /api/teams/:id/pause`
pub async fn pause_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    team::coordinator::pause_team(session_db()?, &team_id)?;
    Ok(Json(json!({ "status": "paused" })))
}

/// `POST /api/teams/:id/resume`
pub async fn resume_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    team::coordinator::resume_team(session_db()?, &team_id).await?;
    Ok(Json(json!({ "status": "resumed" })))
}

/// `POST /api/teams/:id/dissolve`
pub async fn dissolve_team(Path(team_id): Path<String>) -> Result<Json<Value>, AppError> {
    team::coordinator::dissolve_team(session_db()?, &team_id)?;
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
    let db = session_db()?;
    let team_name = body.name.clone();
    let (member_specs, resolved_template_id) = if !body.members.is_empty() {
        (body.members, body.template.clone())
    } else if let Some(ref tpl_name) = body.template {
        let templates = team::templates::all_templates(db);
        let tpl = templates
            .iter()
            .find(|t| t.template_id == *tpl_name || t.name.eq_ignore_ascii_case(tpl_name))
            .ok_or_else(|| AppError::bad_request(format!("Template '{}' not found", tpl_name)))?;
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
                        format!(
                            "Work on your role '{}' as part of team '{}'.",
                            m.name, team_name
                        )
                    }),
                model: m.model_override.clone(),
                description: Some(m.description.clone()).filter(|s| !s.trim().is_empty()),
            })
            .collect();
        (specs, Some(tpl.template_id.clone()))
    } else {
        return Err(AppError::bad_request(
            "Either 'members' or 'template' required",
        ));
    };

    let created = team::coordinator::create_team(
        db,
        &body.name,
        body.description.as_deref(),
        &body.session_id,
        &body.agent_id,
        &member_specs,
        resolved_template_id.as_deref(),
        None,
    )
    .await?;

    Ok(Json(created))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTemplateBody {
    pub template: team::TeamTemplate,
}

/// `POST /api/team-templates`
pub async fn save_team_template(
    Json(body): Json<SaveTemplateBody>,
) -> Result<Json<team::TeamTemplate>, AppError> {
    let saved = team::templates::save_template(session_db()?, body.template)?;
    Ok(Json(saved))
}

/// `DELETE /api/team-templates/:id`
pub async fn delete_team_template(
    Path(template_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    team::templates::delete_template(session_db()?, &template_id)?;
    Ok(Json(
        json!({ "status": "deleted", "templateId": template_id }),
    ))
}
