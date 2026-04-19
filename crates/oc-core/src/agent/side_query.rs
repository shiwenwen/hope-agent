//! Side-query mechanism for cache-friendly LLM calls.
//!
//! Reuses the main conversation's system_prompt + tool_schemas + conversation_history
//! as API request prefix, enabling prompt cache hits on Anthropic (explicit `cache_control`)
//! and OpenAI (automatic prefix caching). Side queries are non-streaming, single-turn,
//! no tool loop, no compaction.
//!
//! HTTP transport, body construction, and response parsing live in
//! [`super::llm_adapter`]; this module is now just the cache-snapshot bookkeeping
//! and the public `side_query()` entry point.

use std::sync::Arc;

use anyhow::Result;

use super::llm_adapter::{OneShotMode, OneShotRequest};
use super::types::{
    AssistantAgent, CacheSafeParams, ProviderFormat, SideQueryResult,
};

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

        let mode = match cached.as_deref() {
            Some(params) => OneShotMode::Cached(params),
            None => OneShotMode::Bare,
        };
        let result = self
            .provider
            .as_adapter()
            .one_shot(
                &client,
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
