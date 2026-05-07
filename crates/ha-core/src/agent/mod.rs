mod active_memory;
pub(super) mod api_types;
mod config;
mod content;
mod context;
mod errors;
mod event_rewrite;
mod events;

pub use event_rewrite::{rewrite_envelope_event_for_http, rewrite_event_for_http};
pub(crate) use events::{extract_media_items, MEDIA_ITEMS_PREFIX};
mod llm_adapter;
mod plan_context;
mod providers;
pub mod resolver;
mod side_query;
mod streaming_adapter;
mod streaming_loop;
mod types;

// Re-export public API
pub use config::{
    build_api_url, get_codex_models, is_complete_endpoint_url, is_valid_codex_model,
    is_valid_reasoning_effort, live_reasoning_effort, USER_AGENT, VALID_REASONING_EFFORTS,
};
pub use config::{build_system_prompt, build_system_prompt_with_session};
pub(crate) use context::build_compaction_provider;
pub use plan_context::{
    merge_extra_system_context, resolve_plan_context_for_session, PlanResolvedContext,
};
pub use types::{AssistantAgent, Attachment, CodexModel, LlmProvider, PlanAgentMode};

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::provider::{ApiType, AuthProfile, ProviderConfig, ThinkingStyle};
use crate::tools;

use config::{ANTHROPIC_API_URL, ANTHROPIC_MODEL};
use types::LlmProvider::*;

/// Single source of truth for `PlanModeState → (PlanAgentMode, allow_paths)`.
/// Callers: turn-start snapshot (chat.rs / channel / cron / spawn_plan_subagent)
/// and streaming_loop mid-turn probe.
pub fn plan_agent_mode_for_state(
    state: crate::plan::PlanModeState,
) -> (PlanAgentMode, Vec<String>) {
    match state {
        crate::plan::PlanModeState::Planning | crate::plan::PlanModeState::Review => {
            let cfg = crate::plan::PlanAgentConfig::default_config();
            (
                PlanAgentMode::PlanAgent {
                    allowed_tools: cfg.allowed_tools,
                    ask_tools: cfg.ask_tools,
                },
                cfg.plan_mode_allow_paths,
            )
        }
        crate::plan::PlanModeState::Executing => (PlanAgentMode::ExecutingAgent, Vec::new()),
        crate::plan::PlanModeState::Off | crate::plan::PlanModeState::Completed => {
            (PlanAgentMode::Off, Vec::new())
        }
    }
}

