use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent_config::{
    AgentConfig, AgentModelConfig, CapabilitiesConfig, FilterConfig, PersonalityConfig,
};
use crate::agent_loader;

// ── OpenClaw Config Parsing (Deserialize only) ─────────────────

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenClawConfig {
    agents: OpenClawAgents,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenClawAgents {
    list: Vec<OpenClawAgent>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct OpenClawAgent {
    id: String,
    name: Option<String>,
    workspace: Option<String>,
    system_prompt_override: Option<String>,
    model: Option<OpenClawModel>,
    identity: Option<OpenClawIdentity>,
    skills: Option<Vec<String>>,
    tools: Option<OpenClawTools>,
    sandbox: Option<OpenClawSandbox>,
    subagents: Option<serde_json::Value>,
    params: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenClawModel {
    primary: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenClawIdentity {
    name: Option<String>,
    theme: Option<String>,
    emoji: Option<String>,
    avatar: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenClawTools {
    allow: Option<Vec<String>>,
    deny: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OpenClawSandbox {
    Object(OpenClawSandboxObj),
    #[allow(dead_code)]
    Other(serde_json::Value),
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OpenClawSandboxObj {
    mode: Option<String>,
}

// ── Public Types ───────────────────────────────────────────────

/// Preview of an OpenClaw agent for the frontend scan step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawAgentPreview {
    pub id: String,
    pub name: String,
    pub emoji: Option<String>,
    pub theme: Option<String>,
    pub avatar: Option<String>,
    pub model_info: Option<String>,
    pub has_system_prompt: bool,
    pub sandbox: bool,
    pub skill_names: Vec<String>,
    pub available_files: Vec<String>,
    pub already_exists: bool,
}

/// User-edited import request for a single agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAgentRequest {
    pub source_id: String,
    pub target_id: String,
    pub name: String,
    pub emoji: Option<String>,
    pub vibe: Option<String>,
    pub sandbox: bool,
    pub import_files: Vec<String>,
}

/// Result of importing a single agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub source_id: String,
    pub imported_id: String,
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Check if a string looks like a valid remote avatar URL or data URI.
fn is_remote_avatar(s: &str) -> bool {
    s.starts_with("http") || s.starts_with("data:")
}

// ── Workspace file mapping (OpenClaw uppercase → OC lowercase) ──

const FILE_MAP: &[(&str, &str)] = &[
    ("AGENTS.md", "agents.md"),
    ("SOUL.md", "soul.md"),
    ("TOOLS.md", "tools.md"),
    ("IDENTITY.md", "identity.md"),
    ("MEMORY.md", "memory.md"),
    ("memory.md", "memory.md"),
];

// ── Public Functions ───────────────────────────────────────────

/// Resolve the OpenClaw config path: ~/.openclaw/openclaw.json
fn openclaw_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".openclaw").join("openclaw.json"))
}

/// Resolve the default OpenClaw workspace path.
fn openclaw_default_workspace() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".openclaw").join("workspace"))
}

/// Parse the OpenClaw config file.
fn parse_openclaw_config() -> Result<OpenClawConfig> {
    let path = openclaw_config_path()?;
    if !path.exists() {
        anyhow::bail!("OpenClaw config not found at {}", path.display());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let config: OpenClawConfig = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(config)
}

/// Expand leading `~` or `~/` to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    Path::new(path).to_path_buf()
}

/// Resolve the workspace directory for an OpenClaw agent.
fn resolve_workspace(agent: &OpenClawAgent) -> Result<PathBuf> {
    if let Some(ws) = &agent.workspace {
        Ok(expand_tilde(ws))
    } else {
        openclaw_default_workspace()
    }
}

/// List available workspace files for an OpenClaw agent.
fn list_available_files(agent: &OpenClawAgent) -> Vec<String> {
    let ws = match resolve_workspace(agent) {
        Ok(ws) => ws,
        Err(_) => return Vec::new(),
    };
    let mut files = Vec::new();
    let mut seen_memory = false;
    for &(src, dst) in FILE_MAP {
        if ws.join(src).exists() {
            // Deduplicate memory.md (both MEMORY.md and memory.md map to the same target)
            if dst == "memory.md" {
                if seen_memory {
                    continue;
                }
                seen_memory = true;
            }
            files.push(dst.to_string());
        }
    }
    files
}

/// Extract sandbox mode from OpenClaw agent config.
fn extract_sandbox(agent: &OpenClawAgent) -> bool {
    match &agent.sandbox {
        Some(OpenClawSandbox::Object(obj)) => obj.mode.as_deref() == Some("all"),
        _ => false,
    }
}

/// Extract temperature from OpenClaw agent params.
fn extract_temperature(agent: &OpenClawAgent) -> Option<f64> {
    agent
        .params
        .as_ref()
        .and_then(|p| p.get("temperature"))
        .and_then(|v| v.as_f64())
}

