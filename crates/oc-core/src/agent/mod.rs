mod api_types;
mod config;
mod content;
mod context;
mod errors;
mod events;
mod providers;
mod side_query;
mod types;

// Re-export public API
pub use config::build_system_prompt;
pub use config::{build_api_url, get_codex_models, USER_AGENT};
pub use types::{AssistantAgent, Attachment, CodexModel, LlmProvider, PlanAgentMode};

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::provider::{ApiType, ProviderConfig, ThinkingStyle};
use crate::tools;

use config::{ANTHROPIC_API_URL, ANTHROPIC_MODEL};
use types::LlmProvider::*;

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
            token_calibrator: std::sync::Mutex::new(
                crate::context_compact::TokenEstimateCalibrator::new(),
            ),
            web_search_enabled: true,
            notification_enabled: false,
            image_gen_config: None,
            canvas_enabled: false,
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            skill_allowed_tools: Vec::new(),
            plan_agent_mode: types::PlanAgentMode::Off,
            plan_mode_allow_paths: Vec::new(),
            temperature: None,
            cache_safe_params: std::sync::Mutex::new(None),
            last_extraction_at: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(3600),
            ),
            tokens_since_extraction: std::sync::atomic::AtomicU32::new(0),
            messages_since_extraction: std::sync::atomic::AtomicU32::new(0),
            manual_memory_saved: std::sync::atomic::AtomicBool::new(false),
            auto_approve_tools: false,
            last_tier2_compaction_at: std::sync::Mutex::new(None),
            agent_caps_cache: std::sync::Mutex::new(None),
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
            token_calibrator: std::sync::Mutex::new(
                crate::context_compact::TokenEstimateCalibrator::new(),
            ),
            web_search_enabled: true,
            notification_enabled: false,
            image_gen_config: None,
            canvas_enabled: false,
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            skill_allowed_tools: Vec::new(),
            plan_agent_mode: types::PlanAgentMode::Off,
            plan_mode_allow_paths: Vec::new(),
            temperature: None,
            cache_safe_params: std::sync::Mutex::new(None),
            last_extraction_at: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(3600),
            ),
            tokens_since_extraction: std::sync::atomic::AtomicU32::new(0),
            messages_since_extraction: std::sync::atomic::AtomicU32::new(0),
            manual_memory_saved: std::sync::atomic::AtomicBool::new(false),
            auto_approve_tools: false,
            last_tier2_compaction_at: std::sync::Mutex::new(None),
            agent_caps_cache: std::sync::Mutex::new(None),
        }
    }

    /// Create agent from a ProviderConfig and a specific model ID
    pub fn new_from_provider(config: &ProviderConfig, model_id: &str) -> Self {
        let provider = match config.api_type {
            ApiType::Anthropic => LlmProvider::Anthropic {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiChat => LlmProvider::OpenAIChat {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::OpenaiResponses => LlmProvider::OpenAIResponses {
                api_key: config.api_key.clone(),
                base_url: config.base_url.clone(),
                model: model_id.to_string(),
            },
            ApiType::Codex => LlmProvider::Codex {
                access_token: config.api_key.clone(),
                account_id: String::new(),
                model: model_id.to_string(),
            },
        };
        // Look up context_window from the provider's model config
        let context_window = config
            .models
            .iter()
            .find(|m| m.id == model_id)
            .map(|m| m.context_window)
            .unwrap_or(200_000);

        Self {
            provider,
            user_agent: config.user_agent.clone(),
            thinking_style: config.thinking_style.clone(),
            conversation_history: std::sync::Mutex::new(Vec::new()),
            agent_id: "default".to_string(),
            extra_system_context: None,
            context_window,
            compact_config: crate::context_compact::CompactConfig::default(),
            token_calibrator: std::sync::Mutex::new(
                crate::context_compact::TokenEstimateCalibrator::new(),
            ),
            web_search_enabled: true,
            notification_enabled: false,
            image_gen_config: None,
            canvas_enabled: false,
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            skill_allowed_tools: Vec::new(),
            plan_agent_mode: types::PlanAgentMode::Off,
            plan_mode_allow_paths: Vec::new(),
            temperature: None,
            cache_safe_params: std::sync::Mutex::new(None),
            last_extraction_at: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(3600),
            ),
            tokens_since_extraction: std::sync::atomic::AtomicU32::new(0),
            messages_since_extraction: std::sync::atomic::AtomicU32::new(0),
            manual_memory_saved: std::sync::atomic::AtomicBool::new(false),
            auto_approve_tools: false,
            last_tier2_compaction_at: std::sync::Mutex::new(None),
            agent_caps_cache: std::sync::Mutex::new(None),
        }
    }

    /// Reset per-chat-round flags. Called at the start of each chat() dispatch.
    pub(crate) fn reset_chat_flags(&self) {
        self.manual_memory_saved
            .store(false, std::sync::atomic::Ordering::SeqCst);
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
            .map(|def| types::AgentCapsCache {
                agent_tool_filter: def.config.capabilities.tools.clone(),
                require_approval_base: def.config.capabilities.require_approval.clone(),
                sandbox: def.config.capabilities.sandbox,
                subagents_enabled: def.config.subagents.enabled,
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

    /// Enable or disable the web_search tool for this agent.
    pub fn set_web_search_enabled(&mut self, enabled: bool) {
        self.web_search_enabled = enabled;
    }

    /// Enable or disable the send_notification tool for this agent.
    pub fn set_notification_enabled(&mut self, enabled: bool) {
        self.notification_enabled = enabled;
    }

    /// Set image generation config (Some = enabled with dynamic tool description).
    pub fn set_image_generate_config(
        &mut self,
        config: Option<crate::tools::image_generate::ImageGenConfig>,
    ) {
        self.image_gen_config = config;
    }

    /// Enable or disable the canvas tool for this agent.
    pub fn set_canvas_enabled(&mut self, enabled: bool) {
        self.canvas_enabled = enabled;
    }

    /// Set the current session ID (for sub-agent context propagation).
    pub fn set_session_id(&mut self, id: &str) {
        self.session_id = Some(id.to_string());
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

    /// Set the plan agent mode (Plan Agent / Executing Agent / Off).
    pub fn set_plan_agent_mode(&mut self, mode: types::PlanAgentMode) {
        self.plan_agent_mode = mode;
    }

    /// Set plan mode path-based allow rules for fine-grained write/edit permission.
    pub fn set_plan_mode_allow_paths(&mut self, paths: Vec<String>) {
        self.plan_mode_allow_paths = paths;
    }

    /// Set temperature for LLM API calls (0.0–2.0). None = use API default.
    pub fn set_temperature(&mut self, temp: Option<f64>) {
        self.temperature = temp;
    }

    /// Set auto-approve mode for all tool calls (used by IM channel auto-approve).
    pub fn set_auto_approve_tools(&mut self, enabled: bool) {
        self.auto_approve_tools = enabled;
    }

    /// Record that a Tier 2+ compaction just happened (resets cache-TTL timer).
    pub fn touch_compaction_timer(&self) {
        *self
            .last_tier2_compaction_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    }

    /// Apply plan-mode tool modifications to a tool schema list.
    /// Called by each provider to inject/filter plan tools without code duplication.
    pub(crate) fn apply_plan_tools(
        &self,
        tool_schemas: &mut Vec<serde_json::Value>,
        provider: tools::ToolProvider,
    ) {
        match &self.plan_agent_mode {
            types::PlanAgentMode::PlanAgent { allowed_tools, .. } => {
                // ask_user_question is now a core/always-loaded tool (injected via
                // get_available_tools), so we only need to add the plan-specific
                // submit tool here. Plan-mode allow-list still controls visibility.
                tool_schemas.push(tools::get_submit_plan_tool().to_provider_schema(provider));
                // Filter to allow-list only
                tool_schemas.retain(|t| {
                    let name = extract_tool_name(t);
                    allowed_tools.iter().any(|a| a == name)
                });
            }
            types::PlanAgentMode::ExecutingAgent { extra_tools } => {
                // Add extra plan execution tools
                for tool_name in extra_tools {
                    match tool_name.as_str() {
                        "update_plan_step" => tool_schemas
                            .push(tools::get_plan_step_tool().to_provider_schema(provider)),
                        "amend_plan" => tool_schemas
                            .push(tools::get_amend_plan_tool().to_provider_schema(provider)),
                        _ => {}
                    }
                }
            }
            types::PlanAgentMode::Off => {}
        }
    }

    /// Build complete tool schema list for a provider, handling:
    /// - Deferred vs full loading
    /// - Conditional tool injection (web_search, notification, image_gen, canvas, subagent)
    /// - Plan mode tool injection/filtering
    /// - Denied tools filtering (depth-based policy)
    /// - Skill allowed-tools filtering
    pub(crate) fn build_tool_schemas(
        &self,
        provider: tools::ToolProvider,
    ) -> Vec<serde_json::Value> {
        let deferred_enabled = crate::config::cached_config().deferred_tools.enabled;
        let caps = self.agent_caps();
        let agent_tool_filter = &caps.agent_tool_filter;

        let mut schemas = if deferred_enabled {
            let mut s = tools::get_core_tools_for_provider(provider);
            s.push(tools::get_tool_search_tool().to_provider_schema(provider));
            if self.subagent_tool_enabled() {
                s.push(tools::get_subagent_tool().to_provider_schema(provider));
            }
            s
        } else {
            let mut s = tools::get_tools_for_provider(provider);
            if self.web_search_enabled {
                s.push(tools::get_web_search_tool().to_provider_schema(provider));
            }
            if self.notification_enabled {
                s.push(tools::get_notification_tool().to_provider_schema(provider));
            }
            if let Some(ref img_config) = self.image_gen_config {
                s.push(
                    tools::get_image_generate_tool_dynamic(img_config).to_provider_schema(provider),
                );
            }
            if self.canvas_enabled {
                s.push(tools::get_canvas_tool().to_provider_schema(provider));
            }
            if self.subagent_tool_enabled() {
                s.push(tools::get_subagent_tool().to_provider_schema(provider));
            }
            s
        };

        // Plan Agent / Executing Agent tool injection
        self.apply_plan_tools(&mut schemas, provider);

        let plan_allowed_tools: &[String] = match &self.plan_agent_mode {
            types::PlanAgentMode::PlanAgent { allowed_tools, .. } => allowed_tools,
            _ => &[],
        };

        schemas.retain(|t| {
            let name = extract_tool_name(t);
            tools::tool_visible_with_filters(
                name,
                agent_tool_filter,
                &self.denied_tools,
                &self.skill_allowed_tools,
                plan_allowed_tools,
            )
        });

        schemas
    }

    /// Build the full system prompt, including any extra context.
    pub(crate) fn build_full_system_prompt(&self, model: &str, provider: &str) -> String {
        let mut prompt = config::build_system_prompt(&self.agent_id, model, provider);
        if self.notification_enabled {
            prompt.push_str("\n\n- **send_notification**: Send a native desktop notification to alert the user about important events, task completions, or findings that need their attention. Parameters: title (optional), body (required).");
        }
        if self.image_gen_config.is_some() {
            prompt.push_str("\n\n- **image_generate**: Generate images from text descriptions. Parameters: prompt (required), size (optional, default 1024x1024), n (optional, 1-4), model (optional, default auto with failover). Generated images are saved to disk.");
        }
        if self.canvas_enabled {
            prompt.push_str("\n\n# Canvas\n\nYou have a `canvas` tool for creating interactive visual content rendered in a preview panel visible to the user.\n\n## Content Types\n- **html**: Full HTML/CSS/JS — web apps, games, animations, interactive demos\n- **markdown**: Rich documents with live preview\n- **code**: Syntax-highlighted code with line numbers\n- **svg**: Scalable vector graphics\n- **mermaid**: Diagrams (flowchart, sequence, class, gantt, etc.)\n- **chart**: Data visualizations (Chart.js JSON config in `content` field)\n- **slides**: Presentation slides (HTML `<section>` tags, arrow key navigation)\n\n## Workflow\n1. `canvas(action=\"create\", content_type=\"html\", title=\"...\", html=\"...\", css=\"...\", js=\"...\")` — create project\n2. Content appears in the user's preview panel immediately\n3. `canvas(action=\"snapshot\", project_id=\"...\")` — capture screenshot to verify visual output\n4. `canvas(action=\"update\", project_id=\"...\", html=\"...\")` — iterate based on screenshot feedback\n5. `canvas(action=\"export\", project_id=\"...\", format=\"html\")` — export when done\n\n## Best Practices\n- Always use snapshot after create/update to verify the visual result\n- For complex UIs, build incrementally — skeleton first, then add features\n- Use semantic HTML and responsive CSS\n- For charts, use Chart.js config JSON format in the `content` field\n- For slides, use `<section>` tags to separate slides");
        }
        // Collect unconfigured capabilities so the model can suggest enabling them
        let mut unconfigured: Vec<&str> = Vec::new();
        if !self.web_search_enabled {
            unconfigured.push("Web Search — Settings → Tools → Web Search");
        }
        if !self.notification_enabled {
            unconfigured.push("Desktop Notifications — Settings → Tools → Notifications");
        }
        if self.image_gen_config.is_none() {
            unconfigured.push("Image Generation — Settings → Tools → Image Generation");
        }
        if !self.canvas_enabled {
            unconfigured.push("Canvas (interactive visual content) — Settings → Tools → Canvas");
        }
        if !unconfigured.is_empty() {
            prompt.push_str("\n\n# Unconfigured Capabilities\n\nThese features are available but not yet enabled. If relevant to the user's request, suggest they enable it:\n");
            for item in &unconfigured {
                prompt.push_str("- ");
                prompt.push_str(item);
                prompt.push('\n');
            }
        }

        if let Some(extra) = &self.extra_system_context {
            prompt.push_str("\n\n");
            prompt.push_str(extra);
        }
        prompt
    }

    /// Whether the subagent tool should be available for this agent.
    pub(crate) fn subagent_tool_enabled(&self) -> bool {
        if self.subagent_depth >= crate::subagent::max_depth_for_agent(&self.agent_id) {
            return false;
        }
        self.agent_caps().subagents_enabled
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
        let mut require_approval = caps.require_approval_base.clone();
        // Merge plan agent ask tools (e.g., exec requires approval during planning)
        if let types::PlanAgentMode::PlanAgent { ask_tools, .. } = &self.plan_agent_mode {
            for tool in ask_tools {
                if !require_approval.contains(tool) {
                    require_approval.push(tool.clone());
                }
            }
        }
        let force_sandbox = caps.sandbox;
        tools::ToolExecContext {
            context_window_tokens: Some(self.context_window),
            used_tokens,
            home_dir: self.agent_home(),
            session_id: self.session_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            subagent_depth: self.subagent_depth,
            require_approval,
            agent_tool_filter,
            denied_tools: self.denied_tools.clone(),
            skill_allowed_tools: self.skill_allowed_tools.clone(),
            force_sandbox,
            plan_mode_allow_paths: self.plan_mode_allow_paths.clone(),
            plan_mode_allowed_tools: match &self.plan_agent_mode {
                types::PlanAgentMode::PlanAgent { allowed_tools, .. } => allowed_tools.clone(),
                _ => Vec::new(),
            },
            auto_approve_tools: self.auto_approve_tools,
        }
    }

    /// Build a ToolExecContext without token usage info (backward-compatible wrapper).
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

    /// If LLM memory selection is enabled and enough candidates exist,
    /// use side_query to select only the most relevant memories and replace
    /// the `# Memory` section in the system prompt.
    pub(crate) async fn select_memories_if_needed(
        &self,
        system_prompt: &mut String,
        user_message: &str,
    ) {
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
        on_delta: impl Fn(&str) + Send + 'static,
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
