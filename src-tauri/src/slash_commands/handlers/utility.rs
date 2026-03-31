use crate::provider::{self, ProviderStore};
use crate::session::{MessageRole, SessionDB};
use crate::slash_commands::registry;
use crate::slash_commands::types::{CommandAction, CommandResult};
use std::sync::Arc;

/// /help — Show all available commands.
pub fn handle_help() -> CommandResult {
    let commands = registry::all_commands();
    let mut lines = vec!["**Available Commands**\n".to_string()];

    let categories = [
        ("Session", "session"),
        ("Model", "model"),
        ("Memory", "memory"),
        ("Agent", "agent"),
        ("Utility", "utility"),
    ];

    for (label, cat_str) in &categories {
        let cmds: Vec<_> = commands
            .iter()
            .filter(|c| format!("{:?}", c.category).to_lowercase() == *cat_str)
            .collect();
        if cmds.is_empty() {
            continue;
        }
        lines.push(format!("\n**{}**", label));
        for c in cmds {
            let arg_hint = c
                .arg_placeholder
                .as_deref()
                .map(|p| format!(" {}", p))
                .unwrap_or_default();
            // Use the command name as description since we can't resolve i18n keys server-side
            lines.push(format!("- `/{}{}`", c.name, arg_hint));
        }
    }

    CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    }
}

/// /status — Show session status.
pub fn handle_status(
    session_db: &Arc<SessionDB>,
    store: &ProviderStore,
    session_id: Option<&str>,
    agent_id: &str,
) -> Result<CommandResult, String> {
    let mut lines = vec!["**Session Status**\n".to_string()];

    // Agent info
    lines.push(format!("- **Agent**: `{}`", agent_id));

    // Model info
    if let Some(ref active) = store.active_model {
        let models = provider::build_available_models(&store.providers);
        let name = models
            .iter()
            .find(|m| m.provider_id == active.provider_id && m.model_id == active.model_id)
            .map(|m| format!("{} / {}", m.provider_name, m.model_name))
            .unwrap_or_else(|| format!("{} / {}", active.provider_id, active.model_id));
        lines.push(format!("- **Model**: {}", name));
    } else {
        lines.push("- **Model**: not set".into());
    }

    // Session info
    if let Some(sid) = session_id {
        lines.push(format!("- **Session ID**: `{}`", sid));
        if let Ok(messages) = session_db.load_session_messages(sid) {
            let user_count = messages
                .iter()
                .filter(|m| m.role == MessageRole::User)
                .count();
            let assistant_count = messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .count();
            lines.push(format!(
                "- **Messages**: {} user, {} assistant",
                user_count, assistant_count
            ));
        }
    } else {
        lines.push("- **Session**: none (new chat)".into());
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /export — Export conversation as Markdown.
pub fn handle_export(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to export")?;
    let messages = session_db
        .load_session_messages(sid)
        .map_err(|e| e.to_string())?;

    if messages.is_empty() {
        return Err("No messages to export".into());
    }

    let session_meta = session_db.get_session(sid).map_err(|e| e.to_string())?;
    let title = session_meta
        .and_then(|m| m.title)
        .unwrap_or_else(|| "Untitled".to_string());

    let mut md = format!("# {}\n\n", title);
    for msg in &messages {
        match msg.role {
            MessageRole::User => {
                md.push_str(&format!("## User\n\n{}\n\n", msg.content));
            }
            MessageRole::Assistant => {
                md.push_str(&format!("## Assistant\n\n{}\n\n", msg.content));
            }
            _ => {}
        }
    }

    let filename = format!("{}.md", sanitize_filename(&title));

    Ok(CommandResult {
        content: format!("Exported {} messages.", messages.len()),
        action: Some(CommandAction::ExportFile {
            content: md,
            filename,
        }),
    })
}

/// /usage — Show token usage for current session.
pub fn handle_usage(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;
    let messages = session_db
        .load_session_messages(sid)
        .map_err(|e| e.to_string())?;

    let mut total_in: i64 = 0;
    let mut total_out: i64 = 0;
    let mut turns = 0;

    for msg in &messages {
        if msg.role == MessageRole::Assistant {
            turns += 1;
            total_in += msg.tokens_in.unwrap_or(0);
            total_out += msg.tokens_out.unwrap_or(0);
        }
    }

    let content = format!(
        "**Token Usage**\n\n- **Input tokens**: {}\n- **Output tokens**: {}\n- **Total**: {}\n- **Turns**: {}",
        total_in,
        total_out,
        total_in + total_out,
        turns,
    );

    Ok(CommandResult {
        content,
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /permission <mode> — Set tool permission mode for current session.
pub fn handle_permission(args: &str) -> Result<CommandResult, String> {
    let mode = args.trim().to_lowercase();
    let (resolved, label) = match mode.as_str() {
        "auto" => ("auto", "Auto"),
        "ask" | "ask_every_time" => ("ask_every_time", "Ask Every Time"),
        "full" | "full_approve" => ("full_approve", "Full Approve"),
        _ => {
            return Err(format!(
                "Invalid permission mode: `{}`. Valid: auto, ask, full",
                mode
            ));
        }
    };
    Ok(CommandResult {
        content: format!("Tool permission set to **{}**.", label),
        action: Some(CommandAction::SetToolPermission {
            mode: resolved.to_string(),
        }),
    })
}

/// /search <query> — Pass through to LLM as a search request.
pub fn handle_search(args: &str) -> Result<CommandResult, String> {
    let query = args.trim();
    if query.is_empty() {
        return Err("Usage: /search <query>".into());
    }
    Ok(CommandResult {
        content: String::new(),
        action: Some(CommandAction::PassThrough {
            message: format!("Please search the web for: {}", query),
        }),
    })
}

/// Simple filename sanitization.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}
