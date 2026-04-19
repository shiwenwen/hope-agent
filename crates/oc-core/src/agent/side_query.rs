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

use super::llm_adapter::{OneShotMode, OneShotRequest};
use super::types::{
    AssistantAgent, CacheSafeParams, ProviderFormat, SideQueryResult,
};
use crate::failover::executor::{execute_with_failover, FailoverPolicy};

impl AssistantAgent {
    /// Save cache-safe params after building the main chat request.
    /// Called from each provider's chat method after compaction, before the tool loop.
    /// Uses Arc to avoid deep-cloning conversation data on every chat turn.
    pub(super) fn save_cache_safe_params(
        &self,
        system_prompt: String,
        tool_schemas: Vec<serde_json::Value>,
        conversation_history: Vec<serde_json::Value>,
    ) {
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

        // Fast path: legacy constructors (new_anthropic / new_openai / test
        // paths) don't carry a ProviderConfig, so we issue a single direct
        // attempt with no failover.
        let (Some(provider_config), Some(session_id)) =
            (self.provider_config.as_ref(), self.session_id.as_deref())
        else {
            return self
                .side_query_direct(&client, cached.as_deref(), instruction, max_tokens)
                .await;
        };

        let model_id = self.provider.model();

        let exec_result = execute_with_failover(
            provider_config.as_ref(),
            session_id,
            FailoverPolicy::side_query_default(),
            // Low-frequency background path — no UI rotation event needed.
            None,
            |profile| {
                let provider = AssistantAgent::build_llm_provider(
                    provider_config.as_ref(),
                    model_id,
                    profile,
                );
                let cached_for_call = cached.clone();
                let client_ref = &client;
                async move {
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

        exec_result.map_err(|e| anyhow::anyhow!("side query: {}", e))
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
        let result = self
            .provider
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
}
