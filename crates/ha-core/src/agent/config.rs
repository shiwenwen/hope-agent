use serde_json::json;

use super::types::CodexModel;
use crate::provider::ThinkingStyle;

pub(super) const CODEX_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
#[allow(dead_code)]
pub(super) const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// User-Agent header for all outgoing HTTP requests.
/// Some API providers (e.g. DashScope CodingPlan) use WAF rules that filter
/// requests based on User-Agent. Using a recognized coding-tool-style UA
/// ensures compatibility with these services.
pub const USER_AGENT: &str = "Hope Agent/1.0";

/// Smart URL builder: if base_url already ends with a version suffix
/// (e.g. /v1, /v2, /v3), strip the version prefix from path to avoid
/// double-prefixing like /v3/v1/chat/completions.
pub fn build_api_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let version_prefixes = ["/v1", "/v2", "/v3"];

    // Check if base already has any version suffix
    let base_has_version = version_prefixes.iter().any(|p| base.ends_with(p));

    if base_has_version {
        // Strip version prefix from path if present
        for prefix in &version_prefixes {
            if path.starts_with(prefix) {
                return format!("{}{}", base, &path[prefix.len()..]);
            }
        }
    }

    format!("{}{}", base, path)
}

#[allow(dead_code)]
pub(super) const ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
pub(super) const ANTHROPIC_API_VERSION: &str = "2023-06-01";
pub(super) const MAX_RETRIES: u32 = 3;
pub(super) const BASE_DELAY_MS: u64 = 1000;
pub(super) const DEFAULT_MAX_TOOL_ROUNDS: u32 = 20;

/// Get the configured max tool rounds from the current agent.
/// Returns 0 for unlimited.
pub(super) fn get_max_tool_rounds() -> u32 {
    crate::agent_loader::load_agent("default")
        .map(|def| def.config.capabilities.max_tool_rounds)
        .unwrap_or(DEFAULT_MAX_TOOL_ROUNDS)
}

/// Whether `id` matches one of the well-known Codex OAuth model IDs.
/// Cheap linear scan over the fixed list returned by [`get_codex_models`];
/// shared by the Tauri `set_codex_model` command and the HTTP handler so
/// validation stays in sync when the list changes.
pub fn is_valid_codex_model(id: &str) -> bool {
    get_codex_models().iter().any(|m| m.id == id)
}

pub fn get_codex_models() -> Vec<CodexModel> {
    vec![
        CodexModel {
            id: "gpt-5.4".into(),
            name: "GPT-5.4".into(),
        },
        CodexModel {
            id: "gpt-5.3-codex".into(),
            name: "GPT-5.3 Codex".into(),
        },
        CodexModel {
            id: "gpt-5.3-codex-spark".into(),
            name: "GPT-5.3 Codex Spark".into(),
        },
        CodexModel {
            id: "gpt-5.2".into(),
            name: "GPT-5.2".into(),
        },
        CodexModel {
            id: "gpt-5.2-codex".into(),
            name: "GPT-5.2 Codex".into(),
        },
        CodexModel {
            id: "gpt-5.1".into(),
            name: "GPT-5.1".into(),
        },
        CodexModel {
            id: "gpt-5.1-codex-max".into(),
            name: "GPT-5.1 Codex Max".into(),
        },
        CodexModel {
            id: "gpt-5.1-codex-mini".into(),
            name: "GPT-5.1 Codex Mini".into(),
        },
    ]
}

/// Read the live reasoning effort from global app state.
///
/// Returns the latest `AppState.reasoning_effort` (treating "none" as `None`)
/// if AppState is initialized, otherwise falls back to the caller-provided
/// value. Provider tool loops call this at the top of every round so a
/// user-side toggle (UI picker, `/think` slash, channel command) applies to
/// the very next API request instead of only to the next user message.
pub async fn live_reasoning_effort(fallback: Option<&str>) -> Option<String> {
    if let Some(cell) = crate::globals::get_reasoning_effort_cell() {
        let eff = cell.lock().await.clone();
        if eff == "none" {
            return None;
        }
        return Some(eff);
    }
    fallback.map(|s| s.to_string())
}

/// Clamp reasoning effort to valid range for the given model
pub fn clamp_reasoning_effort(model: &str, effort: &str) -> Option<String> {
    if effort == "none" {
        return None;
    }
    let efforts = ["minimal", "low", "medium", "high", "xhigh"];
    if !efforts.contains(&effort) {
        return Some("medium".to_string());
    }
    if model.contains("5.1-codex-mini") {
        return match effort {
            "minimal" | "low" => Some("medium".to_string()),
            "xhigh" => Some("high".to_string()),
            _ => Some(effort.to_string()),
        };
    }
    if model.contains("5.1") {
        return match effort {
            "minimal" => Some("low".to_string()),
            "xhigh" => Some("high".to_string()),
            _ => Some(effort.to_string()),
        };
    }
    Some(effort.to_string())
}

