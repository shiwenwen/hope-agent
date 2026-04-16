use anyhow::Result;
use std::sync::Arc;

use crate::session::SessionDB;
use crate::subagent::SUBAGENT_MAILBOX;
use super::events::emit_team_event;
use super::types::*;

/// Send a message from one team member to another (or broadcast).
pub fn send_message(
    db: &Arc<SessionDB>,
    team_id: &str,
    from_member_id: &str,
    to: Option<&str>, // None or "*" = broadcast
    content: &str,
    message_type: TeamMessageType,
) -> Result<TeamMessage> {
    let msg = TeamMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        team_id: team_id.to_string(),
        from_member_id: from_member_id.to_string(),
        to_member_id: to.and_then(|t| if t == "*" { None } else { Some(t.to_string()) }),
        content: content.to_string(),
        message_type,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    // Persist
    db.insert_team_message(&msg)?;

    // Deliver to active member(s) via SUBAGENT_MAILBOX
    let members = db.list_team_members(team_id)?;
    let sender_name = members
        .iter()
        .find(|m| m.member_id == from_member_id || m.name == from_member_id)
        .map(|m| m.name.as_str())
        .unwrap_or(from_member_id);
    let formatted = format!("[Team msg from {}]: {}", sender_name, content);

    match &msg.to_member_id {
        Some(target_id) => {
            // Direct message — find the target member
            if let Some(target) = members.iter().find(|m| m.member_id == *target_id || m.name == *target_id) {
                if target.status.is_active() {
                    if let Some(ref run_id) = target.run_id {
                        SUBAGENT_MAILBOX.push(run_id, formatted);
                    }
                }
            }
        }
        None => {
            // Broadcast — send to all active members except sender
            for member in &members {
                if member.member_id == from_member_id {
                    continue;
                }
                if member.status.is_active() {
                    if let Some(ref run_id) = member.run_id {
                        SUBAGENT_MAILBOX.push(run_id, formatted.clone());
                    }
                }
            }
        }
    }

    // Emit event for frontend
    emit_team_event("message", &msg);

    Ok(msg)
}

/// Post a system message (e.g., "Task #2 completed by Backend").
pub fn post_system_message(
    db: &Arc<SessionDB>,
    team_id: &str,
    content: &str,
) -> Result<TeamMessage> {
    send_message(
        db,
        team_id,
        "*system*",
        None, // broadcast
        content,
        TeamMessageType::System,
    )
}
