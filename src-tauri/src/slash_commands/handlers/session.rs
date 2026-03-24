use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};
use std::sync::Arc;

/// /new — Create a new session.
pub fn handle_new(session_db: &Arc<SessionDB>, agent_id: &str) -> Result<CommandResult, String> {
    let meta = session_db
        .create_session(agent_id)
        .map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!("✅ New session started."),
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
    session_db
        .delete_session(sid)
        .map_err(|e| e.to_string())?;
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
