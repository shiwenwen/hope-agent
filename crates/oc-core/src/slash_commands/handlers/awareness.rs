//! `/awareness` — toggle or inspect the behavior awareness
//! feature at the global level.
//!
//! Subcommands:
//!   /awareness                → show current status
//!   /awareness on|off         → flip global enabled
//!   /awareness mode structured|llm|off
//!   /awareness status         → detailed runtime status

use crate::awareness::AwarenessMode;
use crate::slash_commands::types::{CommandAction, CommandResult};

/// Handle the `/awareness` slash command.
pub fn handle_awareness(args: &str) -> Result<CommandResult, String> {
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
            Some("off") => set_mode(AwarenessMode::Off),
            Some("structured") => set_mode(AwarenessMode::Structured),
            Some("llm") | Some("llm_digest") | Some("digest") => {
                set_mode(AwarenessMode::LlmDigest)
            }
            _ => Err("Usage: /awareness mode [off|structured|llm|llm_digest|digest]".to_string()),
        },
        "status" => Ok(status_result()),
        other => Err(format!(
            "Unknown subcommand '{}'. Try: /awareness [on|off|mode <x>|status]",
            other
        )),
    }
}

fn set_enabled(enabled: bool) -> Result<CommandResult, String> {
    let mut store = crate::config::load_config().map_err(|e| e.to_string())?;
    store.awareness.enabled = enabled;
    crate::config::save_config(&store).map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!(
            "Behavior awareness {}.",
            if enabled { "enabled" } else { "disabled" }
        ),
        action: Some(CommandAction::DisplayOnly),
    })
}

fn set_mode(mode: AwarenessMode) -> Result<CommandResult, String> {
    let mut store = crate::config::load_config().map_err(|e| e.to_string())?;
    store.awareness.mode = mode;
    // Enabling a concrete mode implies the feature is on.
    if !matches!(mode, AwarenessMode::Off) {
        store.awareness.enabled = true;
    }
    crate::config::save_config(&store).map_err(|e| e.to_string())?;
    let label = match mode {
        AwarenessMode::Off => "off",
        AwarenessMode::Structured => "structured (zero LLM cost)",
        AwarenessMode::LlmDigest => "llm_digest (extra side_query per turn)",
    };
    Ok(CommandResult {
        content: format!("Behavior awareness mode set to **{}**.", label),
        action: Some(CommandAction::DisplayOnly),
    })
}

fn status_result() -> CommandResult {
    let cfg = crate::config::cached_config().awareness.clone();
    let mode_label = match cfg.mode {
        AwarenessMode::Off => "off",
        AwarenessMode::Structured => "structured",
        AwarenessMode::LlmDigest => "llm_digest",
    };
    let active = crate::awareness::active_snapshot().len();
    let content = format!(
        "**Behavior Awareness**\n\n\
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
