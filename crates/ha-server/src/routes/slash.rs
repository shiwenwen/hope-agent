use axum::Json;
use serde::Deserialize;

use ha_core::slash_commands;

use crate::error::AppError;

/// `GET /api/slash-commands`
pub async fn list_slash_commands(
) -> Result<Json<Vec<slash_commands::types::SlashCommandDef>>, AppError> {
    slash_commands::list_slash_commands()
        .await
        .map(Json)
        .map_err(AppError::internal)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteBody {
    pub session_id: Option<String>,
    pub agent_id: String,
    pub command_text: String,
}

/// `POST /api/slash-commands/execute`
pub async fn execute_slash_command(
    Json(body): Json<ExecuteBody>,
) -> Result<Json<slash_commands::types::CommandResult>, AppError> {
    slash_commands::execute_slash_command(body.session_id, body.agent_id, body.command_text)
        .await
        .map(Json)
        .map_err(AppError::internal)
}

#[derive(Debug, Deserialize)]
pub struct IsSlashCommandBody {
    pub text: String,
}

/// `POST /api/slash-commands/is-slash`
pub async fn is_slash_command(
    Json(body): Json<IsSlashCommandBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(serde_json::json!({
        "is_slash": slash_commands::is_slash_command(body.text),
    })))
}
