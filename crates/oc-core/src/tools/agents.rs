use anyhow::Result;
use serde_json::Value;

/// Tool: agents_list — list all available agents with their metadata.
pub(crate) async fn tool_agents_list(_args: &Value) -> Result<String> {
    let agents = crate::agent_loader::list_agents()
        .map_err(|e| anyhow::anyhow!("Failed to list agents: {}", e))?;

    if agents.is_empty() {
        return Ok("No agents configured.".to_string());
    }

    let mut output = format!("Available agents ({}):\n", agents.len());

    for (i, agent) in agents.iter().enumerate() {
        let emoji = agent.emoji.as_deref().unwrap_or("");
        output.push_str(&format!(
            "\n{}. {} — \"{}\" {}\n",
            i + 1,
            agent.id,
            agent.name,
            emoji
        ));

        if let Some(desc) = &agent.description {
            output.push_str(&format!("   {}\n", desc));
        }

        let mut config_parts = Vec::new();
        if agent.has_agent_md {
            config_parts.push("agent.md");
        }
        if agent.has_persona {
            config_parts.push("persona");
        }
        if agent.has_tools_guide {
            config_parts.push("tools.md");
        }

        output.push_str(&format!(
            "   Config: {} | Memories: {}\n",
            if config_parts.is_empty() {
                "none".to_string()
            } else {
                config_parts.join(", ")
            },
            agent.memory_count,
        ));
    }

    Ok(output)
}
