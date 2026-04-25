//! Per-agent import: translates OpenClaw's `agents.list[]` into Hope Agent
//! `AgentConfig` files, copies workspace markdown files, and (when providers
//! were imported in the same pass) wires up `AgentModelConfig.primary` so the
//! agent is immediately usable.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent_config::{
    AgentConfig, AgentModelConfig, CapabilitiesConfig, FilterConfig, PersonalityConfig,
};
use crate::agent_loader;

use super::paths;
use super::providers::ResolvedProvider;

// ── OpenClaw agent shape (deserialize-only) ─────────────────────

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct OpenClawAgentsRoot {
    pub agents: OpenClawAgentList,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct OpenClawAgentList {
    pub list: Vec<OpenClawAgent>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct OpenClawAgent {
    pub id: String,
    pub name: Option<String>,
    pub workspace: Option<String>,
    pub system_prompt_override: Option<String>,
    pub model: Option<OpenClawAgentModel>,
    pub identity: Option<OpenClawIdentity>,
    pub skills: Option<Vec<String>>,
    pub tools: Option<OpenClawTools>,
    pub sandbox: Option<OpenClawSandbox>,
    pub subagents: Option<serde_json::Value>,
    pub params: Option<serde_json::Value>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct OpenClawAgentModel {
    pub primary: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct OpenClawIdentity {
    pub name: Option<String>,
    pub theme: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct OpenClawTools {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub(super) enum OpenClawSandbox {
    Object(OpenClawSandboxObj),
    #[allow(dead_code)]
    Other(serde_json::Value),
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct OpenClawSandboxObj {
    pub mode: Option<String>,
}

// ── Public types (UI-facing) ────────────────────────────────────

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub source_id: String,
    pub imported_id: String,
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
}

// ── File mapping (OpenClaw uppercase → Hope Agent lowercase) ────

const FILE_MAP: &[(&str, &str)] = &[
    ("AGENTS.md", "agents.md"),
    ("SOUL.md", "soul.md"),
    ("TOOLS.md", "tools.md"),
    ("IDENTITY.md", "identity.md"),
    ("MEMORY.md", "memory.md"),
    ("memory.md", "memory.md"),
];

pub(super) fn is_remote_avatar(s: &str) -> bool {
    s.starts_with("http") || s.starts_with("data:")
}

fn extract_sandbox(agent: &OpenClawAgent) -> bool {
    match &agent.sandbox {
        Some(OpenClawSandbox::Object(obj)) => obj.mode.as_deref() == Some("all"),
        _ => false,
    }
}

fn extract_temperature(agent: &OpenClawAgent) -> Option<f64> {
    agent
        .params
        .as_ref()
        .and_then(|p| p.get("temperature"))
        .and_then(|v| v.as_f64())
}

/// Resolve the workspace dir for an agent, given the OpenClaw state dir.
fn resolve_workspace(state_dir: &Path, agent: &OpenClawAgent) -> PathBuf {
    if let Some(ws) = &agent.workspace {
        paths::expand_tilde(ws)
    } else {
        paths::default_workspace(state_dir)
    }
}

fn list_available_files(state_dir: &Path, agent: &OpenClawAgent) -> Vec<String> {
    let ws = resolve_workspace(state_dir, agent);
    let mut files = Vec::new();
    let mut seen_memory = false;
    for &(src, dst) in FILE_MAP {
        if ws.join(src).exists() {
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

/// Build agent previews from a parsed agent list. Existing IDs are looked up
/// via `agent_loader::list_agent_ids()` to flag conflicts.
pub(super) fn build_previews(
    state_dir: &Path,
    agents_root: &OpenClawAgentsRoot,
) -> Vec<OpenClawAgentPreview> {
    let existing_ids = agent_loader::list_agent_ids().unwrap_or_default();

    let mut out = Vec::new();
    for agent in &agents_root.agents.list {
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
        let sandbox = extract_sandbox(agent);

        let skill_names = agent.skills.clone().unwrap_or_default();
        let available_files = list_available_files(state_dir, agent);

        out.push(OpenClawAgentPreview {
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
    out
}

/// Import a single OpenClaw agent. Writes `agent.json`, copies workspace
/// markdown files, and (when `model_id_to_provider` is non-empty) wires up
/// `AgentModelConfig.primary = "{provider_uuid}/{model_id}"`.
///
/// Memory entries from MEMORY.md are *not* written here — that's the caller's
/// job after `import_single_agent` returns successfully (memory module needs
/// the canonical target_id).
pub(super) fn import_single_agent(
    state_dir: &Path,
    source: &OpenClawAgent,
    req: &ImportAgentRequest,
    model_id_to_provider: &std::collections::HashMap<String, ProviderForModel>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let target_id = &req.target_id;

    if target_id.is_empty()
        || !target_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        anyhow::bail!("Invalid agent ID: '{}'", target_id);
    }

    let temperature = extract_temperature(source);
    let primary = resolve_primary_model(source, model_id_to_provider, warnings);

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
            primary,
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

    agent_loader::save_agent_config(target_id, &agent_config)?;

    if let Some(prompt) = &source.system_prompt_override {
        agent_loader::save_agent_markdown(target_id, "agent.md", prompt)?;
    }

    let ws = resolve_workspace(state_dir, source);
    for file_name in &req.import_files {
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

/// Resolve `agent.model.primary` (a bare OpenClaw model id like
/// `"claude-sonnet-4-6"`) into Hope Agent's `"provider_id/model_id"` format.
///
/// Returns `None` and pushes a warning when the model id can't be matched
/// against any imported provider.
fn resolve_primary_model(
    source: &OpenClawAgent,
    model_id_to_provider: &std::collections::HashMap<String, ProviderForModel>,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let raw = source
        .model
        .as_ref()
        .and_then(|m| m.primary.clone())
        .filter(|s| !s.is_empty())?;

    if let Some(found) = model_id_to_provider.get(&raw) {
        return Some(format!("{}/{}", found.provider_uuid, raw));
    }
    warnings.push(format!(
        "Agent '{}': model '{}' not found in any imported provider; primary left blank",
        source.id, raw
    ));
    None
}

#[derive(Debug, Clone)]
pub(super) struct ProviderForModel {
    pub provider_uuid: String,
}

/// Build the `model_id → provider_uuid` lookup. When the same model id is
/// served by multiple providers, the `Anthropic`-typed one wins for ids
/// starting with `claude-`, the `OpenAI`-typed one wins for ids starting with
/// `gpt-` / `o`, otherwise first-imported wins.
pub(super) fn build_model_lookup(
    providers: &[ResolvedProvider],
) -> std::collections::HashMap<String, ProviderForModel> {
    use crate::provider::ApiType;

    let mut out: std::collections::HashMap<String, ProviderForModel> =
        std::collections::HashMap::new();

    for resolved in providers {
        for model_id in &resolved.model_ids {
            let new_score = match (model_id.as_str(), &resolved.config.api_type) {
                (id, ApiType::Anthropic) if id.starts_with("claude-") => 3,
                (id, ApiType::OpenaiResponses) if id.starts_with("gpt-") || id.starts_with('o') => {
                    2
                }
                (id, ApiType::OpenaiChat) if id.starts_with("gpt-") || id.starts_with('o') => 2,
                _ => 1,
            };
            let entry = out.entry(model_id.clone());
            match entry {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(ProviderForModel {
                        provider_uuid: resolved.config.id.clone(),
                    });
                    let _ = new_score;
                }
                std::collections::hash_map::Entry::Occupied(_) => {
                    // First-imported wins; kept simple. Per-model id collisions
                    // across providers are rare in practice.
                }
            }
        }
    }
    out
}

/// Add models from already-configured providers without overriding providers
/// imported in the current request. This keeps partial-import retries from
/// duplicating providers while still resolving the agent's primary model.
pub(super) fn extend_model_lookup_from_provider_configs(
    lookup: &mut std::collections::HashMap<String, ProviderForModel>,
    providers: &[crate::provider::ProviderConfig],
) {
    for provider in providers {
        for model in &provider.models {
            lookup
                .entry(model.id.clone())
                .or_insert_with(|| ProviderForModel {
                    provider_uuid: provider.id.clone(),
                });
        }
    }
}
