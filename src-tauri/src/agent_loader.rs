use anyhow::{Context, Result};
use std::path::Path;

use crate::agent_config::{AgentConfig, AgentDefinition, AgentSummary};
use crate::paths;

// ── Constants ────────────────────────────────────────────────────

const DEFAULT_AGENT_ID: &str = "default";

/// The Markdown files an agent directory may contain.
const AGENT_MD: &str = "agent.md";
const PERSONA_MD: &str = "persona.md";
const TOOLS_MD: &str = "tools.md";

// ── Default Agent Template ───────────────────────────────────────

fn default_agent_json() -> AgentConfig {
    AgentConfig {
        name: "Assistant".to_string(),
        description: Some("General-purpose AI assistant".to_string()),
        emoji: Some("🤖".to_string()),
        ..AgentConfig::default()
    }
}

const DEFAULT_AGENT_MD: &str = r#"You are OpenComputer, a personal AI assistant with deep system integration.
You help users interact with their computer naturally and efficiently.

## Principles

- Be concise and direct — act first, explain if needed
- Read existing code before making changes
- Prefer editing existing files over creating new ones
- Keep changes minimal and focused
- Ask for clarification when unsure
- Never execute dangerous operations without explicit confirmation
"#;

// ── Ensure Default Agent ─────────────────────────────────────────

/// Create the default agent directory and files if they don't exist.
/// Called on app startup.
pub fn ensure_default_agent() -> Result<()> {
    let dir = paths::agent_dir(DEFAULT_AGENT_ID)?;
    let config_path = dir.join("agent.json");

    if config_path.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(&dir)?;

    // Write agent.json
    let config = default_agent_json();
    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, json)?;

    // Write agent.md
    std::fs::write(dir.join(AGENT_MD), DEFAULT_AGENT_MD)?;

    Ok(())
}

// ── Load Agent ───────────────────────────────────────────────────

/// Load a complete AgentDefinition from ~/.opencomputer/agents/{id}/
pub fn load_agent(id: &str) -> Result<AgentDefinition> {
    let dir = paths::agent_dir(id)?;
    if !dir.exists() {
        anyhow::bail!("Agent '{}' not found at {}", id, dir.display());
    }

    // Load agent.json (required)
    let config_path = dir.join("agent.json");
    let config: AgentConfig = if config_path.exists() {
        let data = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        serde_json::from_str(&data)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?
    } else {
        AgentConfig::default()
    };

    // Load optional markdown files
    let agent_md = read_optional_md(&dir, AGENT_MD)?;
    let persona = read_optional_md(&dir, PERSONA_MD)?;
    let tools_guide = read_optional_md(&dir, TOOLS_MD)?;

    Ok(AgentDefinition {
        id: id.to_string(),
        dir,
        config,
        agent_md,
        persona,
        tools_guide,
    })
}

/// Read a markdown file if it exists, return None if missing.
/// Returns None for empty files too.
fn read_optional_md(dir: &Path, filename: &str) -> Result<Option<String>> {
    let path = dir.join(filename);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(content))
}

// ── List Agents ──────────────────────────────────────────────────

/// List all available agents from ~/.opencomputer/agents/
pub fn list_agents() -> Result<Vec<AgentSummary>> {
    let agents_dir = paths::agents_dir()?;
    if !agents_dir.exists() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::new();

    for entry in std::fs::read_dir(&agents_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let id = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Try loading the config, skip if invalid
        let config_path = path.join("agent.json");
        let config: AgentConfig = if config_path.exists() {
            match std::fs::read_to_string(&config_path)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
            {
                Some(c) => c,
                None => continue,
            }
        } else {
            AgentConfig::default()
        };

        summaries.push(AgentSummary {
            id,
            name: config.name,
            description: config.description,
            emoji: config.emoji,
            avatar: config.avatar,
            has_agent_md: path.join(AGENT_MD).exists(),
            has_persona: path.join(PERSONA_MD).exists(),
            has_tools_guide: path.join(TOOLS_MD).exists(),
        });
    }

    // Sort: "default" first, then alphabetical
    summaries.sort_by(|a, b| {
        let a_default = a.id == DEFAULT_AGENT_ID;
        let b_default = b.id == DEFAULT_AGENT_ID;
        b_default.cmp(&a_default).then(a.id.cmp(&b.id))
    });

    Ok(summaries)
}

// ── Save Agent Config ────────────────────────────────────────────

/// Save agent.json for the given agent ID. Creates the directory if needed.
pub fn save_agent_config(id: &str, config: &AgentConfig) -> Result<()> {
    let dir = paths::agent_dir(id)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("agent.json");
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ── Save Agent Markdown ──────────────────────────────────────────

/// Save a markdown file for the given agent.
/// `file` must be one of: "agent.md", "persona.md", "tools.md"
pub fn save_agent_markdown(id: &str, file: &str, content: &str) -> Result<()> {
    // Validate filename to prevent path traversal
    match file {
        AGENT_MD | PERSONA_MD | TOOLS_MD => {}
        _ => anyhow::bail!("Invalid agent markdown file: {}", file),
    }

    let dir = paths::agent_dir(id)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    std::fs::write(&path, content)?;
    Ok(())
}

// ── Get Agent Markdown ───────────────────────────────────────────

/// Read a markdown file for the given agent.
pub fn get_agent_markdown(id: &str, file: &str) -> Result<Option<String>> {
    match file {
        AGENT_MD | PERSONA_MD | TOOLS_MD => {}
        _ => anyhow::bail!("Invalid agent markdown file: {}", file),
    }
    let dir = paths::agent_dir(id)?;
    read_optional_md(&dir, file)
}

// ── Delete Agent ─────────────────────────────────────────────────

/// Delete an agent directory. Refuses to delete "default".
pub fn delete_agent(id: &str) -> Result<()> {
    if id == DEFAULT_AGENT_ID {
        anyhow::bail!("Cannot delete the default agent");
    }
    let dir = paths::agent_dir(id)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}
