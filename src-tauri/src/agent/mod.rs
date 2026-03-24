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
pub use types::{Attachment, AssistantAgent, CodexModel, LlmProvider};

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
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
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
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
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
            session_id: None,
            subagent_depth: 0,
            steer_run_id: None,
            denied_tools: Vec::new(),
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

    /// Set tools that are denied for this agent (depth-based tool policy).
    pub fn set_denied_tools(&mut self, tools: Vec<String>) {
        self.denied_tools = tools;
    }

    /// Build the full system prompt, including any extra context.
    pub(crate) fn build_full_system_prompt(&self, model: &str, provider: &str) -> String {
        let mut prompt = config::build_system_prompt(&self.agent_id, model, provider);
        if self.notification_enabled {
            prompt.push_str("\n\n- **send_notification**: Send a native desktop notification to alert the user about important events, task completions, or findings that need their attention. Parameters: title (optional), body (required).");
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
        let require_approval = crate::agent_loader::load_agent(&self.agent_id)
            .map(|def| def.config.behavior.require_approval.clone())
            .unwrap_or_default();
        tools::ToolExecContext {
            context_window_tokens: Some(self.context_window),
            home_dir: self.agent_home(),
            session_id: self.session_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            subagent_depth: self.subagent_depth,
            require_approval,
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
