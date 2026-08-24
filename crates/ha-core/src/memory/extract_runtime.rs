//! Kernel-owned contract for the optional memory extraction machine.
//!
//! The implementation is registered by `ha-memory`; this module deliberately
//! contains no extraction prompts, provider construction or background
//! scheduling policy. Unwired extraction is an explicit, observable no-op so
//! a reduced kernel build can still run without silently pretending to persist
//! memories.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::agent::{AssistantAgent, ChatUsage};
use crate::provider::ProviderConfig;
use crate::session::SessionDB;

pub type MemoryExtractFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type TrackedExtractionFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Captured single-model capability for Memory Extract.
///
/// Memory Extract intentionally does not use the shared automation model-chain
/// runtime: its configuration contract is one model, with provider-profile
/// failover only. Capturing the complete provider here also prevents a config
/// edit after turn completion from changing the model credentials or endpoint
/// underneath an already-admitted extraction.
#[derive(Clone)]
pub struct MemoryExtractModel {
    provider: ProviderConfig,
    model_id: String,
}

pub struct MemoryExtractModelOutput {
    pub text: String,
    pub usage: ChatUsage,
}

impl MemoryExtractModel {
    pub fn capture(provider: &ProviderConfig, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.clone(),
            model_id: model_id.into(),
        }
    }

    pub fn provider_id(&self) -> &str {
        &self.provider.id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Execute the dedicated Memory Extract single-model path against the
    /// captured provider snapshot. This stays kernel-owned so the feature
    /// machine never receives credentials or a concrete `AssistantAgent`.
    pub async fn query(
        &self,
        agent_id: &str,
        session_id: &str,
        instruction: &str,
        max_tokens: u32,
    ) -> Result<MemoryExtractModelOutput> {
        let mut agent = AssistantAgent::try_new_from_provider(&self.provider, &self.model_id)
            .await?
            .with_failover_context(&self.provider);
        agent.set_agent_id(agent_id);
        agent.set_session_id(session_id);
        let result = agent
            .side_query(instruction, max_tokens)
            .await
            .with_context(|| {
                format!(
                    "memory extraction side_query failed (provider_id={}, api_type={}, model={}, session={})",
                    self.provider.id,
                    self.provider.api_type.display_name(),
                    self.model_id,
                    session_id
                )
            })?;
        Ok(MemoryExtractModelOutput {
            text: result.text,
            usage: result.usage,
        })
    }
}

#[derive(Clone, Copy)]
pub struct MemoryExtractRuntime {
    pub run_extraction: for<'a> fn(
        &'a [Value],
        &'a str,
        &'a str,
        &'a MemoryExtractModel,
        Option<Arc<SessionDB>>,
    ) -> MemoryExtractFuture<'a, ()>,
    pub flush_before_compact: for<'a> fn(
        &'a [Value],
        &'a str,
        &'a str,
        &'a MemoryExtractModel,
        Option<Arc<SessionDB>>,
    ) -> MemoryExtractFuture<'a, Result<usize>>,
    pub spawn_tracked_extraction: fn(String, TrackedExtractionFuture),
    pub cancel_active_extractions: fn(&str) -> usize,
    pub cancel_idle_extraction: fn(&str) -> bool,
    pub schedule_idle_extraction: fn(String, String, String, u64),
    pub flush_all_idle_extractions: fn(),
}

static RUNTIME: OnceLock<MemoryExtractRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register(runtime: MemoryExtractRuntime) -> std::result::Result<(), &'static str> {
    RUNTIME
        .set(runtime)
        .map_err(|_| "memory extraction runtime already registered")
}

fn runtime() -> Option<&'static MemoryExtractRuntime> {
    let runtime = RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "extract_runtime_unavailable",
            "Memory extraction runtime is not wired; optional extraction is disabled"
        );
    }
    runtime
}

pub async fn run_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    model: &MemoryExtractModel,
    session_db: Option<Arc<SessionDB>>,
) {
    let Some(runtime) = runtime() else {
        return;
    };
    (runtime.run_extraction)(messages, agent_id, session_id, model, session_db).await;
}

pub async fn flush_before_compact(
    messages_to_discard: &[Value],
    agent_id: &str,
    session_id: &str,
    model: &MemoryExtractModel,
    session_db: Option<Arc<SessionDB>>,
) -> Result<usize> {
    let Some(runtime) = runtime() else {
        return Ok(0);
    };
    (runtime.flush_before_compact)(messages_to_discard, agent_id, session_id, model, session_db)
        .await
}

pub fn spawn_tracked_extraction<F>(session_id: String, future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    if let Some(runtime) = runtime() {
        (runtime.spawn_tracked_extraction)(session_id, Box::pin(future));
    }
}

pub fn cancel_active_extractions(session_id: &str) -> usize {
    runtime().map_or(0, |runtime| (runtime.cancel_active_extractions)(session_id))
}

pub fn cancel_idle_extraction(session_id: &str) -> bool {
    if let Some(runtime) = runtime() {
        return (runtime.cancel_idle_extraction)(session_id);
    }
    false
}

pub fn schedule_idle_extraction(
    agent_id: String,
    session_id: String,
    updated_at: String,
    idle_timeout_secs: u64,
) {
    if let Some(runtime) = runtime() {
        (runtime.schedule_idle_extraction)(agent_id, session_id, updated_at, idle_timeout_secs);
    }
}

pub fn flush_all_idle_extractions() {
    if let Some(runtime) = runtime() {
        (runtime.flush_all_idle_extractions)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ApiType;

    #[test]
    fn model_capability_keeps_the_admitted_provider_snapshot() {
        let mut provider = ProviderConfig::new(
            "original".to_string(),
            ApiType::OpenaiChat,
            "https://original.example/v1".to_string(),
            "original-key".to_string(),
        );
        provider.id = "provider-original".to_string();

        let captured = MemoryExtractModel::capture(&provider, "model-original");
        provider.id = "provider-edited".to_string();
        provider.base_url = "https://edited.example/v1".to_string();
        provider.api_key = "edited-key".to_string();

        assert_eq!(captured.provider_id(), "provider-original");
        assert_eq!(captured.model_id(), "model-original");
        assert_eq!(captured.provider.base_url, "https://original.example/v1");
        assert_eq!(captured.provider.api_key, "original-key");
    }
}
