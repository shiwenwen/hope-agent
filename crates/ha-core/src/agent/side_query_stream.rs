//! Streaming sibling of [`super::side_query`].
//!
//! Same cache-safe prefix + failover machinery as `side_query`, but forwards the
//! assistant's text to a callback **as it streams** instead of discarding
//! deltas. The two modules are deliberately parallel and never touch each
//! other — `side_query` is the non-streaming background path (recall / title /
//! memory / summarize), `side_query_streaming` powers the design space's live
//! generation preview.
//!
//! `on_text` receives the **cumulative text so far for the current attempt**
//! (not a raw delta). On a mid-stream failover retry the accumulator restarts,
//! so the caller re-renders idempotently from a fresh snapshot rather than
//! stitching two providers' partial output together. The authoritative result
//! is always the returned `SideQueryResult.text`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use super::content::build_user_content_for_provider;
use super::llm_adapter::{OneShotMode, OneShotRequest};
use super::types::{
    AssistantAgent, Attachment, CacheSafeParams, LlmProvider, ProviderFormat, SideQueryResult,
};
use crate::failover::executor::{execute_with_failover, FailoverPolicy};

impl AssistantAgent {
    /// Streaming side query: reuses the main conversation's cache-safe prefix,
    /// routes through failover when `provider_config` + `session_id` are set,
    /// and forwards streamed text to `on_text` (cumulative-per-attempt).
    ///
    /// Cancellation is cooperative via `cancel`; the underlying SSE parser stops
    /// pulling chunks when it flips true.
    pub async fn side_query_streaming(
        &self,
        instruction: &str,
        max_tokens: u32,
        cancel: &Arc<AtomicBool>,
        on_text: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<SideQueryResult> {
        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        let cached = self
            .cache_safe_params
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let expected_format = ProviderFormat::from(&self.provider);
        let model_id = self.provider.model();

        // Fast path: legacy constructors (no ProviderConfig / session) → single
        // direct streaming attempt, no rotation. Mirrors `side_query`.
        let (Some(provider_config), Some(session_id)) =
            (self.provider_config.as_ref(), self.session_id.as_deref())
        else {
            let result = self
                .side_query_streaming_direct(
                    &client,
                    cached.as_deref(),
                    instruction,
                    max_tokens,
                    cancel,
                    on_text,
                )
                .await;
            if let Err(e) = &result {
                app_warn!(
                    "agent",
                    "side_query_stream",
                    "Streaming side query failed: provider={} model={} path=direct err={}",
                    expected_format.label(),
                    model_id,
                    e
                );
            }
            return result;
        };

        let exec_result = execute_with_failover(
            provider_config.as_ref(),
            session_id,
            FailoverPolicy::side_query_default(),
            None,
            |profile| {
                let cached_for_call = cached.clone();
                let client_ref = &client;
                let provider_config_ref = provider_config.as_ref();
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
                    // Fresh accumulator per attempt: a retry restarts the
                    // cumulative snapshot so the preview re-renders cleanly.
                    let acc = std::sync::Mutex::new(String::new());
                    let forward = |delta: &str| {
                        let mut g = acc.lock().unwrap_or_else(|e| e.into_inner());
                        g.push_str(delta);
                        on_text(&g);
                    };
                    let result = provider
                        .as_adapter()
                        .one_shot_stream(
                            client_ref,
                            OneShotRequest {
                                instruction,
                                max_tokens,
                                mode,
                                user_content: None,
                            },
                            cancel,
                            &forward,
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

        exec_result.map_err(|e| {
            app_warn!(
                "agent",
                "side_query_stream",
                "Streaming side query failed: provider_id={} model={} session={} path=failover err={}",
                provider_config.id,
                model_id,
                session_id,
                e
            );
            anyhow::anyhow!("side query (streaming): {}", e)
        })
    }

    /// Streaming sibling of [`super::side_query`]'s
    /// `independent_query_with_attachments`: an independent one-shot call that
    /// carries image attachments AND forwards the assistant text to `on_text`
    /// (cumulative-per-attempt, same contract as [`Self::side_query_streaming`]).
    /// Powers the design space's image-referenced live generation — the model
    /// sees the original image directly instead of a second-hand text brief.
    ///
    /// Uses `OneShotMode::Independent { system }` (no cached chat prefix — the
    /// caller supplies a scoped system prompt framing the image as untrusted
    /// source material, never instructions). Does NOT self-record usage: the
    /// automation layer stamps the ledger per candidate, mirroring
    /// [`Self::side_query_streaming`].
    pub async fn side_query_streaming_with_attachments(
        &self,
        system: &str,
        instruction: &str,
        attachments: &[Attachment],
        max_tokens: u32,
        cancel: &Arc<AtomicBool>,
        on_text: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<SideQueryResult> {
        let client =
            crate::provider::apply_proxy(reqwest::Client::builder().user_agent(&self.user_agent))
                .build()
                .map_err(|e| anyhow::anyhow!("HTTP client error: {}", e))?;

        let expected_format = ProviderFormat::from(&self.provider);
        let model_id = self.provider.model();

        // Fast path mirrors `side_query_streaming`: no ProviderConfig / session
        // → single direct streaming attempt, no rotation.
        let (Some(provider_config), Some(session_id)) =
            (self.provider_config.as_ref(), self.session_id.as_deref())
        else {
            let result = self
                .side_query_streaming_with_attachments_direct(
                    &client,
                    system,
                    instruction,
                    attachments,
                    max_tokens,
                    cancel,
                    on_text,
                )
                .await;
            if let Err(e) = &result {
                app_warn!(
                    "agent",
                    "side_query_stream",
                    "Streaming multimodal side query failed: provider={} model={} path=direct err={}",
                    expected_format.label(),
                    model_id,
                    e
                );
            }
            return result;
        };

        let exec_result = execute_with_failover(
            provider_config.as_ref(),
            session_id,
            FailoverPolicy::side_query_default(),
            None,
            |profile| {
                let client_ref = &client;
                let provider_config_ref = provider_config.as_ref();
                let profile_owned = profile.cloned();
                async move {
                    let provider = AssistantAgent::build_llm_provider(
                        provider_config_ref,
                        model_id,
                        profile_owned.as_ref(),
                    )
                    .await?;
                    // Per-attempt content build: the attachment blocks follow
                    // the provider wire format, which is a property of the
                    // provider we just constructed.
                    let user_content = build_user_content_for_provider(
                        ProviderFormat::from(&provider),
                        instruction,
                        attachments,
                    );
                    let acc = std::sync::Mutex::new(String::new());
                    let forward = |delta: &str| {
                        let mut g = acc.lock().unwrap_or_else(|e| e.into_inner());
                        g.push_str(delta);
                        on_text(&g);
                    };
                    let result = provider
                        .as_adapter()
                        .one_shot_stream(
                            client_ref,
                            OneShotRequest {
                                instruction,
                                max_tokens,
                                mode: OneShotMode::Independent { system },
                                user_content: Some(user_content),
                            },
                            cancel,
                            &forward,
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

        exec_result.map_err(|e| {
            app_warn!(
                "agent",
                "side_query_stream",
                "Streaming multimodal side query failed: provider_id={} model={} session={} path=failover err={}",
                provider_config.id,
                model_id,
                session_id,
                e
            );
            anyhow::anyhow!("side query (streaming, attachments): {}", e)
        })
    }

    /// Direct (no-failover) multimodal streaming path — mirrors
    /// [`Self::side_query_streaming_direct`], including Codex OAuth hydration.
    #[allow(clippy::too_many_arguments)]
    async fn side_query_streaming_with_attachments_direct(
        &self,
        client: &reqwest::Client,
        system: &str,
        instruction: &str,
        attachments: &[Attachment],
        max_tokens: u32,
        cancel: &Arc<AtomicBool>,
        on_text: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<SideQueryResult> {
        let hydrated_codex;
        let provider = if matches!(
            &self.provider,
            LlmProvider::Codex { access_token, account_id, .. }
                if access_token.is_empty() || account_id.is_empty()
        ) {
            let LlmProvider::Codex { model, .. } = &self.provider else {
                unreachable!("guarded by the matches! above");
            };
            let (access_token, account_id) = crate::oauth::load_fresh_codex_token().await?;
            hydrated_codex = LlmProvider::Codex {
                access_token,
                account_id,
                model: model.clone(),
            };
            &hydrated_codex
        } else {
            &self.provider
        };

        let user_content = build_user_content_for_provider(
            ProviderFormat::from(provider),
            instruction,
            attachments,
        );
        let acc = std::sync::Mutex::new(String::new());
        let forward = |delta: &str| {
            let mut g = acc.lock().unwrap_or_else(|e| e.into_inner());
            g.push_str(delta);
            on_text(&g);
        };
        let result = provider
            .as_adapter()
            .one_shot_stream(
                client,
                OneShotRequest {
                    instruction,
                    max_tokens,
                    mode: OneShotMode::Independent { system },
                    user_content: Some(user_content),
                },
                cancel,
                &forward,
            )
            .await?;
        Ok(SideQueryResult {
            text: result.text,
            usage: result.usage,
        })
    }

    /// Direct (no-failover) streaming path — mirrors `side_query_direct`,
    /// including Codex OAuth hydration for constructors that carry a placeholder
    /// Codex provider.
    async fn side_query_streaming_direct(
        &self,
        client: &reqwest::Client,
        cached: Option<&CacheSafeParams>,
        instruction: &str,
        max_tokens: u32,
        cancel: &Arc<AtomicBool>,
        on_text: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<SideQueryResult> {
        let mode = match cached {
            Some(params) => OneShotMode::Cached(params),
            None => OneShotMode::Bare,
        };

        let hydrated_codex;
        let provider = if matches!(
            &self.provider,
            LlmProvider::Codex { access_token, account_id, .. }
                if access_token.is_empty() || account_id.is_empty()
        ) {
            let LlmProvider::Codex { model, .. } = &self.provider else {
                unreachable!("guarded by the matches! above");
            };
            let (access_token, account_id) = crate::oauth::load_fresh_codex_token().await?;
            hydrated_codex = LlmProvider::Codex {
                access_token,
                account_id,
                model: model.clone(),
            };
            &hydrated_codex
        } else {
            &self.provider
        };

        let acc = std::sync::Mutex::new(String::new());
        let forward = |delta: &str| {
            let mut g = acc.lock().unwrap_or_else(|e| e.into_inner());
            g.push_str(delta);
            on_text(&g);
        };
        let result = provider
            .as_adapter()
            .one_shot_stream(
                client,
                OneShotRequest {
                    instruction,
                    max_tokens,
                    mode,
                    user_content: None,
                },
                cancel,
                &forward,
            )
            .await?;
        Ok(SideQueryResult {
            text: result.text,
            usage: result.usage,
        })
    }
}
