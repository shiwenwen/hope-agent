use crate::config::AppConfig;
use crate::provider;
use crate::session::{MessageRole, SessionDB};
use crate::slash_commands::registry;
use crate::slash_commands::types::{
    CommandAction, CommandCategory, CommandResult, SlashCommandDef,
};
use std::sync::Arc;

/// /help — Show all available commands.
///
/// Renders one section per `CommandCategory`. Each row shows the command,
/// its argument placeholder (or the fixed option list), and the English
/// description from `description_en()`. When invoked from an IM channel
/// session, commands listed in `IM_DISABLED_COMMANDS` are filtered out and
/// a footer call-out explains the desktop-only ones.
pub async fn handle_help(session_id: Option<&str>) -> CommandResult {
    // Detect IM-channel context defensively — `/help` should still render
    // when SessionDB hasn't been initialized yet (e.g. very early CLI use).
    let in_im_channel = session_id
        .and_then(|sid| {
            crate::require_session_db()
                .ok()
                .and_then(|db| db.get_session(sid).ok().flatten())
        })
        .map(|meta| meta.channel_info.is_some())
        .unwrap_or(false);

    let mut commands: Vec<SlashCommandDef> = registry::all_commands();
    if in_im_channel {
        commands.retain(|c| !registry::is_im_disabled(&c.name));
    }

    // Pull invocable skills so users see the skill-as-slash-command catalog
    // alongside built-ins. Skills are dynamic per agent / extra dirs / disabled
    // list, so we hit the same surface `list_slash_commands` exposes to the UI.
    let cfg = crate::config::cached_config();
    let skills = crate::skills::get_invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
    let reserved: std::collections::HashSet<String> =
        commands.iter().map(|c| c.name.clone()).collect();
    let resolved_skills = crate::slash_commands::resolve_skill_command_names(&skills, &reserved);
    drop(cfg);

    let mut lines: Vec<String> = Vec::new();
    lines.push("**Available Commands**".to_string());
    lines.push(String::new());

    // Category order matches the GUI menu (`CATEGORY_ORDER` in
    // `slash-commands/types.ts`) so the on-screen and `/help` orderings agree.
    let categories: &[(CommandCategory, &str)] = &[
        (CommandCategory::Session, "Session"),
        (CommandCategory::Model, "Model"),
        (CommandCategory::Memory, "Memory"),
        (CommandCategory::Agent, "Agent"),
        (CommandCategory::Utility, "Utility"),
    ];

    for (cat, label) in categories {
        let cmds: Vec<&SlashCommandDef> = commands.iter().filter(|c| &c.category == cat).collect();
        if cmds.is_empty() {
            continue;
        }
        lines.push(format!("**{}**", label));
        for c in cmds {
            lines.push(format_help_row(c));
        }
        lines.push(String::new());
    }

    if !resolved_skills.is_empty() {
        lines.push(format!("**Skills** ({})", resolved_skills.len()));
        // Cap the inline list so big skill catalogs don't drown the rest of
        // the help output. Users can scroll the slash-menu UI for the full set.
        const MAX_SKILLS_INLINE: usize = 20;
        for entry in resolved_skills.iter().take(MAX_SKILLS_INLINE) {
            let desc = truncate(&entry.skill.description, 80);
            lines.push(format!("- `/{}` — {}", entry.typed_name, desc));
        }
        if resolved_skills.len() > MAX_SKILLS_INLINE {
            lines.push(format!(
                "- _… and {} more — open the slash menu to browse all_",
                resolved_skills.len() - MAX_SKILLS_INLINE
            ));
        }
        lines.push(String::new());
    }

    if in_im_channel {
        let disabled: Vec<String> = registry::IM_DISABLED_COMMANDS
            .iter()
            .map(|n| format!("`/{}`", n))
            .collect();
        lines.push(format!(
            "_IM channels can't run {} — use the desktop or web app for those._",
            disabled.join(", ")
        ));
    } else {
        lines.push("_Tip: type `/` to open the inline command menu, or click a row above to autofill arguments._".into());
    }

    CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    }
}

/// Render a single row: `` `/cmd <args>` — description``.
///
/// Uses fixed `arg_options` for the inline hint when available (e.g.
/// `/think off|low|medium|high|xhigh`), otherwise falls back to the
/// `arg_placeholder` from the registry. Description text comes from
/// `description_en()` which is the same source IM channels use for their
/// menu sync, so `/help` and the Telegram / Discord menus stay in lockstep.
fn format_help_row(c: &SlashCommandDef) -> String {
    let arg_hint = match (&c.arg_options, c.arg_placeholder.as_deref()) {
        (Some(opts), _) if !opts.is_empty() => {
            let joined = opts.join("|");
            if c.args_optional {
                format!(" [{}]", joined)
            } else {
                format!(" <{}>", joined)
            }
        }
        (_, Some(p)) => format!(" {}", p),
        _ => String::new(),
    };
    format!("- `/{}{}` — {}", c.name, arg_hint, c.description_en())
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