/// Scan OpenClaw agents and return previews.
pub fn scan_openclaw_agents() -> Result<Vec<OpenClawAgentPreview>> {
    let config = parse_openclaw_config()?;
    let existing_agents = agent_loader::list_agents().unwrap_or_default();
    let existing_ids: std::collections::HashSet<String> =
        existing_agents.iter().map(|a| a.id.clone()).collect();

    let mut previews = Vec::new();
    for agent in &config.agents.list {
        if agent.id.is_empty() {
            continue;
        }

        let name = agent
            .identity
            .as_ref()
            .and_then(|i| i.name.clone())
            .or_else(|| agent.name.clone())
            .unwrap_or_else(|| agent.id.clone());

        let emoji = agent.identity.as_ref().and_then(|i| i.emoji.clone());
        let theme = agent.identity.as_ref().and_then(|i| i.theme.clone());
        let avatar = agent
            .identity
            .as_ref()
            .and_then(|i| i.avatar.clone())
            .filter(|a| is_remote_avatar(a));

        let model_info = agent.model.as_ref().and_then(|m| m.primary.clone());
        let has_system_prompt = agent.system_prompt_override.is_some();
        let sandbox = extract_sandbox(&agent);

        let skill_names = agent.skills.clone().unwrap_or_default();
        let available_files = list_available_files(&agent);

        previews.push(OpenClawAgentPreview {
            id: agent.id.clone(),
            name,
            emoji,
            theme,
            avatar,
            model_info,
            has_system_prompt,
            sandbox,
            skill_names,
            available_files,
            already_exists: existing_ids.contains(&agent.id),
        });
    }

    Ok(previews)
}

/// Import OpenClaw agents with user-edited fields.
pub fn import_openclaw_agents(requests: &[ImportAgentRequest]) -> Result<Vec<ImportResult>> {
    let config = parse_openclaw_config()?;
    let source_map: std::collections::HashMap<&str, &OpenClawAgent> = config
        .agents
        .list
        .iter()
        .map(|a| (a.id.as_str(), a))
        .collect();

    let mut results = Vec::new();

    for req in requests {
        let result = match source_map.get(req.source_id.as_str()) {
            Some(source) => import_single_agent(source, req),
            None => Err(anyhow::anyhow!(
                "Agent '{}' not found in OpenClaw config",
                req.source_id
            )),
        };

        match result {
            Ok(()) => results.push(ImportResult {
                source_id: req.source_id.clone(),
                imported_id: req.target_id.clone(),
                name: req.name.clone(),
                success: true,
                error: None,
            }),
            Err(e) => results.push(ImportResult {
                source_id: req.source_id.clone(),
                imported_id: req.target_id.clone(),
                name: req.name.clone(),
                success: false,
                error: Some(e.to_string()),
            }),
        }
    }

    Ok(results)
}

/// Import a single agent from OpenClaw.
fn import_single_agent(source: &OpenClawAgent, req: &ImportAgentRequest) -> Result<()> {
    let target_id = &req.target_id;

    // Validate target ID
    if target_id.is_empty()
        || !target_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        anyhow::bail!("Invalid agent ID: '{}'", target_id);
    }

    // Build AgentConfig
    let temperature = extract_temperature(source);

    let tools = source
        .tools
        .as_ref()
        .map(|t| FilterConfig {
            allow: t.allow.clone().unwrap_or_default(),
            deny: t.deny.clone().unwrap_or_default(),
        })
        .unwrap_or_default();

    let skills = FilterConfig {
        allow: source.skills.clone().unwrap_or_default(),
        deny: Vec::new(),
    };

    let has_subagents = source
        .subagents
        .as_ref()
        .map(|v| !v.is_null())
        .unwrap_or(false);

    let agent_config = AgentConfig {
        name: req.name.clone(),
        emoji: req.emoji.clone(),
        avatar: source
            .identity
            .as_ref()
            .and_then(|i| i.avatar.clone())
            .filter(|a| is_remote_avatar(a)),
        model: AgentModelConfig {
            primary: None, // User will configure model after import
            temperature,
            ..Default::default()
        },
        personality: PersonalityConfig {
            vibe: req.vibe.clone(),
            ..Default::default()
        },
        capabilities: CapabilitiesConfig {
            sandbox: req.sandbox,
            tools,
            skills,
            ..Default::default()
        },
        openclaw_mode: true,
        subagents: crate::agent_config::SubagentConfig {
            enabled: has_subagents,
            ..Default::default()
        },
        ..Default::default()
    };

    // Save agent config
    agent_loader::save_agent_config(target_id, &agent_config)?;

    // Write system prompt override as agent.md
    if let Some(prompt) = &source.system_prompt_override {
        agent_loader::save_agent_markdown(target_id, "agent.md", prompt)?;
    }

    // Copy workspace files
    let ws = resolve_workspace(source)?;
    for file_name in &req.import_files {
        // Find the source file (try uppercase first, then lowercase)
        let src_path = FILE_MAP
            .iter()
            .filter(|&&(_, dst)| dst == file_name.as_str())
            .map(|&(src, _)| ws.join(src))
            .find(|p| p.exists());

        if let Some(src_path) = src_path {
            let content = std::fs::read_to_string(&src_path)
                .with_context(|| format!("Failed to read workspace file {}", src_path.display()))?;
            if !content.is_empty() {
                agent_loader::save_agent_markdown(target_id, file_name, &content)?;
            }
        }
    }

    Ok(())
}
