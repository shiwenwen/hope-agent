//! One-shot Provider contract and required runtime port.
//!
//! Network/body/SSE implementations live in `ha-agent-runtime`; core keeps the
//! cache-prefix contract because compaction, side-query and judge callers all
//! consume the same typed request surface.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::types::{CacheSafeParams, ChatUsage, LlmProvider};

pub enum OneShotMode<'a> {
    Cached(&'a CacheSafeParams),
    Independent { system: &'a str },
    Bare,
}

impl<'a> OneShotMode<'a> {
    pub fn cached_for(&self, format: super::types::ProviderFormat) -> Option<&'a CacheSafeParams> {
        match self {
            Self::Cached(params) if params.provider_format == format => Some(*params),
            _ => None,
        }
    }
}

pub struct OneShotRequest<'a> {
    pub instruction: &'a str,
    pub max_tokens: u32,
    pub mode: OneShotMode<'a>,
    pub user_content: Option<Value>,
}

pub struct OneShotResult {
    pub text: String,
    pub usage: ChatUsage,
}

#[async_trait]
pub trait LlmApiAdapter: Send + Sync {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult>;

    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult>;
}

#[async_trait]
pub trait OneShotRuntime: Send + Sync {
    async fn one_shot(
        &self,
        provider: &LlmProvider,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult>;

    async fn one_shot_stream(
        &self,
        provider: &LlmProvider,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult>;
}

static RUNTIME: OnceLock<&'static dyn OneShotRuntime> = OnceLock::new();

pub fn register_one_shot_runtime(
    runtime: &'static dyn OneShotRuntime,
) -> Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("one-shot Provider runtime"))
}

struct RuntimeAdapter<'a> {
    provider: &'a LlmProvider,
}

#[async_trait]
impl LlmApiAdapter for RuntimeAdapter<'_> {
    async fn one_shot(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
    ) -> Result<OneShotResult> {
        RUNTIME
            .get()
            .ok_or_else(|| anyhow::anyhow!("one-shot Provider runtime is not registered"))?
            .one_shot(self.provider, client, req)
            .await
    }

    async fn one_shot_stream(
        &self,
        client: &reqwest::Client,
        req: OneShotRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(dyn for<'s> Fn(&'s str) + Send + Sync),
    ) -> Result<OneShotResult> {
        RUNTIME
            .get()
            .ok_or_else(|| anyhow::anyhow!("one-shot Provider runtime is not registered"))?
            .one_shot_stream(self.provider, client, req, cancel, on_delta)
            .await
    }
}

impl LlmProvider {
    pub(super) fn as_adapter(&self) -> Box<dyn LlmApiAdapter + '_> {
        Box::new(RuntimeAdapter { provider: self })
    }
}
