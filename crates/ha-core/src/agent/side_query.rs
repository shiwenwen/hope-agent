//! Side-query mechanism for cache-friendly LLM calls.
//!
//! Reuses the main conversation's system_prompt + tool_schemas + conversation_history
//! as API request prefix, enabling prompt cache hits on Anthropic (explicit `cache_control`)
//! and OpenAI (automatic prefix caching). Side queries are non-streaming, single-turn,
//! no tool loop, no compaction.
//!
//! HTTP transport, body construction, and response parsing live in
//! [`super::llm_adapter`]; this module is just the cache-snapshot bookkeeping
//! and the public `side_query()` entry point.
//!
//! When the agent was constructed with `with_failover_context(provider_config)`
//! and `session_id` is set, `side_query` routes through
//! [`crate::failover::executor::execute_with_failover`] for profile rotation +
//! retry. Without both, it falls back to a single direct one-shot attempt
//! (used by `new_anthropic` / `new_openai` test / Codex OAuth paths).

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use super::llm_adapter::{OneShotMode, OneShotRequest};
use super::types::{AssistantAgent, CacheSafeParams, LlmProvider, ProviderFormat, SideQueryResult};
use crate::failover::executor::{execute_with_failover, FailoverPolicy};

fn side_query_cache_mode(
    cached: Option<&CacheSafeParams>,
    expected_format: &ProviderFormat,
) -> &'static str {
    match cached {
        Some(params) if &params.provider_format == expected_format => "cached",
        Some(_) => "format_mismatch_bare",
        None => "bare",
    }
}

fn codex_direct_needs_oauth_hydration(provider: &LlmProvider) -> bool {
    matches!(
        provider,
        LlmProvider::Codex {
            access_token,
            account_id,
            ..
        } if access_token.is_empty() || account_id.is_empty()
    )
}

