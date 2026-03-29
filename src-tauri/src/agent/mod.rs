mod api_types;
mod config;
mod content;
mod context;
mod errors;
mod events;
mod providers;
mod types;

// Re-export public API
pub use config::{build_api_url, get_codex_models, USER_AGENT};
pub use types::{Attachment, AssistantAgent, CodexModel, LlmProvider, PlanAgentMode};

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use serde_json::json;

use crate::provider::{ApiType, ProviderConfig, ThinkingStyle};
use crate::tools;

use config::{ANTHROPIC_API_URL, ANTHROPIC_MODEL};
use types::LlmProvider::*;

// ── AssistantAgent constructors, setters, and chat dispatcher ─────

impl AssistantAgent {
    /// Create agent with Anthropic API key (legacy, uses default base_url and model)
    #[allow(dead_code)]
    pub fn new_anthropic(api_key: &str) -> Self {
        Self {
            provider: Anthropic {
                api_key: api_key.to_string(),
                base_url: ANTHROPIC_API_URL.trim_end_matches("/v1/messages").to_string(),
                model: ANTHROPIC_MODEL.to_string(),
            },
            user_agent: USER_AGENT.to_string(),
            thinking_style: ThinkingStyle::Anthropic,
            conversation_history: std::sync::Mutex::new(Vec::new()),
            agent_id: "default".to_string(),
            extra_system_context: None,
            context_window: 200_000,
            compact_config: crate::context_compact::CompactConfig::default(),
            token_calibrator: std::sync::Mutex::new(crate::context_compact::TokenEstimateCalibrator::new()),
            notification_enabled: false,
            image_gen_config: None,
            canvas_enabled: false,
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            plan_agent_mode: types::PlanAgentMode::Off,
            plan_mode_allow_paths: Vec::new(),
            temperature: None,
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
            token_calibrator: std::sync::Mutex::new(crate::context_compact::TokenEstimateCalibrator::new()),
            notification_enabled: false,
            image_gen_config: None,
            canvas_enabled: false,
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            plan_agent_mode: types::PlanAgentMode::Off,
            plan_mode_allow_paths: Vec::new(),
            temperature: None,
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
        let context_window = config.models.iter()
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
            token_calibrator: std::sync::Mutex::new(crate::context_compact::TokenEstimateCalibrator::new()),
            notification_enabled: false,
            image_gen_config: None,
            canvas_enabled: false,
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
            plan_agent_mode: types::PlanAgentMode::Off,
            plan_mode_allow_paths: Vec::new(),
            temperature: None,
        }
    }

    /// Set the agent ID (for memory context and home directory).
    pub fn set_agent_id(&mut self, id: &str) {
        self.agent_id = id.to_string();
    }

    /// Set extra context to append to the system prompt.
    pub fn set_extra_system_context(&mut self, context: String) {
        self.extra_system_context = Some(context);
    }

    /// Enable or disable the send_notification tool for this agent.
    pub fn set_notification_enabled(&mut self, enabled: bool) {
        self.notification_enabled = enabled;
    }

    /// Set image generation config (Some = enabled with dynamic tool description).
    pub fn set_image_generate_config(&mut self, config: Option<crate::tools::image_generate::ImageGenConfig>) {
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

    /// Set the plan agent mode (Plan Agent / Build Agent / Off).
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

    /// Apply plan-mode tool modifications to a tool schema list.
    /// Called by each provider to inject/filter plan tools without code duplication.
    pub(crate) fn apply_plan_tools(&self, tool_schemas: &mut Vec<serde_json::Value>, provider: tools::ToolProvider) {
        match &self.plan_agent_mode {
            types::PlanAgentMode::PlanAgent { allowed_tools, .. } => {
                // Add plan-specific tools
                tool_schemas.push(tools::get_plan_question_tool().to_provider_schema(provider));
                tool_schemas.push(tools::get_submit_plan_tool().to_provider_schema(provider));
                // Filter to allow-list only
                tool_schemas.retain(|t| {
                    let name = t.get("name")
                        .and_then(|v| v.as_str())
                        // OpenAI Responses format: tool.function.name
                        .or_else(|| t.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()))
                        .unwrap_or("");
                    allowed_tools.iter().any(|a| a == name)
                });
            }
            types::PlanAgentMode::BuildAgent { extra_tools } => {
                // Add extra plan execution tools
                for tool_name in extra_tools {
                    match tool_name.as_str() {
                        "update_plan_step" => tool_schemas.push(tools::get_plan_step_tool().to_provider_schema(provider)),
                        "amend_plan" => tool_schemas.push(tools::get_amend_plan_tool().to_provider_schema(provider)),
                        _ => {}
                    }
                }
            }
            types::PlanAgentMode::Off => {}
        }
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
        crate::agent_loader::load_agent(&self.agent_id)
            .map(|def| def.config.subagents.enabled)
            .unwrap_or(true)
    }

