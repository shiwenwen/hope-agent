pub mod handlers;
pub mod parser;
pub mod registry;
pub mod types;

use crate::AppState;
use tauri::State;
use types::{CommandCategory, CommandResult, SlashCommandDef};

/// List all available slash commands (for UI menu rendering).
/// Includes both built-in commands and user-invocable skill commands.
#[tauri::command]
pub async fn list_slash_commands(
    state: State<'_, AppState>,
) -> Result<Vec<SlashCommandDef>, String> {
    let mut commands = registry::all_commands();

    // Append user-invocable skills as Skill category commands
    let store = state.provider_store.lock().await;
    let skill_entries =
        crate::skills::get_invocable_skills(&store.extra_skills_dirs, &store.disabled_skills);
    drop(store);

    // Collect existing command names to avoid collisions
    let mut used_names: std::collections::HashSet<String> =
        commands.iter().map(|c| c.name.clone()).collect();

    for skill in skill_entries {
        let mut cmd_name = crate::skills::normalize_skill_command_name(&skill.name);

        // Dedup: add suffix if collision with built-in or other skill
        if used_names.contains(&cmd_name) {
            cmd_name = format!("{}_skill", cmd_name);
        }
        let mut counter = 2;
        let base = cmd_name.clone();
        while used_names.contains(&cmd_name) {
            cmd_name = format!("{}_{}", base, counter);
            counter += 1;
        }
        used_names.insert(cmd_name.clone());

        commands.push(SlashCommandDef {
            name: cmd_name,
            category: CommandCategory::Skill,
            description_key: String::new(), // No i18n key — use raw description
            has_args: true,
            arg_placeholder: Some("[args]".into()),
            arg_options: None,
            // Carry the raw description for frontend display
            description_raw: Some(skill.description.clone()),
        });
    }

    Ok(commands)
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

    // Allow both built-in commands and dynamic skill commands
    // (skill commands are handled in handlers::dispatch fallback)

    app_info!(
        "slash_cmd",
        "dispatch",
        "Executing /{} args={:?}",
        name,
        args
    );

    let result = handlers::dispatch(&state, session_id.as_deref(), &agent_id, &name, &args).await?;

    app_info!(
        "slash_cmd",
        "dispatch",
        "/{} completed: action={:?}",
        name,
        result
            .action
            .as_ref()
            .map(|a| format!("{:?}", a).chars().take(50).collect::<String>())
    );

    Ok(result)
}

/// Quick check if text is a slash command.
#[tauri::command]
pub fn is_slash_command(text: String) -> bool {
    parser::is_command(&text)
}