/// Map reasoning effort to Anthropic/ZAI thinking parameter.
/// Anthropic/ZAI uses `thinking: { type: "enabled", budget_tokens: N }` format.
/// Returns None if thinking should be disabled.
pub(super) fn map_think_anthropic_style(
    effort: Option<&str>,
    max_tokens: u32,
) -> Option<serde_json::Value> {
    let effort = effort?;
    if effort == "none" {
        return None;
    }
    // Map effort level to budget_tokens
    let budget: u32 = match effort {
        "low" => 1024,
        "medium" => 4096,
        "high" => 8192,
        "xhigh" => 16384,
        _ => return None,
    };
    // Anthropic requires budget_tokens < max_tokens specified in request
    let capped_budget = budget.min(max_tokens.saturating_sub(1));
    Some(json!({
        "type": "enabled",
        "budget_tokens": capped_budget
    }))
}

/// Map reasoning effort to OpenAI `reasoning_effort` parameter.
/// Chat Completions supports "low", "medium", "high" (no xhigh).
/// Returns None if thinking should be disabled.
fn map_think_openai_style(effort: Option<&str>) -> Option<String> {
    let effort = effort?;
    match effort {
        "none" => None,
        "xhigh" => Some("high".to_string()), // Downgrade xhigh to high for Chat Completions
        "minimal" | "low" | "medium" | "high" => Some(effort.to_string()),
        _ => None,
    }
}

/// Map reasoning effort to Qwen `enable_thinking` parameter.
/// Returns None if thinking should be disabled.
fn map_think_qwen_style(effort: Option<&str>) -> Option<bool> {
    let effort = effort?;
    match effort {
        "none" => Some(false),
        "low" | "medium" | "high" | "xhigh" => Some(true),
        _ => None,
    }
}

/// Apply thinking parameters to an OpenAI Chat Completions body based on ThinkingStyle.
pub(super) fn apply_thinking_to_chat_body(
    body: &mut serde_json::Value,
    thinking_style: &ThinkingStyle,
    reasoning_effort: Option<&str>,
    max_tokens: u32,
) {
    match thinking_style {
        ThinkingStyle::Openai => {
            if let Some(effort) = map_think_openai_style(reasoning_effort) {
                body["reasoning_effort"] = json!(effort);
            }
        }
        ThinkingStyle::Anthropic | ThinkingStyle::Zai => {
            if let Some(think_config) = map_think_anthropic_style(reasoning_effort, max_tokens) {
                body["thinking"] = think_config;
            }
        }
        ThinkingStyle::Qwen => {
            if let Some(enable) = map_think_qwen_style(reasoning_effort) {
                body["enable_thinking"] = json!(enable);
            }
        }
        ThinkingStyle::None => {
            // Do not send any thinking/reasoning parameters
        }
    }
}

/// Build the full system prompt.
/// Uses the new system_prompt module with AgentDefinition if available,
/// otherwise falls back to legacy behavior for backward compatibility.
pub fn build_system_prompt(agent_id: &str, model: &str, provider: &str) -> String {
    build_system_prompt_with_session(agent_id, model, provider, None)
}

/// Project-aware variant of [`build_system_prompt`]. When `session_id` is
/// supplied and its session is attached to a project, the system prompt
/// includes a "Current Project" section, the project's shared-file catalog,
/// and memories that are scoped to that project.
pub fn build_system_prompt_with_session(
    agent_id: &str,
    model: &str,
    provider: &str,
    session_id: Option<&str>,
) -> String {
    // Try loading the agent definition
    if let Ok(definition) = crate::agent_loader::load_agent(agent_id) {
        let session_meta = crate::session::lookup_session_meta(session_id);
        let incognito = session_meta.as_ref().map(|s| s.incognito).unwrap_or(false);

        // Resolve the current project (if any) via session → session.project_id.
        let project = session_meta
            .as_ref()
            .and_then(|s| s.project_id.clone())
            .and_then(|pid| crate::get_project_db()?.get(&pid).ok().flatten());

        // Load project files if we have a project.
        let project_files: Vec<crate::project::ProjectFile> = project
            .as_ref()
            .and_then(|p| crate::get_project_db().and_then(|db| db.list_files(&p.id).ok()))
            .unwrap_or_default();

        // Load candidate memory entries (unscoped raw list). Budget-based
        // filtering and per-section sub-budgets are applied downstream by
        // `system_prompt::build` so that Layer 1/2 (core memory.md files) can
        // consume the total budget first and Layer 3 picks up only the residual.
        let memory_entries: Vec<crate::memory::MemoryEntry> =
            if definition.config.memory.enabled && !incognito {
                crate::get_memory_backend()
                    .and_then(|b| {
                        b.load_prompt_candidates_with_project(
                            agent_id,
                            project.as_ref().map(|p| p.id.as_str()),
                            definition.config.memory.shared,
                        )
                        .ok()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

        // Resolve the effective memory budget (agent override wins over global).
        let app_cfg = crate::config::cached_config();
        let memory_budget = crate::agent_config::effective_memory_budget(
            &definition.config.memory,
            &app_cfg.memory_budget,
        );

        // Resolve agent home directory
        let agent_home = crate::paths::agent_home_dir(agent_id)
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        return crate::system_prompt::build(
            &definition,
            Some(model),
            Some(provider),
            &memory_entries,
            &memory_budget,
            agent_home.as_deref(),
            project.as_ref(),
            &project_files,
            session_id,
            incognito,
        );
    }
    // Fallback: legacy prompt
    crate::system_prompt::build_legacy(
        Some(model),
        Some(provider),
        crate::session::is_session_incognito(session_id),
    )
}