    /// Get the agent's home directory path.
    fn agent_home(&self) -> Option<String> {
        crate::paths::agent_home_dir(&self.agent_id)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    }

    /// Build a ToolExecContext with agent home directory and context window.
    pub(crate) fn tool_context(&self) -> tools::ToolExecContext {
        let agent_def = crate::agent_loader::load_agent(&self.agent_id);
        let mut require_approval = agent_def.as_ref()
            .map(|def| def.config.behavior.require_approval.clone())
            .unwrap_or_default();
        // Merge plan agent ask tools (e.g., exec requires approval during planning)
        if let types::PlanAgentMode::PlanAgent { ask_tools, .. } = &self.plan_agent_mode {
            for tool in ask_tools {
                if !require_approval.contains(tool) {
                    require_approval.push(tool.clone());
                }
            }
        }
        let force_sandbox = agent_def.as_ref()
            .map(|def| def.config.behavior.sandbox)
            .unwrap_or(false);
        tools::ToolExecContext {
            context_window_tokens: Some(self.context_window),
            home_dir: self.agent_home(),
            session_id: self.session_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            subagent_depth: self.subagent_depth,
            require_approval,
            force_sandbox,
            plan_mode_allow_paths: self.plan_mode_allow_paths.clone(),
        }
    }

    /// Get the context window size.
    pub fn get_context_window(&self) -> u32 {
        self.context_window
    }

    /// Set the compact config (called from lib.rs after agent construction).
    pub fn set_compact_config(&mut self, config: crate::context_compact::CompactConfig) {
        self.compact_config = config;
    }

    pub async fn chat(&self, message: &str, attachments: &[Attachment], reasoning_effort: Option<&str>, cancel: Arc<AtomicBool>, on_delta: impl Fn(&str) + Send + 'static) -> Result<(String, Option<String>)> {
        // Log agent chat dispatch
        if let Some(logger) = crate::get_logger() {
            let (provider_type, model_name) = match &self.provider {
                LlmProvider::Anthropic { model, .. } => ("Anthropic", model.as_str()),
                LlmProvider::OpenAIChat { model, .. } => ("OpenAIChat", model.as_str()),
                LlmProvider::OpenAIResponses { model, .. } => ("OpenAIResponses", model.as_str()),
                LlmProvider::Codex { model, .. } => ("Codex", model.as_str()),
            };
            let history_len = self.conversation_history.lock().unwrap().len();
            let msg_preview = if message.len() > 200 { format!("{}...", crate::truncate_utf8(message, 200)) } else { message.to_string() };
            logger.log("info", "agent", "agent::chat",
                &format!("Agent chat dispatching: provider={}, model={}", provider_type, model_name),
                Some(json!({
                    "provider_type": provider_type,
                    "model": model_name,
                    "reasoning_effort": reasoning_effort,
                    "attachments": attachments.len(),
                    "history_messages": history_len,
                    "message_preview": msg_preview,
                }).to_string()),
                None, None);
        }

        match &self.provider {
            LlmProvider::Anthropic { api_key, base_url, model } => {
                self.chat_anthropic(api_key, base_url, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
            LlmProvider::OpenAIChat { api_key, base_url, model } => {
                self.chat_openai_chat(api_key, base_url, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
            LlmProvider::OpenAIResponses { api_key, base_url, model } => {
                self.chat_openai_responses(api_key, base_url, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
            LlmProvider::Codex { access_token, account_id, model } => {
                self.chat_openai(access_token, account_id, model, message, attachments, reasoning_effort, &cancel, &on_delta).await
            }
        }
    }
}
