//! `prompt` hook handler — runs a one-shot LLM side-query and returns the
//! completion as the hook's stdout (design §7.5).
//!
//! The completion rides the normal output parser (plaintext → context for
//! SessionStart / UserPromptSubmit; JSON envelope honored for any event). The
//! hook input JSON is appended to the configured prompt so the model has the
//! event context. Bounded by the handler deadline.
//!
//! NOTE: this spends tokens and adds provider latency on every fire — keep it
//! off hot blocking events (PreToolUse) unless that cost is intended.

use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::super::config::PromptHookConfig;
use super::super::env::HookEnv;
use super::super::types::HookInput;
use super::{HookHandler, RawHookResult};
use crate::agent::AssistantAgent;
use crate::config::AppConfig;

/// Default `prompt` hook timeout — LLM calls are slower than shells.
const DEFAULT_PROMPT_TIMEOUT_SECS: u64 = 60;
/// Output cap for the side-query.
const PROMPT_MAX_TOKENS: u32 = 2048;

pub struct PromptHandler {
    config: PromptHookConfig,
}

impl PromptHandler {
    pub fn new(config: PromptHookConfig) -> Self {
        Self { config }
    }
}

/// Build the side-query agent: an explicit `provider:model` override when set,
/// else the shared analysis-agent builder (active-model / first-provider
/// fallbacks already wired).
async fn build_prompt_agent(
    model: Option<&str>,
    app_cfg: &AppConfig,
) -> anyhow::Result<AssistantAgent> {
    if let Some(target) = model {
        if let Some((prov_id, model_id)) = target.split_once(':') {
            if let Some(prov) = app_cfg
                .providers
                .iter()
                .find(|p| p.id == prov_id && p.enabled)
            {
                return Ok(AssistantAgent::try_new_from_provider(prov, model_id)
                    .await?
                    .with_failover_context(prov));
            }
        }
    }
    let (agent, _model_id) = crate::recap::report::build_analysis_agent(app_cfg).await?;
    Ok(agent)
}

#[async_trait]
impl HookHandler for PromptHandler {
    fn identity(&self) -> String {
        // Prompt text + model define the handler (design §7.7 dedup).
        format!(
            "{}|{}",
            self.config.prompt,
            self.config.model.as_deref().unwrap_or("")
        )
    }

    fn handler_type(&self) -> &'static str {
        "prompt"
    }

    fn default_timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout.unwrap_or(DEFAULT_PROMPT_TIMEOUT_SECS))
    }

    async fn run(&self, input: &HookInput, _env: &HookEnv, deadline: Instant) -> RawHookResult {
        let start = Instant::now();
        let app_cfg = crate::config::cached_config();
        let agent = match build_prompt_agent(self.config.model.as_deref(), &app_cfg).await {
            Ok(a) => a,
            Err(e) => {
                return RawHookResult::non_blocking_error(format!(
                    "prompt hook agent build failed: {e}"
                ))
            }
        };

        // Give the model the event context alongside the configured prompt.
        let instruction = match serde_json::to_string_pretty(input) {
            Ok(json) => format!(
                "{}\n\n## Hook event input\n```json\n{}\n```",
                self.config.prompt, json
            ),
            Err(_) => self.config.prompt.clone(),
        };

        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1));
        match tokio::time::timeout(remaining, agent.side_query(&instruction, PROMPT_MAX_TOKENS))
            .await
        {
            Ok(Ok(result)) => RawHookResult {
                exit_code: Some(0),
                stdout: result.text,
                stderr: String::new(),
                duration: start.elapsed(),
                timed_out: false,
            },
            Ok(Err(e)) => {
                RawHookResult::non_blocking_error(format!("prompt hook side-query failed: {e}"))
            }
            Err(_) => RawHookResult {
                exit_code: None,
                stdout: String::new(),
                stderr: "prompt hook timed out".to_string(),
                duration: start.elapsed(),
                timed_out: true,
            },
        }
    }
}
