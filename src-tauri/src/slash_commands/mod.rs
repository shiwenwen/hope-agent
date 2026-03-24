pub mod types;
pub mod registry;
pub mod parser;
pub mod handlers;

use types::{CommandResult, SlashCommandDef};
use tauri::State;
use crate::AppState;

/// List all available slash commands (for UI menu rendering).
#[tauri::command]
pub fn list_slash_commands() -> Vec<SlashCommandDef> {
    registry::all_commands()
}

/// Execute a slash command.
///
/// - `session_id`: Current session ID (None if no active session)
/// - `agent_id`: Current agent ID
/// - `command_text`: Full text including "/" prefix, e.g. "/model gpt-4o"
#[tauri::command]
pub async fn execute_slash_command(
    state: State<'_, AppState>,
    session_id: Option<String>,
    agent_id: String,
    command_text: String,
) -> Result<CommandResult, String> {
    let (name, args) = parser::parse(&command_text)?;

    if !registry::is_valid_command(&name) {
        return Err(format!("Unknown command: /{}", name));
    }

    app_info!("slash_cmd", "dispatch", "Executing /{} args={:?}", name, args);

    let result = handlers::dispatch(
        &state,
        session_id.as_deref(),
        &agent_id,
        &name,
        &args,
    )
    .await?;

    app_info!("slash_cmd", "dispatch", "/{} completed: action={:?}", name,
        result.action.as_ref().map(|a| format!("{:?}", a).chars().take(50).collect::<String>()));

    Ok(result)
}

/// Quick check if text is a slash command.
#[tauri::command]
pub fn is_slash_command(text: String) -> bool {
    parser::is_command(&text)
}