/// Extract tool name from a provider-formatted schema value.
/// Handles both Anthropic format (`{"name": ...}`) and OpenAI format (`{"function": {"name": ...}}`).
fn extract_tool_name(t: &serde_json::Value) -> &str {
    t.get("name")
        .and_then(|v| v.as_str())
        .or_else(|| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("")
}

fn backdate_instant_safely(
    now: std::time::Instant,
    duration: std::time::Duration,
) -> std::time::Instant {
    now.checked_sub(duration).unwrap_or(now)
}

fn initial_last_extraction_at() -> std::time::Instant {
    backdate_instant_safely(
        std::time::Instant::now(),
        std::time::Duration::from_secs(3600),
    )
}

// ── AssistantAgent constructors, setters, and chat dispatcher ─────

impl AssistantAgent {
    /// Create agent with Anthropic API key (legacy, uses default base_url and model)
    #[allow(dead_code)]
    pub fn new_anthropic(api_key: &str) -> Self {
        Self {
            provider: Anthropic {
                api_key: api_key.to_string(),
                base_url: ANTHROPIC_API_URL
                    .trim_end_matches("/v1/messages")
                    .to_string(),
                model: ANTHROPIC_MODEL.to_string(),
            },
            user_agent: USER_AGENT.to_string(),
            thinking_style: ThinkingStyle::Anthropic,
            conversation_history: std::sync::Mutex::new(Vec::new()),
            agent_id: "default".to_string(),
            extra_system_context: None,
            context_window: 200_000,
            compact_config: crate::context_compact::CompactConfig::default(),
            context_engine: std::sync::Arc::new(crate::context_compact::DefaultContextEngine),
            compaction_provider: None,
            token_calibrator: std::sync::Mutex::new(
                crate::context_compact::TokenEstimateCalibrator::new(),
            ),
            session_id: None,
            incognito_cached: std::sync::atomic::AtomicBool::new(false),
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            skill_allowed_tools: Vec::new(),
            plan_state_cached: arc_swap::ArcSwap::from_pointee(crate::plan::PlanModeState::Off),
            plan_agent_mode: arc_swap::ArcSwap::from_pointee(types::PlanAgentMode::Off),
            plan_mode_allow_paths: arc_swap::ArcSwap::from_pointee(Vec::new()),
            plan_extra_context: arc_swap::ArcSwap::from_pointee(None),
            plan_agent_mode_externally_locked: std::sync::atomic::AtomicBool::new(false),
            temperature: None,
            cache_safe_params: std::sync::Mutex::new(None),
            last_extraction_at: std::sync::Mutex::new(initial_last_extraction_at()),
            tokens_since_extraction: std::sync::atomic::AtomicU32::new(0),
            messages_since_extraction: std::sync::atomic::AtomicU32::new(0),
            manual_memory_saved: std::sync::atomic::AtomicBool::new(false),
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            last_tier2_compaction_at: std::sync::Mutex::new(None),
            agent_caps_cache: std::sync::Mutex::new(None),
            awareness: std::sync::Mutex::new(None),
            awareness_suffix: std::sync::Mutex::new(None),
            active_memory_state: std::sync::Arc::new(active_memory::ActiveMemoryState::new()),
            active_memory_suffix: std::sync::Mutex::new(None),
            provider_config: None,
        }
    }

    /// Create agent with OpenAI-compatible access token (Codex OAuth)
    pub fn new_openai(access_token: &str, account_id: &str, model: &str) -> Self {
        Self {
            provider: Codex {
                access_token: access_token.to_string(),
                account_id: account_id.to_string(),
                model: model.to_string(),
            },
            user_agent: USER_AGENT.to_string(),
            thinking_style: ThinkingStyle::Openai,
            conversation_history: std::sync::Mutex::new(Vec::new()),
            agent_id: "default".to_string(),
            extra_system_context: None,
            context_window: 200_000,
            compact_config: crate::context_compact::CompactConfig::default(),
            context_engine: std::sync::Arc::new(crate::context_compact::DefaultContextEngine),
            compaction_provider: None,
            token_calibrator: std::sync::Mutex::new(
                crate::context_compact::TokenEstimateCalibrator::new(),
            ),
            session_id: None,
            incognito_cached: std::sync::atomic::AtomicBool::new(false),
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            skill_allowed_tools: Vec::new(),
            plan_state_cached: arc_swap::ArcSwap::from_pointee(crate::plan::PlanModeState::Off),
            plan_agent_mode: arc_swap::ArcSwap::from_pointee(types::PlanAgentMode::Off),
            plan_mode_allow_paths: arc_swap::ArcSwap::from_pointee(Vec::new()),
            plan_extra_context: arc_swap::ArcSwap::from_pointee(None),
            plan_agent_mode_externally_locked: std::sync::atomic::AtomicBool::new(false),
            temperature: None,
            cache_safe_params: std::sync::Mutex::new(None),
            last_extraction_at: std::sync::Mutex::new(initial_last_extraction_at()),
            tokens_since_extraction: std::sync::atomic::AtomicU32::new(0),
            messages_since_extraction: std::sync::atomic::AtomicU32::new(0),
            manual_memory_saved: std::sync::atomic::AtomicBool::new(false),
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            last_tier2_compaction_at: std::sync::Mutex::new(None),
            agent_caps_cache: std::sync::Mutex::new(None),
            awareness: std::sync::Mutex::new(None),
            awareness_suffix: std::sync::Mutex::new(None),
            active_memory_state: std::sync::Arc::new(active_memory::ActiveMemoryState::new()),
            active_memory_suffix: std::sync::Mutex::new(None),
            provider_config: None,
        }
    }

    /// Create agent from a ProviderConfig and a specific model ID.
    ///
    /// Uses the first effective auth profile for the API key. For explicit
    /// profile selection (e.g. during profile rotation), use
    /// [`new_from_provider_with_profile`].
    ///
    /// This synchronous constructor is intentionally non-Codex only. Codex uses
    /// OAuth and may need an async refresh before each request; use
    /// [`try_new_from_provider`] for code paths that may receive a Codex
    /// provider.
    pub fn new_from_provider(config: &ProviderConfig, model_id: &str) -> Self {
        assert!(
            config.api_type != ApiType::Codex,
            "Codex providers require AssistantAgent::try_new_from_provider"
        );
        let profiles = config.effective_profiles();
        if let Some(profile) = profiles.first() {
            return Self::new_from_provider_with_profile(config, model_id, profile);
        }
        // Fallback for empty-key API-compatible providers.
        let api_key = config.api_key.clone();
        let base_url = config.base_url.clone();
        Self::build_from_key(config, model_id, &api_key, &base_url)
    }

    /// Create agent from a ProviderConfig with a specific auth profile.
    /// The profile's API key and optional base_url override are used.
    pub fn new_from_provider_with_profile(
        config: &ProviderConfig,
        model_id: &str,
        profile: &AuthProfile,
    ) -> Self {
        assert!(
            config.api_type != ApiType::Codex,
            "Codex providers require AssistantAgent::try_new_from_provider_with_profile"
        );
        let api_key = profile.api_key.clone();
        let base_url = config.resolve_base_url(profile).to_string();
        Self::build_from_key(config, model_id, &api_key, &base_url)
    }

    /// Async provider constructor that is safe for every provider type.
    ///
    /// Codex loads and refreshes OAuth credentials from disk instead of reading
    /// the placeholder `api_key` field from config.
    pub async fn try_new_from_provider(config: &ProviderConfig, model_id: &str) -> Result<Self> {
        Self::try_new_from_provider_with_profile(config, model_id, None).await
    }

    /// Async profile-specific provider constructor that is safe for every
    /// provider type. Codex ignores API-key profiles and uses OAuth.
    pub async fn try_new_from_provider_with_profile(
        config: &ProviderConfig,
        model_id: &str,
        profile: Option<&AuthProfile>,
    ) -> Result<Self> {
        Self::try_new_from_provider_with_codex_hint(config, model_id, profile, None).await
    }

    /// Like [`try_new_from_provider_with_profile`] but accepts an in-memory
    /// `(access_token, account_id)` hint that will be used for Codex providers
    /// when present, before falling back to disk via `load_fresh_codex_token`.
    /// Used by the desktop entry point so a valid token cached in memory after
    /// OAuth still works when the on-disk copy could not be written.
    pub async fn try_new_from_provider_with_codex_hint(
        config: &ProviderConfig,
        model_id: &str,
        profile: Option<&AuthProfile>,
        codex_token_hint: Option<(String, String)>,
    ) -> Result<Self> {
        if config.api_type == ApiType::Codex {
            let (access_token, account_id) = match codex_token_hint {
                Some(hint) if !hint.0.is_empty() => hint,
                _ => crate::oauth::load_fresh_codex_token().await?,
            };
            let provider = LlmProvider::Codex {
                access_token,
                account_id,
                model: model_id.to_string(),
            };
            return Ok(Self::build_from_resolved_provider(
                config, model_id, provider,
            ));
        }

        Ok(match profile {
            Some(profile) => Self::new_from_provider_with_profile(config, model_id, profile),
            None => Self::new_from_provider(config, model_id),
        })
    }

    /// Internal: build an AssistantAgent from resolved api_key and base_url.
    fn build_from_key(
        config: &ProviderConfig,
        model_id: &str,
        api_key: &str,
        base_url: &str,
    ) -> Self {
        let provider = match config.api_type {
            ApiType::Anthropic => LlmProvider::Anthropic {
                api_key: api_key.to_string(),
                base_url: base_url.to_string(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiChat => LlmProvider::OpenAIChat {
                api_key: api_key.to_string(),
                base_url: base_url.to_string(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiResponses => LlmProvider::OpenAIResponses {
                api_key: api_key.to_string(),
                base_url: base_url.to_string(),
                model: model_id.to_string(),
            },
            ApiType::Codex => panic!("Codex providers require async OAuth construction"),
        };
        Self::build_from_resolved_provider(config, model_id, provider)
    }

    fn build_from_resolved_provider(
        config: &ProviderConfig,
        model_id: &str,
        provider: LlmProvider,
    ) -> Self {
        // Look up context_window from the provider's model config
        let context_window = config
            .model_config(model_id)
            .map(|m| m.context_window)
            .unwrap_or(200_000);
        let effective_thinking_style = config.effective_thinking_style_for_model(model_id);

        Self {
            provider,
            user_agent: config.user_agent.clone(),
            thinking_style: effective_thinking_style,
            conversation_history: std::sync::Mutex::new(Vec::new()),
            agent_id: "default".to_string(),
            extra_system_context: None,
            context_window,
            compact_config: crate::context_compact::CompactConfig::default(),
            context_engine: std::sync::Arc::new(crate::context_compact::DefaultContextEngine),
            compaction_provider: None,
            token_calibrator: std::sync::Mutex::new(
                crate::context_compact::TokenEstimateCalibrator::new(),
            ),
            session_id: None,
            incognito_cached: std::sync::atomic::AtomicBool::new(false),
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            skill_allowed_tools: Vec::new(),
            plan_state_cached: arc_swap::ArcSwap::from_pointee(crate::plan::PlanModeState::Off),
            plan_agent_mode: arc_swap::ArcSwap::from_pointee(types::PlanAgentMode::Off),
            plan_mode_allow_paths: arc_swap::ArcSwap::from_pointee(Vec::new()),
            plan_extra_context: arc_swap::ArcSwap::from_pointee(None),
            plan_agent_mode_externally_locked: std::sync::atomic::AtomicBool::new(false),
            temperature: None,
            cache_safe_params: std::sync::Mutex::new(None),
            last_extraction_at: std::sync::Mutex::new(initial_last_extraction_at()),
            tokens_since_extraction: std::sync::atomic::AtomicU32::new(0),
            messages_since_extraction: std::sync::atomic::AtomicU32::new(0),
            manual_memory_saved: std::sync::atomic::AtomicBool::new(false),
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            last_tier2_compaction_at: std::sync::Mutex::new(None),
            agent_caps_cache: std::sync::Mutex::new(None),
            awareness: std::sync::Mutex::new(None),
            awareness_suffix: std::sync::Mutex::new(None),
            active_memory_state: std::sync::Arc::new(active_memory::ActiveMemoryState::new()),
            active_memory_suffix: std::sync::Mutex::new(None),
            provider_config: None,
        }
    }

    /// Inject the source `ProviderConfig` so `side_query` and the Tier 3
    /// `DedicatedModelProvider` can route through `failover::execute_with_failover`
    /// for profile rotation + retry. Without this, those paths fall back to a
    /// single direct one-shot call (legacy behavior).
    ///
    /// Internally wraps the config in `Arc` so callers don't have to. Pass a
    /// borrow; the one clone happens here, once per agent build.
    pub(crate) fn with_failover_context(mut self, provider_config: &ProviderConfig) -> Self {
        self.provider_config = Some(std::sync::Arc::new(provider_config.clone()));
        self
    }

    /// Reset per-chat-round flags. Called at the start of each chat() dispatch.
    pub(crate) fn reset_chat_flags(&self) {
        self.manual_memory_saved
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.refresh_incognito_cache();
        // Record user activity so the Dreaming idle trigger has a fresh
        // "last activity" timestamp. Must be cheap — it's just an atomic store.
        crate::memory::dreaming::touch_activity();
    }

    /// Reload `sessions.incognito` once and store it in the agent-local atomic
    /// so per-turn hot paths (awareness / active memory / memory selection)
    /// can read the flag without hitting SQLite every time. Safe no-op when
    /// `session_id` is `None`.
    fn refresh_incognito_cache(&self) {
        let incognito = crate::session::is_session_incognito(self.session_id.as_deref());
        self.incognito_cached
            .store(incognito, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if any tool call in this round was a manual memory write
    /// (save_memory / update_core_memory). If so, set the mutual exclusion
    /// flag to skip auto-extraction for this round.
    pub(crate) fn check_manual_memory_save(&self, tool_calls: &[api_types::FunctionCallItem]) {
        if tool_calls.iter().any(|tc| {
            tc.name == crate::tools::TOOL_SAVE_MEMORY
                || tc.name == crate::tools::TOOL_UPDATE_CORE_MEMORY
        }) {
            self.manual_memory_saved
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Accumulate token and message counts for extraction threshold tracking.
    pub(crate) fn accumulate_extraction_stats(&self, tokens: u32, messages: u32) {
        self.tokens_since_extraction
            .fetch_add(tokens, std::sync::atomic::Ordering::SeqCst);
        self.messages_since_extraction
            .fetch_add(messages, std::sync::atomic::Ordering::SeqCst);
    }

    /// Reset extraction tracking state after a successful extraction.
    pub(crate) fn reset_extraction_tracking(&self) {
        if let Ok(mut t) = self.last_extraction_at.lock() {
            *t = std::time::Instant::now();
        }
        self.tokens_since_extraction
            .store(0, std::sync::atomic::Ordering::SeqCst);
        self.messages_since_extraction
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Set the agent ID (for memory context and home directory).
    pub fn set_agent_id(&mut self, id: &str) {
        self.agent_id = id.to_string();
        *self
            .agent_caps_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.active_memory_state.invalidate_config();
    }

    /// Return cached per-session snapshot of the fields used from `agent.json`
    /// on hot paths (`build_tool_schemas`, `tool_context_with_usage`,
    /// `subagent_tool_enabled`). Loads from disk on first call, then reuses
    /// until `set_agent_id` invalidates the cache.
    fn agent_caps(&self) -> std::sync::Arc<types::AgentCapsCache> {
        let mut guard = self
            .agent_caps_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(ref cached) = *guard {
            return cached.clone();
        }
        let caps = crate::agent_loader::load_agent(&self.agent_id)
            .map(|def| {
                let mut capability_toggles = def.config.capabilities.capability_toggles.clone();
                // Legacy `notify_on_complete = Some(false)` (Settings →
                // Notifications → Per-agent override) is the same intent as
                // `capability_toggles.send_notification = Some(false)` — both
                // mean "this agent can't reach the desktop notification
                // channel". The v2 dispatcher only consults the toggle, so
                // collapse the legacy field into it whenever the toggle is
                // unset. Pre-fix this branch was dropped via `let _ =`,
                // making the per-agent off-switch a silent no-op.
                if def.config.notify_on_complete == Some(false)
                    && capability_toggles.send_notification.is_none()
                {
                    capability_toggles.send_notification = Some(false);
                }
                types::AgentCapsCache {
                    agent_tool_filter: def.config.capabilities.tools.clone(),
                    sandbox: def.config.capabilities.sandbox,
                    async_tool_policy: def.config.capabilities.async_tool_policy,
                    mcp_enabled: def.config.capabilities.mcp_enabled,
                    capability_toggles,
                    memory_enabled: def.config.memory.enabled,
                    enable_custom_tool_approval: def
                        .config
                        .capabilities
                        .enable_custom_tool_approval,
                    custom_approval_tools: def.config.capabilities.custom_approval_tools.clone(),
                }
            })
            .unwrap_or_default();
        let arc = std::sync::Arc::new(caps);
        *guard = Some(arc.clone());
        arc
    }

    /// Set extra context to append to the system prompt.
    pub fn set_extra_system_context(&mut self, context: String) {
        self.extra_system_context = Some(context);
    }

    /// Set the current session ID (for sub-agent context propagation).
    pub fn set_session_id(&mut self, id: &str) {
        self.session_id = Some(id.to_string());
        self.refresh_incognito_cache();
        self.init_awareness();
    }

    /// (Re-)initialize behavior awareness based on the current session
    /// id. Safe to call multiple times — the first call registers an
    /// observer, subsequent calls replace the Arc and re-register.
    fn init_awareness(&self) {
        let Some(sid) = self.session_id.as_deref() else {
            return;
        };
        if self.session_is_incognito() {
            let mut slot = self.awareness.lock().unwrap_or_else(|e| e.into_inner());
            *slot = None;
            return;
        }
        let Some(db) = crate::get_session_db() else {
            return;
        };
        let cfg = crate::awareness::resolve_for_session(sid, &db);
        let aware =
            crate::awareness::SessionAwareness::new(sid.to_string(), self.agent_id.clone(), cfg);
        let mut slot = self.awareness.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(aware);
    }

    fn session_is_incognito(&self) -> bool {
        self.incognito_cached
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Return the currently-held Active Memory suffix (if any). Provider
    /// layer calls this when constructing the request to inject the recall
    /// sentence as another independent cache block.
    pub(crate) fn current_active_memory_suffix(&self) -> Option<std::sync::Arc<String>> {
        self.active_memory_suffix
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Refresh the Active Memory suffix for this user turn (Phase B1).
    ///
    /// Called at the top of every provider `chat_*` method, right after
    /// `refresh_awareness_suffix`. Runs a bounded side_query that
    /// distills the most relevant memory for `user_text` into a single
    /// sentence. Degrades silently to no-injection on:
    /// - config disabled
    /// - empty shortlist (no candidates matched)
    /// - side_query timeout / error
    /// - LLM returned "NONE" or empty string
    ///
    /// Never blocks the chat loop longer than `active_memory.timeout_ms`.
    pub(crate) async fn refresh_active_memory_suffix(&self, user_text: &str) {
        use std::time::Duration;

        if self.session_is_incognito() {
            *self
                .active_memory_suffix
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }

        // 1. Resolve per-agent memory config. Cached on ActiveMemoryState
        //    so the per-turn hot path doesn't re-read agent.json from
        //    disk; invalidated by `set_agent_id`.
        let snapshot =
            self.active_memory_state.agent_config_or_load(
                || match crate::agent_loader::load_agent(&self.agent_id) {
                    Ok(def) => active_memory::CachedAgentConfig {
                        active_memory: def.config.memory.active_memory.clone(),
                        shared_global: def.config.memory.shared,
                    },
                    Err(_) => active_memory::CachedAgentConfig {
                        active_memory: crate::agent_config::ActiveMemoryConfig::default(),
                        shared_global: true,
                    },
                },
            );
        let cfg = snapshot.active_memory;
        let shared_global = snapshot.shared_global;
        if !cfg.enabled {
            // Clear any stale suffix from a previous enabled turn.
            *self
                .active_memory_suffix
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }

        let Some(sid) = self.session_id.clone() else {
            return;
        };
        let trimmed = user_text.trim();
        if trimmed.is_empty() {
            return;
        }

        // 2. Cache check — if we already recalled for this exact phrasing
        //    within the TTL window, reuse without another LLM call.
        let hash = active_memory::hash_user_text(trimmed);
        let ttl = Duration::from_secs(cfg.cache_ttl_secs.max(1));
        if let Some(cached) = self.active_memory_state.get_cached(hash, ttl) {
            let suffix_arc = cached
                .as_deref()
                .map(|text| std::sync::Arc::new(active_memory::format_suffix(text)));
            *self
                .active_memory_suffix
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = suffix_arc;
            return;
        }

        // 3. Shortlist candidates via the local memory backend. Synchronous
        //    backend call wrapped in spawn_blocking so SQLite / vector work
        //    doesn't stall the runtime.
        let agent_id = self.agent_id.clone();
        let sid_for_search = sid.clone();
        let query = trimmed.to_string();
        let limit = cfg.candidate_limit.max(1);

        let candidates = tokio::task::spawn_blocking(move || {
            let scopes =
                active_memory::scopes_for_session(&sid_for_search, &agent_id, shared_global);
            active_memory::shortlist_candidates(&query, &scopes, limit)
        })
        .await
        .unwrap_or_default();

        if candidates.is_empty() {
            // Cache the empty decision so we don't re-search for the same
            // text until the TTL expires.
            self.active_memory_state.put_cached(hash, None);
            *self
                .active_memory_suffix
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }

        // 4. Bounded side_query — complete or timeout gracefully.
        let prompt = active_memory::build_recall_prompt(trimmed, &candidates, cfg.max_chars);
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_millis(cfg.timeout_ms),
            self.side_query(&prompt, cfg.budget_tokens),
        )
        .await;

        let recalled: Option<String> = match result {
            Ok(Ok(res)) => {
                let trimmed_out = res.text.trim();
                if trimmed_out.is_empty()
                    || trimmed_out.eq_ignore_ascii_case("NONE")
                    || trimmed_out.eq_ignore_ascii_case("NONE.")
                {
                    None
                } else {
                    // Enforce the configured max_chars bound, defensively.
                    Some(crate::truncate_utf8(trimmed_out, cfg.max_chars).to_string())
                }
            }
            Ok(Err(e)) => {
                app_warn!(
                    "agent",
                    "active_memory",
                    "side_query failed: {} ({} candidates, {}ms)",
                    e,
                    candidates.len(),
                    started.elapsed().as_millis()
                );
                None
            }
            Err(_elapsed) => {
                app_warn!(
                    "agent",
                    "active_memory",
                    "side_query timed out after {}ms ({} candidates)",
                    cfg.timeout_ms,
                    candidates.len()
                );
                None
            }
        };

        // 5. Cache the outcome (including None) and update the suffix slot.
        self.active_memory_state.put_cached(hash, recalled.clone());

        let suffix_arc = recalled
            .as_deref()
            .map(|text| std::sync::Arc::new(active_memory::format_suffix(text)));

        if let Some(ref _arc) = suffix_arc {
            app_info!(
                "agent",
                "active_memory",
                "recalled (len={}) from {} candidates in {}ms",
                recalled.as_deref().map(|s| s.len()).unwrap_or(0),
                candidates.len(),
                started.elapsed().as_millis()
            );
        }

        *self
            .active_memory_suffix
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = suffix_arc;
    }

    /// Return the currently-held awareness suffix (if any), for use by
    /// provider-layer code that needs to inject it as a second system block.
    pub(crate) fn current_awareness_suffix(&self) -> Option<std::sync::Arc<String>> {
        self.awareness_suffix
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Run the dynamic awareness refresh for this turn. Called at the
    /// beginning of every provider `chat_*` method before building the system
    /// prompt. Cheap when nothing changed; runs bounded LLM extraction inline
    /// when `mode == LlmDigest` and throttle allows.
    pub(crate) async fn refresh_awareness_suffix(&self, user_text: &str) {
        if self.session_is_incognito() {
            *self
                .awareness_suffix
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }
        let Some(sid) = self.session_id.clone() else {
            return;
        };
        // 1. Broadcast dirty bit to peer sessions.
        crate::awareness::on_other_session_activity(&sid);
        // 2. Lazy init.
        let aware = {
            let slot = self
                .awareness
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if slot.is_some() {
                slot
            } else {
                self.init_awareness();
                self.awareness_arc()
            }
        };
        let Some(aware) = aware else {
            return;
        };
        // 3. Maybe run LLM extraction inline (bounded) BEFORE the first suffix
        //    build so the resulting digest lands in this turn's suffix.
        if aware.should_run_extraction() && aware.claim_extraction() {
            self.run_extraction_inline(&aware, user_text).await;
        }
        // 4. Build suffix.
        let Some(db) = crate::get_session_db() else {
            return;
        };
        let suffix = aware.prepare_dynamic_suffix(user_text, &db);
        let mut slot = self
            .awareness_suffix
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = suffix;
    }

    /// Execute an LLM digest extraction synchronously with a hard timeout.
    /// Called only when `should_run_extraction()` returned true and we
    /// successfully claimed the in-flight lock.
    async fn run_extraction_inline(
        &self,
        aware: &std::sync::Arc<crate::awareness::SessionAwareness>,
        user_text: &str,
    ) {
        use std::time::Duration;
        const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(5);

        // Drop guard: ensure digest_inflight is released even if we panic.
        // On normal paths the explicit record_digest_failure / set_last_digest
        // calls make this redundant but harmless (idempotent).
        struct InflightGuard(std::sync::Arc<crate::awareness::SessionAwareness>);
        impl Drop for InflightGuard {
            fn drop(&mut self) {
                self.0.record_digest_failure();
            }
        }
        let _guard = InflightGuard(std::sync::Arc::clone(aware));

        let cfg = {
            let guard = aware.cfg.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        // If a custom extraction_agent is configured that differs from the
        // current agent, we cannot switch providers inline. Fall back to
        // structured mode with a one-time warning.
        if let Some(ref ea) = cfg.llm_extraction.extraction_agent {
            if ea != &self.agent_id {
                app_info!(
                    "awareness",
                    "run_extraction_inline",
                    "extraction_agent '{}' differs from current agent '{}'; \
                     using current agent for extraction (override not yet supported)",
                    ea,
                    self.agent_id
                );
            }
        }
        let Some(db) = crate::get_session_db() else {
            aware.record_digest_failure();
            return;
        };
        // Collect candidates & compute hash; skip if unchanged.
        let my_agent = Some(self.agent_id.as_str());
        let mut snap = match crate::awareness::collect::collect_entries(
            &db,
            &cfg,
            &self.session_id.clone().unwrap_or_default(),
            my_agent,
        ) {
            Ok(s) if !s.entries.is_empty() => s,
            _ => {
                aware.record_digest_failure();
                return;
            }
        };
        snap.entries.truncate(cfg.llm_extraction.max_candidates);
        let ids: Vec<String> = snap.entries.iter().map(|e| e.session_id.clone()).collect();
        let candidates_changed = aware.update_candidate_hash(&ids);
        if !candidates_changed && aware.has_digest() {
            aware.record_digest_failure();
            return;
        }
        // Build prompt.
        let prompt =
            match crate::awareness::llm_digest::build_extraction_prompt(&snap.entries, &cfg, &db) {
                Ok(p) if !p.is_empty() => p,
                _ => {
                    aware.record_digest_failure();
                    return;
                }
            };
        // Append the current user message so the model can compare topics.
        let prompt = if !user_text.is_empty() {
            format!(
                "{}\n\nCurrent conversation's latest user message:\n\"{}\"",
                prompt,
                crate::truncate_utf8(user_text, 500)
            )
        } else {
            prompt
        };
        // Fire side_query with a hard timeout.
        let max_tokens = ((cfg.llm_extraction.digest_max_chars / 3) as u32).clamp(256, 2048);
        let fut = self.side_query(&prompt, max_tokens);
        match tokio::time::timeout(EXTRACTION_TIMEOUT, fut).await {
            Ok(Ok(res)) => {
                let trimmed = res.text.trim();
                if trimmed.is_empty() {
                    aware.record_digest_failure();
                    return;
                }
                let truncated =
                    crate::truncate_utf8(trimmed, cfg.llm_extraction.digest_max_chars).to_string();
                aware.set_last_digest(std::sync::Arc::new(truncated));
            }
            Ok(Err(e)) => {
                app_warn!(
                    "awareness",
                    "refresh_awareness_suffix",
                    "extraction side_query failed: {}",
                    e
                );
                aware.record_digest_failure();
            }
            Err(_) => {
                app_warn!(
                    "awareness",
                    "refresh_awareness_suffix",
                    "extraction timed out after 5s"
                );
                aware.record_digest_failure();
            }
        }
    }

    /// Force-refresh the awareness suffix on the next turn. Called from
    /// `context_compact` after Tier 2+ compaction since the prompt cache has
    /// already been invalidated.
    pub(crate) fn force_refresh_awareness(&self) {
        let aware = self
            .awareness
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(a) = aware {
            a.mark_force_refresh();
        }
    }

    /// Return the currently held `SessionAwareness` for this agent, if any.
    fn awareness_arc(&self) -> Option<std::sync::Arc<crate::awareness::SessionAwareness>> {
        self.awareness
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Set the sub-agent nesting depth.
    pub fn set_subagent_depth(&mut self, depth: u32) {
        self.subagent_depth = depth;
    }

    /// Set the run ID for steer mailbox (only used when running as a sub-agent).
    pub fn set_steer_run_id(&mut self, run_id: String) {
        self.steer_run_id = Some(run_id);
    }

    /// Get the current denied tools list.
    pub fn get_denied_tools(&self) -> &[String] {
        &self.denied_tools
    }

    /// Set tools that are denied for this agent (depth-based tool policy).
    pub fn set_denied_tools(&mut self, tools: Vec<String>) {
        self.denied_tools = tools;
    }

    /// Set skill-level allowed tools: when non-empty, only these tools are sent to the LLM.
    pub fn set_skill_allowed_tools(&mut self, tools: Vec<String>) {
        self.skill_allowed_tools = tools;
    }

    /// Apply a Plan-mode snapshot supplied externally by the spawn caller
    /// (`spawn_plan_subagent` is the only current case). Sets the
    /// "externally locked" flag so the streaming loop's mid-turn probe
    /// won't overwrite this with the (typically `Off`) child-session
    /// backend state.
    ///
    /// `&self`: ArcSwap + AtomicBool give us interior mutability so the
    /// streaming loop (which holds `&self`) can call the from-backend
    /// variant without forcing the entire chat → provider → loop chain
    /// into `&mut`.
    pub fn apply_plan_resolved_external(&self, ctx: plan_context::PlanResolvedContext) {
        self.write_plan_slots(ctx);
        self.plan_agent_mode_externally_locked
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Apply a Plan-mode snapshot derived from this session's backend plan
    /// state. Used by chat_engine at turn start (chat.rs / channel / cron /
    /// HTTP server) and the streaming loop's mid-turn probe. Future probes
    /// stay free to update — the externally-locked flag stays cleared.
    pub fn apply_plan_resolved_from_backend(&self, ctx: plan_context::PlanResolvedContext) {
        self.write_plan_slots(ctx);
        self.plan_agent_mode_externally_locked
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Atomic 4-slot write (state + mode + allow_paths + extra_context).
    /// Caller picks the locked / unlocked variant above; this helper just
    /// fans the bundle out to the individual ArcSwaps. Stores happen in
    /// the order a future reader is most sensitive to: `state` last so a
    /// mid-turn probe that races a write either sees the old snapshot
    /// in full or notices the new state on its next iteration.
    fn write_plan_slots(&self, ctx: plan_context::PlanResolvedContext) {
        self.plan_agent_mode.store(std::sync::Arc::new(ctx.mode));
        self.plan_mode_allow_paths
            .store(std::sync::Arc::new(ctx.allow_paths));
        self.plan_extra_context
            .store(std::sync::Arc::new(ctx.extra_system_context));
        self.plan_state_cached.store(std::sync::Arc::new(ctx.state));
    }

    /// Snapshot of the current Plan-mode. Returns an owned `Arc` so the
    /// caller can hold it across `await` points without keeping the
    /// `ArcSwap` guard alive (which would block writers).
    pub fn plan_agent_mode(&self) -> std::sync::Arc<types::PlanAgentMode> {
        self.plan_agent_mode.load_full()
    }

    /// Snapshot of the current plan-mode path allow-list. See
    /// `plan_agent_mode()` for the `Arc` return rationale.
    pub fn plan_mode_allow_paths(&self) -> std::sync::Arc<Vec<String>> {
        self.plan_mode_allow_paths.load_full()
    }

    /// True when `set_plan_agent_mode_externally` was the last setter to
    /// run. Used by the streaming loop's mid-turn probe to skip overwriting
    /// a spawn-supplied mode (P2 fix: plan subagent's child session has
    /// `plan_mode = Off` in its DB, but the spawn caller explicitly set
    /// `PlanAgent` and that's the source of truth).
    pub fn is_plan_agent_mode_externally_locked(&self) -> bool {
        self.plan_agent_mode_externally_locked
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Snapshot of the current plan-derived system-prompt segment.
    pub fn plan_extra_context(&self) -> std::sync::Arc<Option<String>> {
        self.plan_extra_context.load_full()
    }

    /// Snapshot of the cached `PlanModeState` last applied to this agent.
    /// The streaming loop's mid-turn probe uses this — NOT the derived
    /// `plan_agent_mode` — because `Planning ↔ Review` and `Completed ↔
    /// Off` produce identical mode values but materially different
    /// `extra_system_context` bundles.
    pub fn plan_state_cached(&self) -> crate::plan::PlanModeState {
        **self.plan_state_cached.load()
    }

    /// Re-sync the full Plan-mode bundle from this session's backend
    /// `plan_mode` when (a) the agent isn't externally-locked and (b) the
    /// live `PlanModeState` differs from the cached snapshot. Returns
    /// `true` when an update happened so the streaming loop can rebuild
    /// dependent artifacts (`tool_schemas`, the round's `system_prompt`).
    ///
    /// **State-level diff, NOT mode-level**: `Planning` and `Review` both
    /// map to `PlanAgentMode::PlanAgent { ... }` (identical value), and
    /// `Completed` and `Off` both map to `PlanAgentMode::Off`. A
    /// mode-only check would silently miss `Planning → Review` (`submit_plan`)
    /// and `Completed → Off` (user exits a completed plan), letting the
    /// model continue under the stale Planning prompt and re-submit the
    /// already-submitted plan. We compare on the original
    /// `PlanModeState` so any backend transition triggers a fresh
    /// `resolve_plan_context_for_session`.
    ///
    /// All four plan slots — `state`, `mode`, `allow_paths`,
    /// `extra_system_context` — are written through `apply_plan_resolved_from_backend`
    /// in one shot so a flip Off→Planning (or any same-mode/different-prompt
    /// transition like Planning→Review) installs a coherent contract:
    /// matching tool schema, allow-list paths, AND the right plan-mode
    /// system-prompt segment.
    ///
    /// Called both at round head (catches state changes that happened
    /// between rounds) and before each sequential tool inside a round
    /// (catches the case where an `enter_plan_mode` / `submit_plan`
    /// earlier in the same batch flipped state — without this the
    /// subsequent tools in the batch would run under a stale snapshot).
    pub async fn maybe_resync_plan_mode_from_backend(&self) -> bool {
        if self.is_plan_agent_mode_externally_locked() {
            return false;
        }
        let Some(sid) = self.session_id.as_deref() else {
            return false;
        };
        let live_state = crate::plan::get_plan_state(sid).await;
        let cached_state = self.plan_state_cached();
        if live_state == cached_state {
            return false;
        }
        app_info!(
            "plan",
            "agent",
            "Plan state re-sync for session {}: {:?} → {:?}",
            sid,
            cached_state,
            live_state
        );
        // Single source of truth — pull the full bundle (state, mode,
        // allow_paths, extra_system_context) through the same code path
        // the chat_engine uses at turn start.
        let resolved = plan_context::resolve_plan_context_for_session(sid).await;
        self.apply_plan_resolved_from_backend(resolved);
        true
    }

    /// Set temperature for LLM API calls (0.0–2.0). None = use API default.
    pub fn set_temperature(&mut self, temp: Option<f64>) {
        self.temperature = temp;
    }

    /// Set auto-approve mode for all tool calls (used by IM channel auto-approve).
    pub fn set_auto_approve_tools(&mut self, enabled: bool) {
        self.auto_approve_tools = enabled;
    }

    /// Opt into live reasoning-effort tracking (main chat path only).
    ///
    /// When enabled, each tool-loop round re-reads `AppState.reasoning_effort`
    /// so UI toggles apply to the next API request. Off by default so
    /// subagents / side_query / memory_extract keep their caller-specified
    /// effort even when the user toggles the main chat picker.
    pub fn set_follow_global_reasoning_effort(&mut self, enabled: bool) {
        self.follow_global_reasoning_effort = enabled;
    }

    /// Resolve the reasoning effort string for this round.
    /// Main-chat agents pull the live value from `AppState`; everyone else
    /// keeps the caller-specified fallback so subagents / side_query / cron
    /// aren't silently overridden by the UI picker.
    pub(super) async fn effective_reasoning_effort(
        &self,
        fallback: Option<&str>,
    ) -> Option<String> {
        if self.follow_global_reasoning_effort {
            config::live_reasoning_effort(fallback).await
        } else {
            fallback.map(|s| s.to_string())
        }
    }

    /// Build a Responses/Codex `ReasoningConfig` for this round, clamping to
    /// the model's supported range. Returns `None` when effort is disabled.
    pub(super) async fn resolve_reasoning_config(
        &self,
        model: &str,
        fallback: Option<&str>,
    ) -> Option<api_types::ReasoningConfig> {
        if self.thinking_style == ThinkingStyle::None {
            return None;
        }
        self.effective_reasoning_effort(fallback)
            .await
            .and_then(|e| config::clamp_reasoning_effort(model, &e))
            .map(|effort| api_types::ReasoningConfig {
                effort,
                summary: Some("auto".to_string()),
            })
    }

    /// Record that a Tier 2+ compaction just happened (resets cache-TTL timer).
    pub fn touch_compaction_timer(&self) {
        *self
            .last_tier2_compaction_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    }

    /// Plan-tool injection: filter / extend the schema list according to
    /// the agent's current Plan-mode. Reads `self.plan_agent_mode` via
    /// ArcSwap so `streaming_loop`'s mid-turn `set_plan_agent_mode_from_backend`
    /// is reflected on the very next `build_tool_schemas` call without
    /// any explicit threading.
    pub(crate) fn apply_plan_tools(
        &self,
        tool_schemas: &mut Vec<serde_json::Value>,
        provider: tools::ToolProvider,
    ) {
        let plan_mode = self.plan_agent_mode.load();
        match &**plan_mode {
            types::PlanAgentMode::PlanAgent { allowed_tools, .. } => {
                // ask_user_question is a core/always-loaded tool (injected via
                // get_available_tools), so we only need to add the plan-specific
                // submit tool here. The allow-list filter then drops anything
                // outside the Plan Agent toolset.
                tool_schemas.push(tools::get_submit_plan_tool().to_provider_schema(provider));
                tool_schemas.retain(|t| {
                    let name = extract_tool_name(t);
                    allowed_tools.iter().any(|a| a == name)
                });
            }
            types::PlanAgentMode::ExecutingAgent => {
                // Plan execution adds no extra tools — progress lives in the
                // standard task_create / task_update flow (always-loaded core
                // tools); structural plan changes require re-entering Planning.
            }
            types::PlanAgentMode::Off => {
                // Off (regular session): inject `enter_plan_mode` so the model
                // can proactively suggest entering Plan Mode. The tool itself
                // triggers a user-facing Yes/No prompt and never transitions
                // state on its own — sovereignty stays with the user.
                tool_schemas.push(tools::get_enter_plan_mode_tool().to_provider_schema(provider));
            }
        }
    }

    /// Build complete tool schema list for a provider. Reads
    /// `plan_agent_mode` via ArcSwap, so the streaming loop's mid-turn
    /// `set_plan_agent_mode_from_backend` is observed automatically on the
    /// next call — no `_with_mode` override needed.
    pub(crate) fn build_tool_schemas(
        &self,
        provider: tools::ToolProvider,
    ) -> Vec<serde_json::Value> {
        let app_config = crate::config::cached_config();
        let caps = self.agent_caps();
        let ctx = tools::dispatch::DispatchContext {
            agent_id: self.agent_id.as_str(),
            mcp_enabled: caps.mcp_enabled,
            memory_enabled: caps.memory_enabled,
            tools_filter: &caps.agent_tool_filter,
            capability_toggles: &caps.capability_toggles,
            app_config: &app_config,
        };

        let mut schemas: Vec<serde_json::Value> = Vec::new();

        for def in tools::dispatch::all_dispatchable_tools() {
            if !matches!(
                tools::dispatch::resolve_tool_fate(def, &ctx),
                tools::dispatch::ToolFate::InjectEager
            ) {
                continue;
            }
            let schema = if def.name == tools::TOOL_IMAGE_GENERATE {
                tools::get_image_generate_tool_dynamic(&app_config.image_generate)
                    .to_provider_schema(provider)
            } else {
                def.to_provider_schema(provider)
            };
            schemas.push(schema);
        }

        if !self.subagent_depth_allows_subagent() {
            schemas.retain(|t| extract_tool_name(t) != tools::TOOL_SUBAGENT);
        }

        if caps.mcp_enabled {
            if let Some(mcp) = crate::mcp::McpManager::global() {
                for def in mcp.mcp_tool_definitions().iter() {
                    if crate::mcp::catalog::tool_belongs_to_deferred_server(
                        &def.name,
                        &app_config.mcp_servers,
                    ) {
                        continue;
                    }
                    schemas.push(def.to_provider_schema(provider));
                }
            }
        }

        // Plan Agent / Executing Agent tool injection. apply_plan_tools and
        // the plan-allowed filter below both load `self.plan_agent_mode` via
        // ArcSwap, so they observe the same snapshot as the streaming loop's
        // most recent probe without manual threading.
        self.apply_plan_tools(&mut schemas, provider);

        // Final filter pipeline (skill / denied / plan-allowed) — defense
        // in depth on top of dispatcher visibility.
        let plan_mode = self.plan_agent_mode.load();
        let plan_allowed_tools: &[String] = match &**plan_mode {
            types::PlanAgentMode::PlanAgent { allowed_tools, .. } => allowed_tools,
            _ => &[],
        };
        schemas.retain(|t| {
            let name = extract_tool_name(t);
            tools::tool_visible_with_filters(
                name,
                &caps.agent_tool_filter,
                &self.denied_tools,
                &self.skill_allowed_tools,
                plan_allowed_tools,
            )
        });

        schemas
    }

    /// Whether the current subagent depth permits spawning further sub-agents.
    fn subagent_depth_allows_subagent(&self) -> bool {
        self.subagent_depth < crate::subagent::max_depth_for_agent(&self.agent_id)
    }

    /// Build the full system prompt, including any extra context.
    pub(crate) fn build_full_system_prompt(&self, model: &str, provider: &str) -> String {
        let mut prompt = config::build_system_prompt_with_session(
            &self.agent_id,
            model,
            provider,
            self.session_id.as_deref(),
        );
        // Single walk over the static catalog: classify every tool's fate
        // up front, then drive both the eager-capability guidance blocks
        // and the # Unconfigured Capabilities section from the same map.
        let app_config = crate::config::cached_config();
        let caps = self.agent_caps();
        let ctx = tools::dispatch::DispatchContext {
            agent_id: self.agent_id.as_str(),
            mcp_enabled: caps.mcp_enabled,
            memory_enabled: caps.memory_enabled,
            tools_filter: &caps.agent_tool_filter,
            capability_toggles: &caps.capability_toggles,
            app_config: &app_config,
        };
        let mut eager: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut hints: Vec<String> = Vec::new();
        for t in tools::dispatch::all_dispatchable_tools() {
            match tools::dispatch::resolve_tool_fate(t, &ctx) {
                tools::dispatch::ToolFate::InjectEager => {
                    eager.insert(t.name.as_str());
                }
                tools::dispatch::ToolFate::HintOnly { config_hint } => {
                    hints.push(format!("- {} — {}", t.name, config_hint));
                }
                _ => {}
            }
        }

        if eager.contains(tools::TOOL_SEND_NOTIFICATION) {
            prompt.push_str("\n\n- **send_notification**: Send a native desktop notification to alert the user about important events, task completions, or findings that need their attention. Parameters: title (optional), body (required).");
        }
        if eager.contains(tools::TOOL_IMAGE_GENERATE) {
            prompt.push_str("\n\n- **image_generate**: Generate images from text descriptions. Parameters: prompt (required), size (optional, default 1024x1024), n (optional, 1-4), model (optional, default auto with failover). Generated images are saved to disk.");
        }
        if eager.contains(tools::TOOL_CANVAS) {
            prompt.push_str("\n\n# Canvas\n\nYou have a `canvas` tool for creating interactive visual content rendered in a preview panel visible to the user.\n\n## Content Types\n- **html**: Full HTML/CSS/JS — web apps, games, animations, interactive demos\n- **markdown**: Rich documents with live preview\n- **code**: Syntax-highlighted code with line numbers\n- **svg**: Scalable vector graphics\n- **mermaid**: Diagrams (flowchart, sequence, class, gantt, etc.)\n- **chart**: Data visualizations (Chart.js JSON config in `content` field)\n- **slides**: Presentation slides (HTML `<section>` tags, arrow key navigation)\n\n## Workflow\n1. `canvas(action=\"create\", content_type=\"html\", title=\"...\", html=\"...\", css=\"...\", js=\"...\")` — create project\n2. Content appears in the user's preview panel immediately\n3. `canvas(action=\"snapshot\", project_id=\"...\")` — capture screenshot to verify visual output\n4. `canvas(action=\"update\", project_id=\"...\", html=\"...\")` — iterate based on screenshot feedback\n5. `canvas(action=\"export\", project_id=\"...\", format=\"html\")` — export when done\n\n## Best Practices\n- Always use snapshot after create/update to verify the visual result\n- For complex UIs, build incrementally — skeleton first, then add features\n- Use semantic HTML and responsive CSS\n- For charts, use Chart.js config JSON format in the `content` field\n- For slides, use `<section>` tags to separate slides");
        }

        // Stable ordering for prompt-cache hits.
        hints.sort();
        if !hints.is_empty() {
            prompt.push_str(
                "\n\n# Unconfigured Capabilities\n\n\
                 These features are available but not yet provisioned. If relevant to the \
                 user's request, suggest they enable it:\n",
            );
            for line in &hints {
                prompt.push_str(line);
                prompt.push('\n');
            }
        }

        // Caller-supplied extra context (cron task description, subagent
        // role, etc.) — frames the model's task before any Plan Mode
        // contract.
        if let Some(extra) = &self.extra_system_context {
            prompt.push_str("\n\n");
            prompt.push_str(extra);
        }
        // Plan-derived segment, kept separate so the streaming loop's
        // mid-turn probe can swap it via `set_plan_extra_context`. Reads
        // the ArcSwap so a probe that landed since the last build is
        // observed on the very next system-prompt rebuild.
        if let Some(plan_extra) = &**self.plan_extra_context.load() {
            prompt.push_str("\n\n");
            prompt.push_str(plan_extra);
        }
        // MCP-connected servers advertise capabilities through a small
        // appended section. Suppressed entirely when no MCP server has
        // reached `Ready` — keeps the prompt shape stable for users who
        // don't use MCP.
        if let Some(snippet) = crate::mcp::catalog::system_prompt_snippet() {
            prompt.push_str("\n\n");
            prompt.push_str(&snippet);
        }
        prompt
    }

    /// Build the "static" system prompt — excludes the dynamic awareness
    /// suffix which providers append as a separate cache breakpoint.
    ///
    /// Currently unused but kept as the named dual of
    /// [`Self::build_merged_system_prompt`]; compaction call sites use the
    /// merged form and side-query shortcuts go through `CacheSafeParams`.
    #[allow(dead_code)]
    pub(crate) fn build_static_system_prompt(&self, model: &str, provider: &str) -> String {
        self.build_full_system_prompt(model, provider)
    }

    /// Build the merged system prompt string (static prefix + awareness
    /// suffix). Used for compaction token budgets and any code path that
    /// needs a flat string.
    pub(crate) fn build_merged_system_prompt(&self, model: &str, provider: &str) -> String {
        let mut prompt = self.build_full_system_prompt(model, provider);
        if let Some(suffix) = self.current_awareness_suffix() {
            if !suffix.is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(&suffix);
            }
        }
        prompt
    }

    /// Get the agent's home directory path.
    fn agent_home(&self) -> Option<String> {
        crate::paths::agent_home_dir(&self.agent_id)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    }

    /// Build a ToolExecContext with agent home directory, context window, and
    /// estimated token usage for adaptive tool output sizing.
    pub(crate) fn tool_context_with_usage(
        &self,
        used_tokens: Option<u32>,
    ) -> tools::ToolExecContext {
        let caps = self.agent_caps();
        let agent_tool_filter = caps.agent_tool_filter.clone();
        let force_sandbox = caps.sandbox;
        // Pull working_dir / permission_mode / project_id from a single
        // SessionMeta lookup — avoids 3 separate SQLite roundtrips per
        // tool round.
        let meta = crate::session::lookup_session_meta(self.session_id.as_deref());
        let session_working_dir = meta
            .as_ref()
            .and_then(|m| m.working_dir.clone().filter(|s| !s.trim().is_empty()))
            .or_else(|| {
                // Fall back to the session's project default working_dir.
                let pid = meta.as_ref()?.project_id.as_deref()?;
                crate::get_project_db()?
                    .get(pid)
                    .ok()
                    .flatten()?
                    .working_dir
                    .filter(|s| !s.trim().is_empty())
            });
        let session_mode = meta.as_ref().map(|m| m.permission_mode).unwrap_or_default();
        let project_id = meta.as_ref().and_then(|m| m.project_id.clone());
        tools::ToolExecContext {
            context_window_tokens: Some(self.context_window),
            used_tokens,
            home_dir: self.agent_home(),
            session_working_dir,
            session_id: self.session_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            subagent_depth: self.subagent_depth,
            agent_tool_filter,
            denied_tools: self.denied_tools.clone(),
            skill_allowed_tools: self.skill_allowed_tools.clone(),
            force_sandbox,
            // Load both ArcSwaps once per ctx build so the snapshot is
            // internally consistent with the schema build that just preceded
            // this dispatch (both go through `self.plan_agent_mode` /
            // `self.plan_mode_allow_paths` ArcSwap loads — same data source,
            // no manual threading).
            plan_mode_allow_paths: (**self.plan_mode_allow_paths.load()).clone(),
            plan_mode_allowed_tools: match &**self.plan_agent_mode.load() {
                types::PlanAgentMode::PlanAgent { allowed_tools, .. } => allowed_tools.clone(),
                _ => Vec::new(),
            },
            plan_mode_ask_tools: match &**self.plan_agent_mode.load() {
                types::PlanAgentMode::PlanAgent { ask_tools, .. } => ask_tools.clone(),
                _ => Vec::new(),
            },
            auto_approve_tools: self.auto_approve_tools,
            session_mode,
            agent_custom_approval_enabled: caps.enable_custom_tool_approval,
            agent_custom_approval_tools: caps.custom_approval_tools.clone(),
            project_id,
            async_tool_policy: caps.async_tool_policy,
            bypass_async_dispatch: false,
            metadata_sink: None,
        }
    }

    /// Build a ToolExecContext without token usage info (backward-compatible wrapper).
    #[allow(dead_code)]
    pub(crate) fn tool_context(&self) -> tools::ToolExecContext {
        self.tool_context_with_usage(None)
    }

    /// Get the context window size.
    pub fn get_context_window(&self) -> u32 {
        self.context_window
    }

    /// Set the compact config (called from lib.rs after agent construction).
    pub fn set_compact_config(&mut self, mut config: crate::context_compact::CompactConfig) {
        config.clamp();
        self.compact_config = config;
    }

    /// Replace the context engine (default: `DefaultContextEngine`).
    pub fn set_context_engine(
        &mut self,
        engine: std::sync::Arc<dyn crate::context_compact::ContextEngine>,
    ) {
        self.context_engine = engine;
    }

    /// Access the active context engine.
    pub fn context_engine(&self) -> &dyn crate::context_compact::ContextEngine {
        &*self.context_engine
    }

    /// Replace the compaction provider (dedicated summarization model).
    /// `None` = use default side_query / direct HTTP path.
    pub fn set_compaction_provider(
        &mut self,
        provider: Option<std::sync::Arc<dyn crate::context_compact::CompactionProvider>>,
    ) {
        self.compaction_provider = provider;
    }

    /// Apply the context engine's optional system prompt addition.
    pub(super) fn apply_engine_prompt_addition(&self, system_prompt: &mut String) {
        if let Some(addition) = self.context_engine.system_prompt_addition() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&addition);
        }
    }

    /// Reactive microcompaction invoked between tool loop rounds.
    ///
    /// When estimated usage ratio meets `compact.reactive_trigger_ratio`,
    /// runs Tier 0 microcompaction to clear ephemeral tool results
    /// (ls/grep/find/web_search/... per `tool_policies`), preventing
    /// emergency compaction from firing when tool_result accumulation
    /// would otherwise push context over the limit mid-loop.
    ///
    /// Tier 0 is cache-safe: it only touches content inside existing
    /// tool_result blocks without reordering messages, so the cached
    /// prefix remains valid.
    pub(super) fn reactive_microcompact_in_loop(
        &self,
        messages: &mut [serde_json::Value],
        system_prompt_for_budget: &str,
        max_output_tokens: u32,
    ) {
        let cfg = &self.compact_config;
        if !cfg.enabled || !cfg.reactive_microcompact_enabled {
            return;
        }
        if self.context_window == 0 {
            return;
        }

        let used = crate::context_compact::estimate_request_tokens(
            system_prompt_for_budget,
            messages,
            max_output_tokens,
        );
        let ratio = used as f64 / self.context_window as f64;
        if ratio < cfg.reactive_trigger_ratio {
            return;
        }

        let cleared = crate::context_compact::microcompact(messages, cfg);
        if cleared > 0 {
            app_info!(
                "agent",
                "reactive_microcompact",
                "cleared {} ephemeral tool_results at ratio={:.2} (threshold={:.2})",
                cleared,
                ratio,
                cfg.reactive_trigger_ratio
            );
        }
    }

    /// If LLM memory selection is enabled and enough candidates exist,
    /// use side_query to select only the most relevant memories and replace
    /// the `# Memory` section in the system prompt.
    pub(crate) async fn select_memories_if_needed(
        &self,
        system_prompt: &mut String,
        user_message: &str,
    ) {
        if self.session_is_incognito() {
            return;
        }
        let config = crate::memory::helpers::load_memory_selection_config();
        if !config.enabled {
            return;
        }

        let backend = match crate::get_memory_backend() {
            Some(b) => b,
            None => return,
        };
        let agent_def = crate::agent_loader::load_agent(&self.agent_id).ok();
        let shared = agent_def
            .as_ref()
            .map(|d| d.config.memory.shared)
            .unwrap_or(true);

        let candidates = match backend.load_prompt_candidates(&self.agent_id, shared) {
            Ok(c) => c,
            Err(_) => return,
        };

        if candidates.len() <= config.threshold {
            return;
        }

        // Build compact manifest: (id, first-line preview)
        let manifest: Vec<(i64, String)> = candidates
            .iter()
            .map(|e| {
                let preview = e.content.lines().next().unwrap_or(&e.content);
                let truncated = crate::truncate_utf8(preview, 120);
                (e.id, truncated.to_string())
            })
            .collect();

        let instruction = crate::memory::selection::build_selection_instruction(
            user_message,
            &manifest,
            config.max_selected,
        );

        let result = match self.side_query(&instruction, 1024).await {
            Ok(r) => r,
            Err(e) => {
                app_warn!(
                    "memory",
                    "selection",
                    "LLM memory selection failed, using full set: {}",
                    e
                );
                return;
            }
        };

        let selected_ids = crate::memory::selection::parse_selection_response(&result.text);
        if selected_ids.is_empty() {
            return;
        }

        // Filter candidates to selected IDs (preserve selection order)
        let selected: Vec<crate::memory::MemoryEntry> = selected_ids
            .iter()
            .filter_map(|id| candidates.iter().find(|e| e.id == *id).cloned())
            .collect();

        if selected.is_empty() {
            return;
        }

        let budget = agent_def
            .as_ref()
            .map(|d| d.config.memory.prompt_budget)
            .unwrap_or(5000);
        let new_summary = crate::memory::sqlite::format_prompt_summary(&selected, budget);

        crate::memory::selection::replace_memory_section(system_prompt, &new_summary);

        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info",
                "memory",
                "selection",
                &format!(
                    "LLM memory selection: {} candidates → {} selected, cache_read={}",
                    candidates.len(),
                    selected.len(),
                    result.usage.cache_read_input_tokens,
                ),
                None,
                None,
                None,
            );
        }
    }

    pub async fn chat(
        &self,
        message: &str,
        attachments: &[Attachment],
        reasoning_effort: Option<&str>,
        cancel: Arc<AtomicBool>,
        on_delta: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<(String, Option<String>)> {
        // Log agent chat dispatch
        if let Some(logger) = crate::get_logger() {
            let (provider_type, model_name) = match &self.provider {
                LlmProvider::Anthropic { model, .. } => ("Anthropic", model.as_str()),
                LlmProvider::OpenAIChat { model, .. } => ("OpenAIChat", model.as_str()),
                LlmProvider::OpenAIResponses { model, .. } => ("OpenAIResponses", model.as_str()),
                LlmProvider::Codex { model, .. } => ("Codex", model.as_str()),
            };
            let history_len = self
                .conversation_history
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            let msg_preview = if message.len() > 200 {
                format!("{}...", crate::truncate_utf8(message, 200))
            } else {
                message.to_string()
            };
            logger.log(
                "info",
                "agent",
                "agent::chat",
                &format!(
                    "Agent chat dispatching: provider={}, model={}",
                    provider_type, model_name
                ),
                Some(
                    json!({
                        "provider_type": provider_type,
                        "model": model_name,
                        "reasoning_effort": reasoning_effort,
                        "attachments": attachments.len(),
                        "history_messages": history_len,
                        "message_preview": msg_preview,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        match &self.provider {
            LlmProvider::Anthropic {
                api_key,
                base_url,
                model,
            } => {
                self.chat_anthropic(
                    api_key,
                    base_url,
                    model,
                    message,
                    attachments,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
            }
            LlmProvider::OpenAIChat {
                api_key,
                base_url,
                model,
            } => {
                self.chat_openai_chat(
                    api_key,
                    base_url,
                    model,
                    message,
                    attachments,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
            }
            LlmProvider::OpenAIResponses {
                api_key,
                base_url,
                model,
            } => {
                self.chat_openai_responses(
                    api_key,
                    base_url,
                    model,
                    message,
                    attachments,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
            }
            LlmProvider::Codex {
                access_token,
                account_id,
                model,
            } => {
                self.chat_openai(
                    access_token,
                    account_id,
                    model,
                    message,
                    attachments,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::backdate_instant_safely;

    #[test]
    fn backdate_instant_safely_subtracts_when_duration_fits() {
        let now = Instant::now();
        let earlier = backdate_instant_safely(now, Duration::from_millis(1));

        assert!(now.duration_since(earlier) >= Duration::from_millis(1));
    }

    #[test]
    fn backdate_instant_safely_saturates_when_duration_underflows() {
        let now = Instant::now();

        assert_eq!(backdate_instant_safely(now, Duration::MAX), now);
    }
}
