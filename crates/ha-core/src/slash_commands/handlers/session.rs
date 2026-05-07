use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult, SessionPickerItem};
use std::sync::Arc;

/// Maximum sessions surfaced in the `/sessions` picker. IM platforms cap
/// inline-button payloads, so we keep the list small enough to render as
/// buttons without truncation.
const SESSION_PICKER_LIMIT: usize = 30;

/// /new — Create a new session, returning a markdown receipt with the agent
/// name, project (if any), and effective working directory.
pub fn handle_new(session_db: &Arc<SessionDB>, agent_id: &str) -> Result<CommandResult, String> {
    let meta = session_db
        .create_session(agent_id)
        .map_err(|e| e.to_string())?;

    let working_dir = crate::session::effective_session_working_dir(Some(&meta.id));

    let mut lines = vec![format!("✅ New session — agent **{}**", agent_id)];
    if let Some(pid) = meta.project_id.as_deref() {
        if let Some(project) = crate::globals::get_project_db()
            .and_then(|db| db.get(pid).ok().flatten())
        {
            lines.push(format!("- Project: **{}**", project.name));
        }
    }
    if let Some(wd) = working_dir.as_deref() {
        lines.push(format!("- Working dir: `{}`", wd));
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::NewSession {
            session_id: meta.id,
        }),
    })
}

/// /clear — Delete current session messages.
pub fn handle_clear(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to clear")?;
    session_db.delete_session(sid).map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: "Session cleared.".into(),
        action: Some(CommandAction::SessionCleared),
    })
}

/// /stop — Signal to stop current streaming.
pub fn handle_stop() -> CommandResult {
    CommandResult {
        content: "Stopping current reply...".into(),
        action: Some(CommandAction::StopStream),
    }
}

/// /rename <title> — Rename current session.
pub fn handle_rename(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to rename")?;
    let title = args.trim();
    if title.is_empty() {
        return Err("Usage: /rename <title>".into());
    }
    session_db
        .update_session_title(sid, title)
        .map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!("Session renamed to **{}**", title),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /sessions — picker of user-conversation sessions, filtering out cron-driven,
/// subagent-child, and incognito sessions (see `SessionMeta.is_regular_chat`
/// for the policy rationale).
pub fn handle_sessions(session_db: &Arc<SessionDB>) -> Result<CommandResult, String> {
    let all = session_db.list_sessions(None).map_err(|e| e.to_string())?;
    // `list_sessions` already excludes incognito. Filter out cron + subagent
    // children — channel-bound sessions stay in the list (they're real user
    // conversations, just surfaced from IM).
    let candidates: Vec<crate::session::SessionMeta> = all
        .into_iter()
        .filter(|s| !s.is_cron && s.parent_session_id.is_none())
        .take(SESSION_PICKER_LIMIT)
        .collect();

    let picker_items: Vec<SessionPickerItem> = candidates
        .iter()
        .map(|s| SessionPickerItem {
            id: s.id.clone(),
            title: s
                .title
                .clone()
                .unwrap_or_else(|| "(untitled)".to_string()),
            agent_id: s.agent_id.clone(),
            project_id: s.project_id.clone(),
            channel_label: s.channel_info.as_ref().map(|c| {
                let chat = c
                    .sender_name
                    .clone()
                    .unwrap_or_else(|| c.chat_id.clone());
                format!("{} · {}", c.channel_id, chat)
            }),
            updated_at: s.updated_at.clone(),
        })
        .collect();

    let content = if picker_items.is_empty() {
        "No active sessions.".to_string()
    } else {
        let mut lines = vec![format!("**Sessions** ({})", picker_items.len())];
        for s in picker_items.iter().take(10) {
            let id_short: String = s.id.chars().take(8).collect();
            let chip = s
                .channel_label
                .as_deref()
                .map(|c| format!(" · _{}_", c))
                .unwrap_or_default();
            lines.push(format!("- `{}` · {}{}", id_short, s.title, chip));
        }
        if picker_items.len() > 10 {
            lines.push(format!("…and {} more", picker_items.len() - 10));
        }
        lines.join("\n")
    };

    Ok(CommandResult {
        content,
        action: Some(CommandAction::ShowSessionPicker {
            sessions: picker_items,
        }),
    })
}

/// /session — show / attach / exit. Sub-actions:
/// - **(no args)** — `info` view of the current session (agent / project /
///   working dir / attached IM chats / primary marker).
/// - **`exit`** — detach the current IM chat from its session.
/// - **`<id>` (any other arg)** — attach the current chat to that session.
pub fn handle_session(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return handle_session_info(session_db, session_id);
    }
    if trimmed.eq_ignore_ascii_case("exit") {
        return Ok(CommandResult {
            content: "Detaching this chat from its session...".into(),
            action: Some(CommandAction::DetachFromSession),
        });
    }

    // Treat the remaining argument as a session id. Validate the id exists
    // before emitting the action so a typo gets caught at the slash layer
    // rather than blowing up inside `attach_session`.
    let target_id = trimmed.to_string();
    let exists = session_db
        .get_session(&target_id)
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("Session `{}` not found", target_id));
    }
    Ok(CommandResult {
        content: format!("Attaching to session `{}`...", target_id),
        action: Some(CommandAction::AttachToSession {
            session_id: target_id,
        }),
    })
}

