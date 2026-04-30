use crate::config::AppConfig;
use crate::provider;
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
    store: &AppConfig,
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
        if let Ok((user_count, assistant_count)) = session_db.count_user_assistant_messages(sid) {
            lines.push(format!(
                "- **Messages**: {} user, {} assistant",
                user_count, assistant_count
            ));
        }
        let mode = session_db
            .get_session_permission_mode(sid)
            .ok()
            .flatten()
            .unwrap_or(crate::permission::SessionMode::Default);
        lines.push(format!("- **Permission Mode**: `{}`", mode.as_str()));
        if let Some(project_lines) = render_project_section(session_db, sid) {
            lines.push(String::new());
            lines.extend(project_lines);
        }
    } else {
        lines.push("- **Session**: none (new chat)".into());
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}

fn render_project_section(session_db: &Arc<SessionDB>, sid: &str) -> Option<Vec<String>> {
    let meta = session_db.get_session(sid).ok().flatten()?;
    let project_id = meta.project_id.as_deref()?;
    let project_db = crate::require_project_db().ok()?;
    let project = project_db.get(project_id).ok().flatten()?;

    let mut lines = vec![
        "**Current Project**".to_string(),
        format!("- **Name**: {}", project.name),
    ];
    if let Some(desc) = project
        .description
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!("- **Description**: {}", truncate(desc, 200)));
    }
    if let Some(default_agent) = project.default_agent_id.as_deref() {
        lines.push(format!("- **Default Agent**: `{}`", default_agent));
    }
    if let Some(working_dir) = project.working_dir.as_deref() {
        lines.push(format!("- **Working Directory**: `{}`", working_dir));
    }
    if let Some(bound) = project.bound_channel.as_ref() {
        lines.push(format!(
            "- **Bound IM Channel**: `{}` / `{}`",
            bound.channel_id, bound.account_id
        ));
    }
    if let Some(instructions) = project
        .instructions
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        lines.push(format!(
            "- **Instructions**: {}",
            truncate(instructions, 200)
        ));
    }

    let cfg = crate::config::cached_config();
    let channel_account = meta
        .channel_info
        .as_ref()
        .and_then(|ci| cfg.channels.find_account(&ci.account_id))
        .cloned();
    let (_, source) = crate::agent::resolver::resolve_default_agent_id_with_source(
        Some(&project),
        channel_account.as_ref(),
    );
    lines.push(format!("- **Agent Source**: {}", source.label()));
    Some(lines)
}

/// Char-bounded truncate with ellipsis suffix. Used for status / display.
fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
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

/// /permission <default|smart|yolo> — Switch the session permission mode.
/// Use `/status` to view the current mode.
pub fn handle_permission(args: &str) -> Result<CommandResult, String> {
    let mode_arg = args.trim().to_lowercase();
    let resolved = match mode_arg.as_str() {
        "default" => crate::permission::SessionMode::Default,
        "smart" => crate::permission::SessionMode::Smart,
        "yolo" => crate::permission::SessionMode::Yolo,
        _ => {
            return Err(format!(
                "Invalid permission mode: `{}`. Valid: default, smart, yolo",
                mode_arg
            ));
        }
    };

    Ok(CommandResult {
        content: format!("Permission mode set to **{}**.", resolved.as_str()),
        action: Some(CommandAction::SetToolPermission {
            mode: resolved.as_str().to_string(),
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

/// /prompts — Open the system prompt viewer.
pub fn handle_prompts() -> CommandResult {
    CommandResult {
        content: String::new(),
        action: Some(CommandAction::ViewSystemPrompt),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_modes_emit_set_action() {
        for (input, expected) in [
            ("default", "default"),
            ("smart", "smart"),
            ("yolo", "yolo"),
            // case-insensitive — handler lowercases args
            ("YOLO", "yolo"),
            ("  smart  ", "smart"),
        ] {
            let res = handle_permission(input).expect("ok");
            match res.action {
                Some(CommandAction::SetToolPermission { ref mode }) => {
                    assert_eq!(mode, expected, "input {:?}", input);
                }
                other => panic!("unexpected action for {:?}: {:?}", input, other),
            }
            assert!(res.content.contains(&format!("**{}**", expected)));
        }
    }

    #[test]
    fn rejects_legacy_and_unknown_aliases() {
        for bad in [
            "auto",
            "ask",
            "full",
            "ask_every_time",
            "full_approve",
            "garbage",
            "",
        ] {
            let err = handle_permission(bad).expect_err("should error");
            assert!(
                err.contains("Invalid permission mode") && err.contains("default, smart, yolo"),
                "input {:?}, got {:?}",
                bad,
                err
            );
        }
    }
}
