//! Kernel contract and owner wire types for feature-owned external memory IO.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::{
    ExternalMemoryProviderCompatibilityReport, ExternalMemoryProviderSyncReport,
    ExternalMemoryProvidersConfig, MemoryStats,
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderCredentialInput {
    pub provider_id: String,
    pub endpoint: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub subject_id: String,
    #[serde(default)]
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMemoryProviderCredentialStatus {
    pub provider_id: String,
    pub configured: bool,
    pub endpoint_configured: bool,
    pub api_key_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub type ExternalSyncFuture =
    Pin<Box<dyn Future<Output = ExternalMemoryProviderSyncReport> + Send + 'static>>;
pub type SaveCredentialsFuture =
    Pin<Box<dyn Future<Output = Result<ExternalMemoryProviderCredentialStatus>> + Send + 'static>>;
pub type TestConnectionFuture = Pin<
    Box<dyn Future<Output = Result<ExternalMemoryProviderCompatibilityReport>> + Send + 'static>,
>;

#[derive(Clone, Copy)]
pub struct ExternalMemoryRuntime {
    pub execute_sync:
        fn(ExternalMemoryProvidersConfig, MemoryStats, Option<String>) -> ExternalSyncFuture,
    pub schedule_sync: fn(),
    pub spawn_sync_loop: fn(),
    pub hydrate_config: fn(ExternalMemoryProvidersConfig) -> ExternalMemoryProvidersConfig,
    pub save_credentials: fn(ExternalMemoryProviderCredentialInput) -> SaveCredentialsFuture,
    pub test_connection: fn(String) -> TestConnectionFuture,
    pub compatibility_snapshot:
        fn(&super::ExternalMemoryProviderConfig) -> ExternalMemoryProviderCompatibilityReport,
    pub credential_status: fn(&str) -> Result<ExternalMemoryProviderCredentialStatus>,
    pub clear_credentials: fn(&str) -> Result<()>,
    pub save_config: fn(ExternalMemoryProvidersConfig, &'static str) -> Result<()>,
    pub patch_config: fn(serde_json::Value, &str) -> Result<ExternalMemoryProvidersConfig>,
}

static RUNTIME: OnceLock<ExternalMemoryRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_external_memory_runtime(
    runtime: ExternalMemoryRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("external memory runtime"))
}

fn runtime() -> Option<&'static ExternalMemoryRuntime> {
    let runtime = RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "external_runtime_unavailable",
            "External memory runtime is not wired"
        );
    }
    runtime
}

pub fn schedule_external_memory_provider_sync() {
    if let Some(runtime) = runtime() {
        (runtime.schedule_sync)();
    }
}

pub fn spawn_external_memory_provider_sync_loop() {
    if let Some(runtime) = runtime() {
        (runtime.spawn_sync_loop)();
    }
}

pub fn hydrate_external_memory_provider_config(
    config: ExternalMemoryProvidersConfig,
) -> ExternalMemoryProvidersConfig {
    runtime().map_or(config.clone(), |runtime| (runtime.hydrate_config)(config))
}

pub async fn execute_external_memory_provider_sync(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
) -> ExternalMemoryProviderSyncReport {
    let Some(runtime) = runtime() else {
        return ExternalMemoryProviderSyncReport {
            generated_at: chrono::Utc::now().to_rfc3339(),
            global_enabled: config.enabled,
            external_io_performed: false,
            local_memory_total: stats.total,
            local_memory_with_embedding: stats.with_embedding,
            stats_unavailable: stats_error.is_some(),
            stats_error,
            runnable_provider_count: 0,
            blocked_provider_count: config.providers.len(),
            executed_provider_count: 0,
            succeeded_provider_count: 0,
            failed_provider_count: 0,
            providers: Vec::new(),
        };
    };
    (runtime.execute_sync)(config, stats, stats_error).await
}

pub async fn save_external_memory_provider_credentials(
    input: ExternalMemoryProviderCredentialInput,
) -> Result<ExternalMemoryProviderCredentialStatus> {
    let runtime = runtime().ok_or_else(|| anyhow!("external memory runtime is not wired"))?;
    (runtime.save_credentials)(input).await
}

pub async fn test_external_memory_provider_connection(
    provider_id: String,
) -> Result<ExternalMemoryProviderCompatibilityReport> {
    let runtime = runtime().ok_or_else(|| anyhow!("external memory runtime is not wired"))?;
    (runtime.test_connection)(provider_id).await
}

pub fn external_memory_provider_compatibility_snapshot(
    provider: &super::ExternalMemoryProviderConfig,
) -> ExternalMemoryProviderCompatibilityReport {
    if let Some(runtime) = runtime() {
        return (runtime.compatibility_snapshot)(provider);
    }
    ExternalMemoryProviderCompatibilityReport {
        provider_id: provider.id.clone(),
        kind: provider.kind,
        status: super::ExternalMemoryProviderCompatibilityStatus::Unverified,
        checked_at: String::new(),
        external_io_performed: false,
        detected_version: None,
        minimum_version: None,
        recommended_version: None,
        capabilities: Vec::new(),
        error: Some("external memory runtime is not wired".to_string()),
    }
}

pub fn get_external_memory_provider_credential_status(
    provider_id: &str,
) -> Result<ExternalMemoryProviderCredentialStatus> {
    let runtime = runtime().ok_or_else(|| anyhow!("external memory runtime is not wired"))?;
    (runtime.credential_status)(provider_id)
}

pub fn clear_external_memory_provider_credentials(provider_id: &str) -> Result<()> {
    let runtime = runtime().ok_or_else(|| anyhow!("external memory runtime is not wired"))?;
    (runtime.clear_credentials)(provider_id)
}

pub fn save_external_memory_providers_config(
    config: ExternalMemoryProvidersConfig,
    source: &'static str,
) -> Result<()> {
    let runtime = runtime().ok_or_else(|| anyhow!("external memory runtime is not wired"))?;
    (runtime.save_config)(config, source)
}

pub fn patch_external_memory_providers_config(
    patch: serde_json::Value,
    source: &str,
) -> Result<ExternalMemoryProvidersConfig> {
    let runtime = runtime().ok_or_else(|| anyhow!("external memory runtime is not wired"))?;
    (runtime.patch_config)(patch, source)
}