fn handle_session_info(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session")?;
    let meta = session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session `{}` not found", sid))?;

    let mut lines = vec![
        format!(
            "**Session** `{}`",
            meta.id.chars().take(8).collect::<String>()
        ),
        format!(
            "- Title: {}",
            meta.title.as_deref().unwrap_or("(untitled)")
        ),
        format!("- Agent: `{}`", meta.agent_id),
    ];
    if let Some(pid) = meta.project_id.as_deref() {
        if let Some(project) = crate::globals::get_project_db()
            .and_then(|db| db.get(pid).ok().flatten())
        {
            lines.push(format!("- Project: **{}**", project.name));
        }
    }
    if let Some(wd) = crate::session::effective_session_working_dir(Some(sid)).as_deref() {
        lines.push(format!("- Working dir: `{}`", wd));
    }

    if let Some(channel_db) = crate::globals::get_channel_db() {
        if let Ok(attaches) = channel_db.list_attached(sid) {
            if !attaches.is_empty() {
                lines.push(String::new());
                lines.push("**Attached IM channels**".into());
                for a in attaches.iter() {
                    let star = if a.is_primary { "★ " } else { "" };
                    let label = a.sender_name.as_deref().unwrap_or(&a.chat_id);
                    lines.push(format!(
                        "- {}{} · {} ({})",
                        star, a.channel_id, label, a.chat_type
                    ));
                }
            }
        }
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /handover — push the current session to an IM chat. Args expect the
/// shape `<channel_id>:<account_id>:<chat_id>[:<thread_id>]`. With no args,
/// we hint to the user to use the GUI Handover dialog (the slash form is
/// for power users / scripting). Always GUI-only — IM-side handovers go
/// through `/session <id>` from the target chat instead.
pub fn handle_handover(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let sid = session_id.ok_or("No active session to hand over")?;
    let _meta = session_db
        .get_session(sid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session `{}` not found", sid))?;

    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Err(
            "Usage: /handover <channelId>:<accountId>:<chatId>[:<threadId>] (or use the Handover button in the chat header)".into(),
        );
    }

    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() < 3 || parts.len() > 4 {
        return Err(
            "Usage: /handover <channelId>:<accountId>:<chatId>[:<threadId>]".into(),
        );
    }
    let channel_id = parts[0].trim();
    let account_id = parts[1].trim();
    let chat_id = parts[2].trim();
    let thread_id = parts.get(3).map(|s| s.trim().to_string());
    if channel_id.is_empty() || account_id.is_empty() || chat_id.is_empty() {
        return Err("Channel id / account id / chat id may not be empty".into());
    }

    Ok(CommandResult {
        content: format!(
            "Handing session over to `{}` / `{}` / `{}`...",
            channel_id, account_id, chat_id
        ),
        action: Some(CommandAction::HandoverToChannel {
            session_id: sid.to_string(),
            channel_id: channel_id.to_string(),
            account_id: account_id.to_string(),
            chat_id: chat_id.to_string(),
            thread_id,
        }),
    })
}