impl AssistantAgent {
    /// Save cache-safe params after building the main chat request.
    /// Called from each provider's chat method after compaction, before the tool loop.
    /// Uses Arc to avoid deep-cloning conversation data on every chat turn.
    ///
    /// Captures only the cache-safe prefix. Awareness + active-memory
    /// suffixes are appended per-request inside `chat_round` (separate
    /// `cache_control` blocks for Anthropic, leading `system`/input items
    /// for OpenAI-family) — including them here would churn the snapshot
    /// every user turn and defeat the invariant this snapshot upholds.
    pub(super) fn save_cache_safe_params(
        &self,
        system_prompt: String,
        tool_schemas: Vec<serde_json::Value>,
        conversation_history: Vec<serde_json::Value>,
    ) {
        let mut conversation_history = conversation_history;
        crate::context_compact::round_grouping::strip_rounds(&mut conversation_history);
        let format = ProviderFormat::from(&self.provider);
        *self
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(CacheSafeParams {
            system_prompt,
            tool_schemas,
            conversation_history,
            provider_format: format,
        }));
    }

    /// Execute a side query that reuses the main conversation's cached prefix.
    ///
    /// - Non-streaming, single-turn, no tool loop, no compaction
    /// - Falls back to a minimal request if no cache-safe params are available
    /// - Returns response text + usage metrics (including cache hit info)
    ///
    /// When `provider_config` and `session_id` are both set on this agent,
    /// rotation/retry is delegated to [`execute_with_failover`] under
    /// [`FailoverPolicy::side_query_default`]. Otherwise we issue a single
    /// direct one-shot call (legacy fast path).
    pub async fn side_query(&self, instruction: &str, max_tokens: u32) -> Result<SideQueryResult> {
        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        // Arc::clone is cheap (pointer bump), no deep copy
        let cached = self
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let expected_format = ProviderFormat::from(&self.provider);
        let cache_mode = side_query_cache_mode(cached.as_deref(), &expected_format);
        let model_id = self.provider.model();
        let instruction_bytes = instruction.len();

        // Fast path: legacy constructors (new_anthropic / new_openai / test
        // paths) don't carry a ProviderConfig, so we issue a single direct
        // attempt with no failover.
        let (Some(provider_config), Some(session_id)) =
            (self.provider_config.as_ref(), self.session_id.as_deref())
        else {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "debug",
                    "agent",
                    "side_query::dispatch",
                    &format!(
                        "Side query dispatch: provider={}, model={}, path=direct, cache_mode={}, max_tokens={}",
                        expected_format.label(),
                        model_id,
                        cache_mode,
                        max_tokens
                    ),
                    Some(
                        json!({
                            "provider": expected_format.label(),
                            "model": model_id,
                            "path": "direct",
                            "cache_mode": cache_mode,
                            "max_tokens": max_tokens,
                            "instruction_bytes": instruction_bytes,
                            "has_provider_config": self.provider_config.is_some(),
                            "has_session_id": self.session_id.is_some(),
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }
            let result = self
                .side_query_direct(&client, cached.as_deref(), instruction, max_tokens)
                .await;
            if let Err(e) = &result {
                app_warn!(
                    "agent",
                    "side_query",
                    "Side query failed: provider={} model={} path=direct cache_mode={} has_provider_config={} has_session_id={} err={}",
                    expected_format.label(),
                    model_id,
                    cache_mode,
                    self.provider_config.is_some(),
                    self.session_id.is_some(),
                    e
                );
            }
            return result;
        };

        if let Some(logger) = crate::get_logger() {
            logger.log(
                "debug",
                "agent",
                "side_query::dispatch",
                &format!(
                    "Side query dispatch: provider={}, model={}, session={}, path=failover, cache_mode={}, max_tokens={}",
                    provider_config.api_type.display_name(),
                    model_id,
                    session_id,
                    cache_mode,
                    max_tokens
                ),
                Some(
                    json!({
                        "provider_id": provider_config.id,
                        "provider_name": provider_config.name,
                        "api_type": provider_config.api_type.display_name(),
                        "model": model_id,
                        "session_id": session_id,
                        "path": "failover",
                        "cache_mode": cache_mode,
                        "max_tokens": max_tokens,
                        "instruction_bytes": instruction_bytes,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }

        let exec_result = execute_with_failover(
            provider_config.as_ref(),
            session_id,
            FailoverPolicy::side_query_default(),
            // Low-frequency background path — no UI rotation event needed.
            None,
            |profile| {
                let cached_for_call = cached.clone();
                let client_ref = &client;
                let provider_config_ref = provider_config.as_ref();
                // profile is `Option<&AuthProfile>`; clone to own it across
                // the `.await` inside build_llm_provider (Codex branch).
                let profile_owned = profile.cloned();
                async move {
                    let provider = AssistantAgent::build_llm_provider(
                        provider_config_ref,
                        model_id,
                        profile_owned.as_ref(),
                    )
                    .await?;
                    let mode = match cached_for_call.as_deref() {
                        Some(p) => OneShotMode::Cached(p),
                        None => OneShotMode::Bare,
                    };
                    let result = provider
                        .as_adapter()
                        .one_shot(
                            client_ref,
                            OneShotRequest {
                                instruction,
                                max_tokens,
                                mode,
                            },
                        )
                        .await?;
                    Ok(SideQueryResult {
                        text: result.text,
                        usage: result.usage,
                    })
                }
            },
        )
        .await;

        match exec_result {
            Ok(result) => Ok(result),
            Err(e) => {
                app_warn!(
                    "agent",
                    "side_query",
                    "Side query failed: provider_id={} api_type={} model={} session={} path=failover cache_mode={} err={}",
                    provider_config.id,
                    provider_config.api_type.display_name(),
                    model_id,
                    session_id,
                    cache_mode,
                    e
                );
                Err(anyhow::anyhow!("side query: {}", e))
            }
        }
    }

    /// Legacy fast path: single direct one-shot, no rotation, no retry.
    /// Used when the agent was built without `with_failover_context` or has
    /// no `session_id` (test paths, Codex OAuth fallback, etc.).
    async fn side_query_direct(
        &self,
        client: &reqwest::Client,
        cached: Option<&CacheSafeParams>,
        instruction: &str,
        max_tokens: u32,
    ) -> Result<SideQueryResult> {
        let mode = match cached {
            Some(params) => OneShotMode::Cached(params),
            None => OneShotMode::Bare,
        };

        let hydrated_codex;
        let provider = if codex_direct_needs_oauth_hydration(&self.provider) {
            let LlmProvider::Codex { model, .. } = &self.provider else {
                unreachable!("checked by codex_direct_needs_oauth_hydration");
            };
            let (access_token, account_id) = crate::oauth::load_fresh_codex_token().await?;
            app_info!(
                "agent",
                "side_query",
                "Hydrated Codex OAuth token for direct side_query: model={} has_account_id={}",
                model,
                !account_id.is_empty()
            );
            hydrated_codex = LlmProvider::Codex {
                access_token,
                account_id,
                model: model.clone(),
            };
            &hydrated_codex
        } else {
            &self.provider
        };

        let result = provider
            .as_adapter()
            .one_shot(
                client,
                OneShotRequest {
                    instruction,
                    max_tokens,
                    mode,
                },
            )
            .await?;

        Ok(SideQueryResult {
            text: result.text,
            usage: result.usage,
        })
    }

    /// Bare one-shot LLM call against an arbitrary `ProviderConfig` + model.
    ///
    /// Used by [`crate::permission::judge`] to run an independent judge
    /// model query — no `AssistantAgent` instance, no main-conversation
    /// cache reuse, no tool loop. Returns the assistant's text reply.
    pub(crate) async fn judge_one_shot(
        provider_config: &crate::provider::ProviderConfig,
        model_id: &str,
        instruction: &str,
        max_tokens: u32,
    ) -> Result<String> {
        let client = crate::provider::apply_proxy(
            reqwest::Client::builder().user_agent(super::config::USER_AGENT),
        )
        .build()
        .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        let provider = AssistantAgent::build_llm_provider(provider_config, model_id, None).await?;
        let result = provider
            .as_adapter()
            .one_shot(
                &client,
                OneShotRequest {
                    instruction,
                    max_tokens,
                    mode: OneShotMode::Bare,
                },
            )
            .await?;
        Ok(result.text)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::context_compact::round_grouping::{stamp_round, ROUND_KEY};
    use crate::provider::{ApiType, ProviderConfig};

    #[test]
    fn save_cache_safe_params_strips_round_metadata() {
        let agent = AssistantAgent::new_openai("token", "account", "gpt-5.4");
        let mut history = vec![
            json!({ "role": "user", "content": "hello" }),
            json!({ "role": "assistant", "content": "hi" }),
        ];
        stamp_round(&mut history[1], "r0");

        agent.save_cache_safe_params("SYS".to_string(), Vec::new(), history);

        let cached = agent
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .expect("cache snapshot");

        assert!(cached.conversation_history[1].get(ROUND_KEY).is_none());
    }

    #[test]
    fn cache_mode_labels_missing_and_mismatched_snapshots() {
        let params = CacheSafeParams {
            system_prompt: "SYS".to_string(),
            tool_schemas: Vec::new(),
            conversation_history: Vec::new(),
            provider_format: ProviderFormat::Codex,
        };

        assert_eq!(
            side_query_cache_mode(Some(&params), &ProviderFormat::Codex),
            "cached"
        );
        assert_eq!(
            side_query_cache_mode(Some(&params), &ProviderFormat::OpenAIResponses),
            "format_mismatch_bare"
        );
        assert_eq!(side_query_cache_mode(None, &ProviderFormat::Codex), "bare");
    }

    #[test]
    #[should_panic(expected = "Codex providers require AssistantAgent::try_new_from_provider")]
    fn sync_codex_provider_constructor_panics() {
        let provider = ProviderConfig::new(
            "Codex".to_string(),
            ApiType::Codex,
            ApiType::Codex.default_base_url().to_string(),
            String::new(),
        );
        let _ = AssistantAgent::new_from_provider(&provider, "gpt-5.5");
    }

    #[test]
    fn hydrated_codex_side_query_does_not_need_oauth_hydration() {
        let hydrated = AssistantAgent::new_openai("token", "account", "gpt-5.5");
        assert!(!codex_direct_needs_oauth_hydration(&hydrated.provider));
    }

    #[test]
    fn empty_codex_provider_needs_oauth_hydration() {
        let placeholder = LlmProvider::Codex {
            access_token: String::new(),
            account_id: String::new(),
            model: "gpt-5.5".to_string(),
        };
        assert!(codex_direct_needs_oauth_hydration(&placeholder));

        let token_only = LlmProvider::Codex {
            access_token: "token".to_string(),
            account_id: String::new(),
            model: "gpt-5.5".to_string(),
        };
        assert!(codex_direct_needs_oauth_hydration(&token_only));

        let account_only = LlmProvider::Codex {
            access_token: String::new(),
            account_id: "account".to_string(),
            model: "gpt-5.5".to_string(),
        };
        assert!(codex_direct_needs_oauth_hydration(&account_only));
    }
}
