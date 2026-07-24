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
use crate::automation::{self, ModelTaskSpec};
use crate::config::AppConfig;
use crate::provider::ActiveModel;

/// Default `prompt` hook timeout (official 30s).
const DEFAULT_PROMPT_TIMEOUT_SECS: u64 = 30;
/// Output cap for the side-query.
const PROMPT_MAX_TOKENS: u32 = 2048;

/// INPUT cap for the serialized hook payload spliced into the instruction.
/// Roughly 8k tokens of JSON — generous for every event's own fields, while
/// bounding the one field that has no natural size limit (`tool_input`).
const PROMPT_MAX_PAYLOAD_CHARS: usize = 32_768;

/// Bound the payload spliced into the LLM instruction, keeping a head slice plus
/// an explicit marker so the model can tell "the rest was cut" from "the field
/// was empty". Splits on a char boundary via `truncate_utf8` (byte slicing a
/// multi-byte payload would panic).
fn truncate_payload_for_prompt(json: &str) -> std::borrow::Cow<'_, str> {
    if json.len() <= PROMPT_MAX_PAYLOAD_CHARS {
        return std::borrow::Cow::Borrowed(json);
    }
    let head = crate::truncate_utf8(json, PROMPT_MAX_PAYLOAD_CHARS);
    std::borrow::Cow::Owned(format!(
        "{head}\n… [truncated: hook payload was {} bytes, capped at {}]",
        json.len(),
        PROMPT_MAX_PAYLOAD_CHARS
    ))
}

pub struct PromptHandler {
    config: PromptHookConfig,
}

impl PromptHandler {
    pub fn new(config: PromptHookConfig) -> Self {
        Self { config }
    }
}

/// Resolve this hook's model chain: `modelOverride` (new) → the deprecated
/// `model` string (parsed) → `function_models.automation` → chat default.
fn resolve_prompt_hook_chain(cfg: &PromptHookConfig, app_cfg: &AppConfig) -> Vec<ActiveModel> {
    let override_chain = cfg.model_override.clone().or_else(|| {
        cfg.model
            .as_deref()
            .and_then(automation::parse_legacy_model_string)
    });
    automation::effective_chain(app_cfg, override_chain)
}

#[async_trait]
impl HookHandler for PromptHandler {
    fn identity(&self) -> String {
        // Prompt text + full model chain + timeout define the handler
        // (design §7.7 dedup). Two configs sharing a prompt and primary
        // model but differing in fallback chain or timeout have materially
        // different failover/retry behavior and must not collide — or the
        // dispatch-time `(handler_type, identity)` dedup / `once` claim gate
        // would silently drop or permanently block one of them for the rest
        // of the session (same concern `mcp_tool.rs`'s `identity()` guards
        // against for its own fields). Prefer the new override for the
        // identity string; fall back to the deprecated raw string so
        // existing dedup keys don't shift under configs that haven't been
        // re-saved yet.
        let model_part = self
            .config
            .model_override
            .as_ref()
            .map(|c| {
                let mut parts = vec![c.primary.to_string()];
                parts.extend(c.fallbacks.iter().map(|m| m.to_string()));
                parts.join(",")
            })
            .or_else(|| self.config.model.clone())
            .unwrap_or_default();
        let timeout_part = self
            .config
            .timeout
            .map(|t| t.to_string())
            .unwrap_or_default();
        format!("{}|{}|{}", self.config.prompt, model_part, timeout_part)
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
        let chain = resolve_prompt_hook_chain(&self.config, &app_cfg);

        // Give the model the event context alongside the configured prompt,
        // BOUNDED. The payload is attacker-shaped in the sense that its size is
        // set by whatever the tool was called with: `tool_input` on a `write` of
        // a generated file, or an `apply_patch` diff, is unbounded, and this
        // instruction is a billed request on the interactive approval path. An
        // uncapped splice makes token cost and latency proportional to file size
        // in front of a user-facing dialog.
        let instruction = match serde_json::to_string_pretty(input) {
            Ok(json) => format!(
                "{}\n\n## Hook event input\n```json\n{}\n```",
                self.config.prompt,
                truncate_payload_for_prompt(&json)
            ),
            Err(_) => self.config.prompt.clone(),
        };

        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1));
        let fut = automation::run(ModelTaskSpec {
            purpose: "hooks.prompt",
            chain,
            session_key: "automation:hooks_prompt",
            instruction: &instruction,
            max_tokens: PROMPT_MAX_TOKENS,
        });
        match tokio::time::timeout(remaining, fut).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ModelChain;

    #[test]
    fn oversized_payload_is_capped_before_reaching_the_model() {
        // Under the cap: passed through byte-for-byte, no marker.
        let small = "{\"a\":1}";
        assert_eq!(truncate_payload_for_prompt(small), small);
        assert!(matches!(
            truncate_payload_for_prompt(small),
            std::borrow::Cow::Borrowed(_)
        ));

        // Over the cap: bounded, and the model is told it was cut rather than
        // being handed a silently short payload it would read as complete.
        let huge = format!("{{\"tool_input\":\"{}\"}}", "x".repeat(200_000));
        let out = truncate_payload_for_prompt(&huge);
        assert!(
            out.len() < huge.len(),
            "a 200 KB payload must not reach the billed request verbatim"
        );
        assert!(
            out.contains("truncated: hook payload was"),
            "the cut must be explicit, got tail {:?}",
            &out[out.len().saturating_sub(120)..]
        );
        assert!(
            out.starts_with("{\"tool_input\":\"xxx"),
            "head slice preserved"
        );

        // A multi-byte payload must be cut on a char boundary, not panic.
        let cjk = format!("{{\"t\":\"{}\"}}", "工具参数".repeat(20_000));
        assert!(cjk.len() > PROMPT_MAX_PAYLOAD_CHARS);
        let out = truncate_payload_for_prompt(&cjk);
        assert!(out.contains("truncated: hook payload was"));
    }

    fn base_config() -> PromptHookConfig {
        PromptHookConfig {
            prompt: "summarize".to_string(),
            model: None,
            model_override: Some(ModelChain {
                primary: ActiveModel {
                    provider_id: "openai".to_string(),
                    model_id: "gpt-5".to_string(),
                },
                fallbacks: Vec::new(),
            }),
            timeout: None,
            status_message: None,
            if_rule: None,
            once: None,
        }
    }

    #[test]
    fn identity_disambiguates_different_fallback_chains() {
        // Two hooks sharing a prompt + primary model but with different
        // fallback chains have different failover behavior and must not
        // dedup to one identity, or the dispatch dedup / `once` gate would
        // silently drop or permanently block one of them.
        let a = PromptHandler::new(base_config());
        let mut cfg_b = base_config();
        cfg_b
            .model_override
            .as_mut()
            .unwrap()
            .fallbacks
            .push(ActiveModel {
                provider_id: "anthropic".to_string(),
                model_id: "claude-sonnet-5".to_string(),
            });
        let b = PromptHandler::new(cfg_b);
        assert_ne!(a.identity(), b.identity());
    }

    #[test]
    fn identity_disambiguates_different_timeout() {
        let a = PromptHandler::new(base_config());
        let mut cfg_b = base_config();
        cfg_b.timeout = Some(120);
        let b = PromptHandler::new(cfg_b);
        assert_ne!(a.identity(), b.identity());
    }

    #[test]
    fn identity_stable_for_identical_config() {
        let a = PromptHandler::new(base_config());
        let b = PromptHandler::new(base_config());
        assert_eq!(a.identity(), b.identity());
    }
}
