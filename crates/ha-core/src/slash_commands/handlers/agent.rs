use crate::agent_config::AgentSummary;
use crate::agent_loader;
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};
use std::sync::Arc;

/// /agent <name> — Switch to a different agent.
///
/// IM channels are forbidden from invoking `/agent` because the IM dispatcher
/// resolves agent_id from `channel_account` config on every inbound message
/// (see `channel/worker/dispatcher.rs::resolved_agent_id`), not from the
/// session's stored agent_id. Allowing `/agent` in IM would silently desync:
/// the new session's stored agent_id is the matched one, but subsequent
/// inbound messages would still run under the channel-account agent — a
/// hallucinated switch. The IM_DISABLED_COMMANDS list in `registry.rs` keeps
/// the menu in sync with this runtime check.
pub fn handle_agent(
    session_db: &Arc<SessionDB>,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    if let Some(sid) = session_id {
        if let Ok(Some(meta)) = session_db.get_session(sid) {
            if meta.channel_info.is_some() {
                return Ok(CommandResult {
                    content: "`/agent` is not available in IM channels. The active agent for an IM chat is decided by the channel-account / topic / group settings — change it under **Settings → IM Channel → <account> → Agent** (or per-topic / per-group override).".into(),
                    action: Some(CommandAction::DisplayOnly),
                });
            }
        }
    }

    let query = args.trim();
    if query.is_empty() {
        return Err("Usage: /agent <name>".into());
    }

    let agents = agent_loader::list_agents().map_err(|e| e.to_string())?;
    let matched = fuzzy_match_agent(&agents, query)?;

    // Create a new session for the matched agent
    let meta = session_db
        .create_session(&matched.id)
        .map_err(|e| e.to_string())?;

    Ok(CommandResult {
        content: format!("Switched to agent **{}**", matched.name),
        action: Some(CommandAction::SwitchAgent {
            agent_id: matched.id.clone(),
            session_id: meta.id,
        }),
    })
}

/// /agents — List available agents.
pub fn handle_agents() -> Result<CommandResult, String> {
    let agents = agent_loader::list_agents().map_err(|e| e.to_string())?;

    if agents.is_empty() {
        return Ok(CommandResult {
            content: "No agents configured.".into(),
            action: Some(CommandAction::DisplayOnly),
        });
    }

    let mut lines = vec![format!("**Available Agents** ({})\n", agents.len())];
    for a in &agents {
        let emoji = a.emoji.as_deref().unwrap_or("");
        let desc = a
            .description
            .as_deref()
            .map(|d| format!(" — {}", d))
            .unwrap_or_default();
        lines.push(format!("- {} **{}**{}", emoji, a.name, desc));
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// Fuzzy match an agent by name or id.
fn fuzzy_match_agent(agents: &[AgentSummary], query: &str) -> Result<AgentSummary, String> {
    let q = query.to_lowercase();

    // Exact id match
    if let Some(a) = agents.iter().find(|a| a.id.to_lowercase() == q) {
        return Ok(a.clone());
    }

    // Exact name match
    if let Some(a) = agents.iter().find(|a| a.name.to_lowercase() == q) {
        return Ok(a.clone());
    }

    // Prefix match
    let prefix: Vec<_> = agents
        .iter()
        .filter(|a| a.name.to_lowercase().starts_with(&q) || a.id.to_lowercase().starts_with(&q))
        .collect();
    if prefix.len() == 1 {
        return Ok(prefix[0].clone());
    }

    // Contains match
    let contains: Vec<_> = agents
        .iter()
        .filter(|a| a.name.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q))
        .collect();
    if contains.len() == 1 {
        return Ok(contains[0].clone());
    }

    if contains.is_empty() {
        Err(format!("No agent matching `{}`", query))
    } else {
        let names: Vec<String> = contains.iter().map(|a| format!("`{}`", a.name)).collect();
        Err(format!(
            "Ambiguous agent `{}`. Matches: {}",
            query,
            names.join(", ")
        ))
    }
}
