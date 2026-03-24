pub mod session;
pub mod model;
pub mod memory;
pub mod agent;
pub mod utility;

use crate::AppState;
use crate::get_memory_backend;
use crate::slash_commands::types::CommandResult;

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

        // ── Utility ──
        "help" => Ok(utility::handle_help()),
        "status" => {
            let store = state.provider_store.lock().await;
            utility::handle_status(&state.session_db, &store, session_id, agent_id)
        }
        "export" => utility::handle_export(&state.session_db, session_id),
        "usage" => utility::handle_usage(&state.session_db, session_id),
        "search" => utility::handle_search(args),

        _ => Err(format!("Unknown command: /{}", command)),
    }
}
