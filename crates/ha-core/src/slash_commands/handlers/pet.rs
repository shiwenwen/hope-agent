use std::sync::Arc;

use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};

fn display(content: impl Into<String>) -> CommandResult {
    CommandResult {
        content: content.into(),
        action: Some(CommandAction::DisplayOnly),
    }
}

/// Control the desktop overlay without involving the model. The runtime-role
/// check is intentional: the same slash dispatcher is shared by desktop,
/// HTTP, ACP and IM entrypoints, but only the Tauri primary owns a PetWindow.
pub async fn handle_pet(
    session_db: Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    let is_im = if let Some(session_id) = session_id {
        let session_id = session_id.to_string();
        session_db
            .run(move |db| {
                db.get_session(&session_id)
                    .map(|session| session.is_some_and(|session| session.channel_info.is_some()))
                    .map_err(|error| error.to_string())
            })
            .await?
    } else {
        false
    };
    if is_im {
        return Ok(display(
            "The desktop pet is controlled from the Hope desktop app and is unavailable in IM chats.",
        ));
    }

    let command = match args.trim().to_ascii_lowercase().as_str() {
        "" | "toggle" => "toggle",
        "on" | "wake" => "on",
        "off" | "tuck" => "off",
        "status" => "status",
        _ => return Err("Usage: /pet [on|off|toggle|status]".to_string()),
    };
    let current = crate::config::cached_config().pet.clone();

    if command == "status" {
        let state = if current.enabled {
            "awake"
        } else {
            "tucked away"
        };
        let runtime = if crate::app_init::is_desktop() {
            "desktop overlay available"
        } else {
            "desktop overlay unavailable in this runtime"
        };
        return Ok(display(format!(
            "Desktop pet: **{state}** · selected `{}` · {runtime}.",
            current.selected_pet_ref.0
        )));
    }

    if !crate::app_init::is_desktop() {
        return Ok(display(
            "The pet overlay can only be woken or tucked away in the Hope desktop app. Use `/pet status` to inspect the saved setting.",
        ));
    }

    let enabled = match command {
        "on" => true,
        "off" => false,
        _ => !current.enabled,
    };
    if current.enabled != enabled {
        crate::pet::update_config(Some(enabled), None, "slash-command")
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(display(if enabled {
        "Desktop pet awakened."
    } else {
        "Desktop pet tucked away."
    }))
}
