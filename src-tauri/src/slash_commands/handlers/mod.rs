pub mod agent;
pub mod memory;
pub mod model;
pub mod plan;
pub mod session;
pub mod utility;

use crate::get_memory_backend;
use crate::slash_commands::types::CommandResult;
use crate::AppState;

/// Dispatch a parsed command to the appropriate handler.
pub async fn dispatch(
    state: &AppState,
    session_id: Option<&str>,
    agent_id: &str,
    command: &str,
    args: &str,
) -> Result<CommandResult, String> {
    match command {
        // ── Session ──
        "new" => session::handle_new(&state.session_db, agent_id),
        "clear" => session::handle_clear(&state.session_db, session_id),
        "stop" => Ok(session::handle_stop()),
        "rename" => session::handle_rename(&state.session_db, session_id, args),
        "compact" => {
            // Return Compact action — frontend delegates to existing compact_context_now
            Ok(CommandResult {
                content: "Compacting context...".into(),
                action: Some(crate::slash_commands::types::CommandAction::Compact),
            })
        }

        // ── Model ──
        "model" => {
            let store = state.provider_store.lock().await;
            model::handle_model(&store, args)
        }
        "models" => {
            let store = state.provider_store.lock().await;
            model::handle_model(&store, "")
        }
        "think" => model::handle_think(args),

        // ── Memory ──
        "remember" => {
            let backend = get_memory_backend().ok_or("Memory backend not initialized")?;
            memory::handle_remember(backend, args, session_id)
        }
        "forget" => {
            let backend = get_memory_backend().ok_or("Memory backend not initialized")?;
            memory::handle_forget(backend, args)
        }
        "memories" => {
            let backend = get_memory_backend().ok_or("Memory backend not initialized")?;
            memory::handle_memories(backend)
        }

        // ── Agent ──
        "agent" => agent::handle_agent(&state.session_db, args),
        "agents" => agent::handle_agents(),

        // ── Plan ──
        "plan" => plan::handle_plan(&state.session_db, session_id, args).await,

        // ── Utility ──
        "permission" => utility::handle_permission(args),
        "help" => Ok(utility::handle_help()),
        "status" => {
            let store = state.provider_store.lock().await;
            utility::handle_status(&state.session_db, &store, session_id, agent_id)
        }
        "export" => utility::handle_export(&state.session_db, session_id),
        "usage" => utility::handle_usage(&state.session_db, session_id),
        "search" => utility::handle_search(args),
        "prompts" => Ok(utility::handle_prompts()),

        _ => {
            // Check if it matches a user-invocable skill command
            if let Some(result) = handle_skill_command(state, command, args).await {
                result
            } else {
                Err(format!("Unknown command: /{}", command))
            }
        }
    }
}

/// Try to handle a command as a skill slash command.
/// Returns None if no matching skill found.
async fn handle_skill_command(
    state: &AppState,
    command: &str,
    args: &str,
) -> Option<Result<CommandResult, String>> {
    let store = state.provider_store.lock().await;
    let skills =
        crate::skills::get_invocable_skills(&store.extra_skills_dirs, &store.disabled_skills);
    drop(store);

    // Find a skill whose normalized name matches the command
    let matched = skills
        .into_iter()
        .find(|s| crate::skills::normalize_skill_command_name(&s.name) == command)?;

    // If skill has command_dispatch == "tool" with a command_tool, dispatch to that tool
    if matched.command_dispatch.as_deref() == Some("tool") {
        if let Some(tool_name) = &matched.command_tool {
            let message = if args.is_empty() {
                format!("Use the {} tool for skill '{}'.", tool_name, matched.name)
            } else {
                format!(
                    "Use the {} tool for skill '{}' with: {}",
                    tool_name, matched.name, args
                )
            };
            return Some(Ok(CommandResult {
                content: format!("Dispatching to tool `{}`...", tool_name),
                action: Some(crate::slash_commands::types::CommandAction::PassThrough { message }),
            }));
        }
    }

    // Default: pass through to LLM with skill context
    let message = if args.is_empty() {
        format!(
            "Use the skill '{}'. Read the skill file at {} for instructions.",
            matched.name, matched.file_path
        )
    } else {
        format!(
            "Use the skill '{}' to: {}. Read the skill file at {} for instructions.",
            matched.name, args, matched.file_path
        )
    };

    Some(Ok(CommandResult {
        content: format!("Invoking skill **{}**...", matched.name),
        action: Some(crate::slash_commands::types::CommandAction::PassThrough { message }),
    }))
}
