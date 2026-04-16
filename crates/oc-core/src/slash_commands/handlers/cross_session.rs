//! `/cross-session` — toggle or inspect the cross-session behavior awareness
//! feature at the global level.
//!
//! Subcommands:
//!   /cross-session                → show current status
//!   /cross-session on|off         → flip global enabled
//!   /cross-session mode structured|llm|off
//!   /cross-session status         → detailed runtime status

use crate::cross_session::CrossSessionMode;
use crate::slash_commands::types::{CommandAction, CommandResult};

/// Handle the `/cross-session` slash command.
pub fn handle_cross_session(args: &str) -> Result<CommandResult, String> {
    let args_trim = args.trim();

    if args_trim.is_empty() {
        return Ok(status_result());
    }

    let mut parts = args_trim.split_whitespace();
    let sub = parts.next().unwrap_or("").to_lowercase();
    let arg1 = parts.next().map(|s| s.to_lowercase());

    match sub.as_str() {
        "on" | "enable" => set_enabled(true),
        "off" | "disable" => set_enabled(false),
        "mode" => match arg1.as_deref() {
            Some("off") => set_mode(CrossSessionMode::Off),
            Some("structured") => set_mode(CrossSessionMode::Structured),
            Some("llm") | Some("llm_digest") | Some("digest") => {
                set_mode(CrossSessionMode::LlmDigest)
            }
            _ => Err("Usage: /cross-session mode [off|structured|llm|llm_digest|digest]".to_string()),
        },
        "status" => Ok(status_result()),
        other => Err(format!(
            "Unknown subcommand '{}'. Try: /cross-session [on|off|mode <x>|status]",
            other
        )),
    }
}

fn set_enabled(enabled: bool) -> Result<CommandResult, String> {
    let mut store = crate::config::load_config().map_err(|e| e.to_string())?;
    store.cross_session.enabled = enabled;
    crate::config::save_config(&store).map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!(
            "Cross-session awareness {}.",
            if enabled { "enabled" } else { "disabled" }
        ),
        action: Some(CommandAction::DisplayOnly),
    })
}

fn set_mode(mode: CrossSessionMode) -> Result<CommandResult, String> {
    let mut store = crate::config::load_config().map_err(|e| e.to_string())?;
    store.cross_session.mode = mode;
    // Enabling a concrete mode implies the feature is on.
    if !matches!(mode, CrossSessionMode::Off) {
        store.cross_session.enabled = true;
    }
    crate::config::save_config(&store).map_err(|e| e.to_string())?;
    let label = match mode {
        CrossSessionMode::Off => "off",
        CrossSessionMode::Structured => "structured (zero LLM cost)",
        CrossSessionMode::LlmDigest => "llm_digest (extra side_query per turn)",
    };
    Ok(CommandResult {
        content: format!("Cross-session mode set to **{}**.", label),
        action: Some(CommandAction::DisplayOnly),
    })
}

fn status_result() -> CommandResult {
    let cfg = crate::config::cached_config().cross_session.clone();
    let mode_label = match cfg.mode {
        CrossSessionMode::Off => "off",
        CrossSessionMode::Structured => "structured",
        CrossSessionMode::LlmDigest => "llm_digest",
    };
    let active = crate::cross_session::active_snapshot().len();
    let content = format!(
        "**Cross-Session Awareness**\n\n\
         - Enabled: `{}`\n\
         - Mode: `{}`\n\
         - Max sessions: `{}`\n\
         - Lookback: `{}h`\n\
         - Min refresh: `{}s`\n\
         - Include cron: `{}`\n\
         - Include channel: `{}`\n\
         - Include subagents: `{}`\n\
         - Active peers right now: `{}`",
        cfg.enabled,
        mode_label,
        cfg.max_sessions,
        cfg.lookback_hours,
        cfg.min_refresh_secs,
        !cfg.exclude_cron,
        !cfg.exclude_channel,
        !cfg.exclude_subagents,
        active,
    );
    CommandResult {
        content,
        action: Some(CommandAction::DisplayOnly),
    }
}
