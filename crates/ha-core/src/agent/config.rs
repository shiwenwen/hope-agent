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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompleteEndpointKind {
    ChatCompletions,
    Responses,
    Messages,
}

impl CompleteEndpointKind {
    const fn suffix(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/chat/completions",
            Self::Responses => "/responses",
            Self::Messages => "/messages",
        }
    }
}

fn complete_endpoint_kind(url: &str) -> Option<CompleteEndpointKind> {
    let url = url.trim_end_matches('/');
    [
        CompleteEndpointKind::ChatCompletions,
        CompleteEndpointKind::Responses,
        CompleteEndpointKind::Messages,
    ]
    .into_iter()
    .find(|kind| url.ends_with(kind.suffix()))
}

pub fn is_complete_endpoint_url(url: &str) -> bool {
    complete_endpoint_kind(url).is_some()
}

/// Smart URL builder.
///
/// Rules, in order:
/// 1. If `base_url` already ends with the same complete endpoint as `path`,
///    return it unchanged.
/// 2. If `base_url` ends with some other complete endpoint, strip that suffix
///    first and rebuild from the endpoint parent.
/// 3. If the resulting base ends with `/v1`, `/v2`, `/v3`, strip the version
///    prefix from `path` to avoid double-prefixing like `/v3/v1/chat/completions`.
pub fn build_api_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_end_matches('/');

    let base = if let Some(base_kind) = complete_endpoint_kind(base) {
        if complete_endpoint_kind(path) == Some(base_kind) {
            return base.to_string();
        }
        base.trim_end_matches(base_kind.suffix())
            .trim_end_matches('/')
    } else {
        base
    };

    let version_prefixes = ["/v1", "/v2", "/v3"];

    let base_has_version = version_prefixes.iter().any(|p| base.ends_with(p));

    if base_has_version {
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
pub(super) const DEFAULT_MAX_TOOL_ROUNDS: u32 = 0;

/// Get the configured max tool rounds from the current agent.
/// Returns 0 for unlimited.
pub(super) fn get_max_tool_rounds(agent_id: &str) -> u32 {
    crate::agent_loader::load_agent(agent_id)
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

/// Default Codex model id selected at login / onboarding when the user hasn't
/// picked one. Single source of truth. All auth paths (server / desktop /
/// CLI) reference this instead of hard-coding a model id, so bumping the
/// default is a one-line change.
///
/// Deliberately `gpt-5.6-terra`, not the flagship `gpt-5.6-sol` at the top of
/// [`get_codex_models`]: GPT-5.6 access is plan-tiered — Free/Go ChatGPT plans
/// only get Terra, Sol requires a paid plan or workspace. Every new Codex
/// login is activated on this id via `ActiveModelUpdate::Always` before the
/// app knows the account's plan, so it must stay on the tier every Codex
/// account actually has.
pub const DEFAULT_CODEX_MODEL_ID: &str = "gpt-5.6-terra";

pub fn get_codex_models() -> Vec<CodexModel> {
    vec![
        CodexModel {
            id: "gpt-5.6-sol".into(),
            name: "GPT-5.6 Sol".into(),
        },
        CodexModel {
            id: DEFAULT_CODEX_MODEL_ID.into(),
            name: "GPT-5.6 Terra".into(),
        },
        CodexModel {
            id: "gpt-5.6-luna".into(),
            name: "GPT-5.6 Luna".into(),
        },
        CodexModel {
            id: "gpt-5.5".into(),
            name: "GPT-5.5".into(),
        },
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
/// user-side toggle (UI picker, `/thinking` slash, channel command) applies to
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

pub const VALID_REASONING_EFFORTS: [&str; 6] =
    ["none", "minimal", "low", "medium", "high", "xhigh"];

pub fn is_valid_reasoning_effort(effort: &str) -> bool {
    VALID_REASONING_EFFORTS.contains(&effort)
}

/// Clamp reasoning effort to valid range for the given model
pub fn clamp_reasoning_effort(model: &str, effort: &str) -> Option<String> {
    if effort == "none" {
        return None;
    }
    if !is_valid_reasoning_effort(effort) {
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

pub(crate) struct SystemPromptBuild {
    pub prompt: String,
    pub static_memory_refs: Vec<super::active_memory::UsedMemoryRef>,
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
    build_system_prompt_bundle_with_session(agent_id, model, provider, session_id).prompt
}

/// Build the system prompt and the exact static-memory references represented
/// in it from the same snapshot. Keeping these together avoids a second pass
/// over agent files, session/project state, memory SQLite, profiles and claims.
pub(crate) fn build_system_prompt_bundle_with_session(
    agent_id: &str,
    model: &str,
    provider: &str,
    session_id: Option<&str>,
) -> SystemPromptBuild {
    build_system_prompt_bundle_with_session_db(
        agent_id,
        model,
        provider,
        session_id,
        crate::get_session_db().map(std::sync::Arc::as_ref),
    )
}

/// Bound-database variant used by chat-engine turns. Supplying a database is
/// authoritative: missing rows fail closed instead of falling back to the
/// process-global store and mixing isolated eval/headless state with desktop
/// session state.
pub(crate) fn build_system_prompt_bundle_with_session_db(
    agent_id: &str,
    model: &str,
    provider: &str,
    session_id: Option<&str>,
    session_db: Option<&crate::session::SessionDB>,
) -> SystemPromptBuild {
    let (session_meta, active_goal) = resolve_prompt_session_state(session_id, session_db);
    let incognito = session_meta
        .as_ref()
        .map(|session| session.incognito)
        .unwrap_or(session_id.is_some() && session_db.is_some());

    // Try loading the agent definition
    if let Ok(definition) = crate::agent_loader::load_agent(agent_id) {
        // Resolve the current project (if any) via session → session.project_id.
        let project = session_meta
            .as_ref()
            .and_then(|s| s.project_id.clone())
            .and_then(|pid| crate::get_project_db()?.get(&pid).ok().flatten());

        // Load candidate memory entries (unscoped raw list). Budget-based
        // filtering and per-section sub-budgets are applied downstream by
        // `system_prompt::build` so that Layer 1/2 (core memory.md files) can
        // consume the total budget first and Layer 3 picks up only the residual.
        let app_cfg = crate::config::cached_config();
        let long_term_memory_enabled = app_cfg.memory_extract.enabled;

        let memory_entries: Vec<crate::memory::MemoryEntry> =
            if long_term_memory_enabled && definition.config.memory.enabled && !incognito {
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
        let memory_budget = crate::agent_config::effective_memory_budget(
            &definition.config.memory,
            &app_cfg.memory_budget,
        );

        // Memory Profile snapshot (next-gen Dreaming Phase 4): when profile
        // synthesis is enabled and a snapshot exists, it renders the
        // `## User Profile` section in place of the legacy profile-tagged
        // memories; otherwise the legacy rendering is the fallback (so disabling
        // synthesis — the default — never blanks the section). Global +
        // current-agent snapshots are concatenated here; the project profile is
        // shown in the read-only view but injected via the Context Pack later.
        let mut profile_refs: Vec<super::active_memory::UsedMemoryRef> = Vec::new();
        let profile_snapshot: Option<String> = if long_term_memory_enabled
            && definition.config.memory.enabled
            && !incognito
            && app_cfg.dreaming.profile_synthesis.enabled
        {
            let mut parts: Vec<String> = Vec::new();
            for (scope_type, scope_id) in [("global", ""), ("agent", agent_id)] {
                if let Some(body) =
                    crate::memory::dreaming::latest_profile_body(scope_type, scope_id)
                {
                    let body = body.trim();
                    if !body.is_empty() {
                        if let Some(source_ref) =
                            super::profile_snapshot_ref(scope_type, scope_id, body)
                        {
                            profile_refs.push(source_ref);
                        }
                        parts.push(body.to_string());
                    }
                }
            }
            (!parts.is_empty()).then(|| parts.join("\n"))
        } else {
            None
        };

        // Context Pack — static Pinned Claims segment (next-gen Dreaming Phase 5,
        // design §4.8). Built once here (query-independent, cache-stable) and
        // folded into the system prompt prefix by `build_memory_section`. Same
        // gate as the profile snapshot: memory on + not incognito. Empty on the
        // dual-track default (no claims yet) → None → no injection. Dynamic
        // per-turn claim recall is served separately by Active Memory v2.
        let context_pack =
            if long_term_memory_enabled && definition.config.memory.enabled && !incognito {
                let mut scopes = vec![
                    crate::memory::MemoryScope::Global,
                    crate::memory::MemoryScope::Agent {
                        id: agent_id.to_string(),
                    },
                ];
                if let Some(p) = project.as_ref() {
                    scopes.push(crate::memory::MemoryScope::Project { id: p.id.clone() });
                }
                let pack = crate::memory::dreaming::build_context_pack(
                    &scopes,
                    &crate::memory::dreaming::ContextPackOptions::default(),
                );
                if !pack.source_digest.is_empty() {
                    crate::app_debug!(
                        "memory",
                        "context_pack",
                        "context pack: {} pinned claim(s) for agent {}",
                        pack.source_digest.len(),
                        agent_id
                    );
                }
                (!pack.is_empty()).then_some(pack)
            } else {
                None
            };

        // Resolve agent home directory
        let agent_home = crate::paths::agent_home_dir(agent_id)
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        // Single source of truth: session-level dir → project's explicit dir →
        // project's lazily-created default workspace. Editing the project
        // default applies immediately to sessions that haven't overridden it.
        let session_working_dir = session_meta
            .as_ref()
            .and_then(crate::session::effective_working_dir_for_meta);
        let permission_mode = session_meta
            .as_ref()
            .map(|m| m.permission_mode)
            .unwrap_or_default();
        let execution_mode = session_meta
            .as_ref()
            .map(|m| m.execution_mode)
            .unwrap_or_default();
        let workflow_mode = session_meta
            .as_ref()
            .map(|m| m.workflow_mode)
            .unwrap_or_default();
        let channel_info = session_meta.as_ref().and_then(|m| m.channel_info.as_ref());
        let mut static_memory_refs = context_pack
            .as_ref()
            .map(|pack| {
                crate::system_prompt::rendered_pinned_memory_sources(
                    definition.memory_md.as_deref(),
                    definition.global_memory_md.as_deref(),
                    &memory_budget,
                    pack,
                )
                .into_iter()
                .map(|source| super::active_memory::UsedMemoryRef {
                    kind: "claim".to_string(),
                    id: source.claim_id,
                    source_type: source.claim_type,
                    scope: super::static_memory_scope_label(
                        &source.scope_type,
                        source.scope_id.as_deref(),
                    ),
                    origin: match source.section.as_str() {
                        "pinned" => "pinned_memory".to_string(),
                        other => format!("context_pack:{other}"),
                    },
                    role: "injected".to_string(),
                    preview: source.preview,
                    path: None,
                    line: None,
                    col: None,
                    heading_path: None,
                    block_id: None,
                    score: None,
                    confidence: None,
                    salience: None,
                })
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let has_profile_snapshot = profile_snapshot
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        let sqlite_cap = crate::system_prompt::sqlite_memory_budget_after_static_layers(
            definition.memory_md.as_deref(),
            definition.global_memory_md.as_deref(),
            &memory_budget,
            context_pack.as_ref(),
        );
        if (!memory_entries.is_empty() || has_profile_snapshot) && sqlite_cap > 0 {
            let scaled = memory_budget.sqlite_sections.scaled_to(sqlite_cap);
            let summary = crate::memory::sqlite::format_prompt_summary_v2_with_refs(
                &memory_entries,
                &scaled,
                sqlite_cap,
                memory_budget.sqlite_entry_max_chars,
                profile_snapshot.as_deref(),
            );
            if !profile_refs.is_empty() && summary.text.contains("## User Profile") {
                static_memory_refs.extend(profile_refs);
            }
            static_memory_refs.extend(summary.refs.into_iter().map(|source| {
                super::active_memory::UsedMemoryRef {
                    kind: "memory".to_string(),
                    id: source.id.to_string(),
                    source_type: source.memory_type,
                    scope: source.scope,
                    origin: "static_memory".to_string(),
                    role: "injected".to_string(),
                    preview: source.preview,
                    path: None,
                    line: None,
                    col: None,
                    heading_path: None,
                    block_id: None,
                    score: None,
                    confidence: None,
                    salience: None,
                }
            }));
        }

        let prompt = crate::system_prompt::build_with_resolved_session(
            &definition,
            Some(model),
            Some(provider),
            &memory_entries,
            &memory_budget,
            profile_snapshot.as_deref(),
            context_pack.as_ref(),
            agent_home.as_deref(),
            project.as_ref(),
            session_id,
            incognito,
            session_working_dir.as_deref(),
            channel_info,
            permission_mode,
            execution_mode,
            workflow_mode,
            active_goal.as_ref(),
            session_meta.as_ref().map(|meta| meta.sandbox_mode),
        );
        return SystemPromptBuild {
            prompt,
            static_memory_refs,
        };
    }
    // Fallback: legacy prompt
    SystemPromptBuild {
        prompt: crate::system_prompt::build_legacy(Some(model), Some(provider), incognito),
        static_memory_refs: Vec::new(),
    }
}

fn resolve_prompt_session_state(
    session_id: Option<&str>,
    session_db: Option<&crate::session::SessionDB>,
) -> (
    Option<crate::session::SessionMeta>,
    Option<crate::goal::GoalSnapshot>,
) {
    let session_meta = session_id.and_then(|sid| {
        session_db.and_then(|db| match db.get_session(sid) {
            Ok(meta) => meta,
            Err(error) => {
                crate::app_warn!(
                    "session",
                    "prompt_session_meta",
                    "bound prompt meta lookup for {} failed: {}",
                    sid,
                    error
                );
                None
            }
        })
    });
    let incognito = session_meta
        .as_ref()
        .map(|session| session.incognito)
        .unwrap_or(session_id.is_some() && session_db.is_some());
    let active_goal = if incognito {
        None
    } else {
        session_id.and_then(|sid| {
            session_db
                .and_then(|db| db.active_goal_for_session(sid).ok())
                .flatten()
        })
    };
    (session_meta, active_goal)
}

#[cfg(test)]
mod build_api_url_tests {
    use super::{build_api_url, is_complete_endpoint_url, resolve_prompt_session_state};

    #[test]
    fn prompt_session_state_reads_bound_database_goal() {
        let dir = tempfile::tempdir().expect("temp session db dir");
        let db = std::sync::Arc::new(
            crate::session::SessionDB::open(&dir.path().join("sessions.db"))
                .expect("open isolated session db"),
        );
        crate::channel::ChannelDB::new(db.clone())
            .migrate()
            .expect("migrate channel tables");
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Bound database objective".to_string(),
                completion_criteria: "Bound database criterion".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create isolated goal");

        let (meta, active_goal) =
            resolve_prompt_session_state(Some(&session.id), Some(db.as_ref()));

        assert_eq!(meta.expect("bound session meta").id, session.id);
        assert_eq!(
            active_goal.expect("bound active goal").goal.id,
            goal.goal.id
        );
    }

    #[test]
    fn plain_host_appends_full_path() {
        assert_eq!(
            build_api_url("https://api.openai.com", "/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn v1_suffix_strips_from_path() {
        assert_eq!(
            build_api_url("https://api.openai.com/v1", "/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn trailing_slash_is_trimmed() {
        assert_eq!(
            build_api_url("https://api.openai.com/v1/", "/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn full_endpoint_is_passthrough_for_matching_path() {
        assert_eq!(
            build_api_url(
                "https://aigc.sankuai.com/v1/openai/native/chat/completions",
                "/v1/chat/completions"
            ),
            "https://aigc.sankuai.com/v1/openai/native/chat/completions"
        );
        assert_eq!(
            build_api_url("https://host/custom/responses", "/v1/responses"),
            "https://host/custom/responses"
        );
        assert_eq!(
            build_api_url("https://host/proxy/messages", "/v1/messages"),
            "https://host/proxy/messages"
        );
    }

    #[test]
    fn full_endpoint_rebuilds_other_paths_from_parent_base() {
        assert_eq!(
            build_api_url("https://api.openai.com/v1/responses", "/v1/models"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            build_api_url(
                "https://api.openai.com/v1/responses",
                "/v1/chat/completions"
            ),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            build_api_url(
                "https://host/v1/openai/native/chat/completions",
                "/v1/models"
            ),
            "https://host/v1/openai/native/v1/models"
        );
    }

    #[test]
    fn complete_endpoint_detection_matches_supported_suffixes() {
        assert!(is_complete_endpoint_url(
            "https://gateway/v1/openai/native/chat/completions"
        ));
        assert!(is_complete_endpoint_url(
            "https://gateway/v1/openai/native/responses"
        ));
        assert!(is_complete_endpoint_url("https://gateway/v1/messages"));
        assert!(!is_complete_endpoint_url("https://gateway/v1"));
    }
}
