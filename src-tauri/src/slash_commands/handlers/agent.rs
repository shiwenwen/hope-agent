use crate::agent_config::AgentSummary;
use crate::agent_loader;
use crate::session::SessionDB;
use crate::slash_commands::types::{CommandAction, CommandResult};
use std::sync::Arc;

/// /agent <name> — Switch to a different agent.
pub fn handle_agent(session_db: &Arc<SessionDB>, args: &str) -> Result<CommandResult, String> {
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
