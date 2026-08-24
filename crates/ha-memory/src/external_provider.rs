//! Additive external memory provider runtime.
//!
//! The local SQLite/claim stores remain authoritative. Credentials live in a
//! separate restricted file and are never returned by owner read APIs. Pulls
//! from a provider must enter the local review path before they can influence
//! prompts; concrete adapters own only network protocol translation.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use ha_core::paths::{
    external_memory_compatibility_path, external_memory_credential_path,
    external_memory_sync_lock_path, external_memory_sync_state_path,
};
use ha_core::platform::write_secure_file;

use ha_core::memory::{
    ExternalMemoryProviderCompatibilityReport, ExternalMemoryProviderCompatibilityStatus,
    ExternalMemoryProviderConfig, ExternalMemoryProviderKind,
    ExternalMemoryProviderPreflightAction, ExternalMemoryProviderSyncReport,
    ExternalMemoryProviderSyncResult, ExternalMemoryProviderSyncStatus,
    ExternalMemoryProvidersConfig, MemoryStats,
};

mod custom;
mod hindsight;
mod honcho;
mod http;
mod mem0;
mod open_viking;
mod supermemory;
mod zep;

const CREDENTIAL_SCHEMA_VERSION: u32 = 1;
const COMPATIBILITY_SCHEMA_VERSION: u32 = 1;
const MAX_ENDPOINT_CHARS: usize = 2_048;
const MAX_SUBJECT_ID_CHARS: usize = 256;
const MAX_PROTOCOL_CHARS: usize = 48;
const SYNC_STATE_SCHEMA_VERSION: u32 = 1;
const MAX_IMPORTED_CONTENT_CHARS: usize = 16_000;
const IMPORT_LEDGER_CHECKPOINT_EVERY: usize = 100;
const PROVIDER_SYNC_TIMEOUT: Duration = Duration::from_secs(120);
const PROVIDER_SYNC_LOCK_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROVIDER_SYNC_LOCK_POLL: Duration = Duration::from_millis(25);
/// A successful compatibility probe is an owner-granted capability to send
/// local memories. Keep that grant short-lived so a server replaced in place
/// cannot inherit compatibility evidence indefinitely.
const COMPATIBILITY_GRANT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
tokio::task_local! {
    static PROVIDER_SYNC_DEADLINE: std::time::Instant;
}
static AUTO_SYNC_QUEUED: AtomicBool = AtomicBool::new(false);
static AUTO_SYNC_DIRTY: AtomicBool = AtomicBool::new(false);
static EXTERNAL_PROVIDER_SYNC_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));
// Keep provider-list config commits and their credential/ledger cleanup in one
// lifecycle transaction. The global AppConfig write lock only covers the
// config file, so cleanup performed after `mutate_config` otherwise races a
// second provider-list save that re-adds an id.
static EXTERNAL_PROVIDER_CONFIG_WRITE_LOCK: Lazy<std::sync::Mutex<()>> =
    Lazy::new(|| std::sync::Mutex::new(()));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalMemoryProvidersPatch {
    #[serde(default)]
    enabled: Option<bool>,
    /// Provider entries are metadata patches keyed by `id`, not a replacement
    /// array. Omitting an existing provider therefore preserves it.
    #[serde(default)]
    providers: Vec<ExternalMemoryProviderPatch>,
    /// Destructive removal stays explicit so a partial metadata update cannot
    /// accidentally delete another provider and its credential files.
    #[serde(default)]
    remove_provider_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalMemoryProviderPatch {
    id: String,
    #[serde(default)]
    kind: Option<ExternalMemoryProviderKind>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    sync_policy: Option<ha_core::memory::ExternalMemorySyncPolicy>,
}

pub use ha_core::memory::external_provider::{
    ExternalMemoryProviderCredentialInput, ExternalMemoryProviderCredentialStatus,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalMemoryProviderCredentials {
    schema_version: u32,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub subject_id: String,
    pub protocol: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredExternalMemoryProviderCompatibility {
    schema_version: u32,
    /// Hash of the exact provider kind and credentials used by the probe.
    /// This is kept out of the owner-facing report so it cannot become a
    /// credential-correlation surface through transport APIs.
    #[serde(default)]
    credential_fingerprint: String,
    report: ExternalMemoryProviderCompatibilityReport,
}

#[derive(Clone, Copy)]
struct VersionRequirement {
    minimum: &'static str,
    recommended: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ParsedVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalMemoryProviderSyncLedger {
    #[serde(default = "default_sync_state_schema_version")]
    schema_version: u32,
    #[serde(default)]
    pub exported_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub exported_remote_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub pending_export_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub imported_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub remote_versions: BTreeMap<String, String>,
}

fn default_sync_state_schema_version() -> u32 {
    SYNC_STATE_SCHEMA_VERSION
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ExternalMemoryAdapterSyncOutcome {
    pub external_io_performed: bool,
    pub imported_memory_count: usize,
    pub exported_memory_count: usize,
    pub updated_memory_count: usize,
    pub skipped_memory_count: usize,
}

#[derive(Debug)]
pub(crate) struct ExternalMemoryAdapterSyncFailure {
    pub outcome: ExternalMemoryAdapterSyncOutcome,
    pub error: anyhow::Error,
}

#[async_trait]
pub(crate) trait ExternalMemoryProviderAdapter: Send + Sync {
    fn kind(&self) -> ExternalMemoryProviderKind;

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>;
}

#[derive(Clone, Copy)]
enum ExternalMemoryProviderSyncOrigin {
    Owner,
    Automatic,
}

pub async fn execute_external_memory_provider_sync(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
) -> ExternalMemoryProviderSyncReport {
    execute_external_memory_provider_sync_for_origin(
        config,
        stats,
        stats_error,
        ExternalMemoryProviderSyncOrigin::Owner,
    )
    .await
}

pub fn execute_sync_boxed(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
) -> ha_core::memory::external_provider::ExternalSyncFuture {
    Box::pin(execute_external_memory_provider_sync(
        config,
        stats,
        stats_error,
    ))
}

async fn execute_external_memory_provider_sync_for_origin(
    requested_config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
    origin: ExternalMemoryProviderSyncOrigin,
) -> ExternalMemoryProviderSyncReport {
    // The async mutex avoids occupying multiple blocking-pool workers for
    // same-process contenders. The OS lock is the actual data boundary: GUI,
    // server and ACP processes share credentials, ledgers and health fields.
    let _sync_guard = EXTERNAL_PROVIDER_SYNC_LOCK.lock().await;
    let _cross_process_guard = match acquire_external_memory_state_lock_async().await {
        Ok(guard) => guard,
        Err(error) => {
            return sync_lock_failure_report(
                requested_config,
                stats,
                stats_error,
                error.to_string(),
            )
        }
    };
    // Keep `_cross_process_guard` alive from before the authoritative config,
    // credential and ledger hydration through the final checkpoint and health
    // persistence below. The caller's snapshot is intentionally used only for
    // lock/reload failure projection: it may have waited behind an owner
    // policy mutation in another process.
    let live_config = ha_core::blocking::run_blocking(move || -> Result<_> {
        let snapshot = ha_core::config::reload_config_snapshot_from_disk()
            .context("reload external memory provider configuration")?;
        let mut config = snapshot.memory_providers.clone();
        apply_external_memory_sync_origin(&mut config, origin);
        Ok(hydrate_external_memory_provider_config(config))
    })
    .await;
    let config = match live_config {
        Ok(config) => config,
        Err(error) => {
            return sync_config_reload_failure_report(
                requested_config,
                stats,
                stats_error,
                error.to_string(),
            )
        }
    };
    let preflight = ha_core::memory::types::external_memory_sync_preflight_with_stats_status(
        &config,
        &stats,
        stats_error,
    );
    let preflight_report = preflight.clone();
    let mut results = Vec::with_capacity(preflight.providers.len());

    for provider_preflight in preflight.providers {
        let provider_config = config
            .providers
            .iter()
            .find(|provider| provider.id == provider_preflight.id);
        let result = match provider_preflight.action {
            ExternalMemoryProviderPreflightAction::Off => ExternalMemoryProviderSyncResult {
                id: provider_preflight.id.clone(),
                kind: provider_preflight.kind,
                display_name: provider_preflight.display_name.clone(),
                status: ExternalMemoryProviderSyncStatus::Off,
                external_io_performed: false,
                preflight: provider_preflight,
                imported_memory_count: 0,
                exported_memory_count: 0,
                updated_memory_count: 0,
                skipped_memory_count: 0,
                error: None,
            },
            ExternalMemoryProviderPreflightAction::Blocked => ExternalMemoryProviderSyncResult {
                id: provider_preflight.id.clone(),
                kind: provider_preflight.kind,
                display_name: provider_preflight.display_name.clone(),
                status: ExternalMemoryProviderSyncStatus::Blocked,
                external_io_performed: false,
                preflight: provider_preflight,
                imported_memory_count: 0,
                exported_memory_count: 0,
                updated_memory_count: 0,
                skipped_memory_count: 0,
                error: None,
            },
            ExternalMemoryProviderPreflightAction::WouldSync => match provider_config {
                Some(provider) => execute_provider_sync(provider, provider_preflight).await,
                None => ExternalMemoryProviderSyncResult {
                    id: provider_preflight.id.clone(),
                    kind: provider_preflight.kind,
                    display_name: provider_preflight.display_name.clone(),
                    status: ExternalMemoryProviderSyncStatus::Failed,
                    external_io_performed: false,
                    preflight: provider_preflight,
                    imported_memory_count: 0,
                    exported_memory_count: 0,
                    updated_memory_count: 0,
                    skipped_memory_count: 0,
                    error: Some("external memory provider config disappeared".to_string()),
                },
            },
        };
        results.push(result);
    }

    let health_results = results.clone();
    ha_core::blocking::run_blocking(move || persist_sync_health(&health_results)).await;
    summarize_sync_report(preflight_summary(results, preflight_report))
}

fn apply_external_memory_sync_origin(
    config: &mut ExternalMemoryProvidersConfig,
    origin: ExternalMemoryProviderSyncOrigin,
) {
    if !matches!(origin, ExternalMemoryProviderSyncOrigin::Automatic) {
        return;
    }
    for provider in &mut config.providers {
        if !matches!(
            provider.sync_policy,
            ha_core::memory::ExternalMemorySyncPolicy::PullOnly
                | ha_core::memory::ExternalMemorySyncPolicy::PushOnly
                | ha_core::memory::ExternalMemorySyncPolicy::Bidirectional
        ) {
            provider.enabled = false;
        }
    }
}

fn acquire_external_memory_state_lock() -> Result<File> {
    let lock_path = external_memory_sync_lock_path()?;
    acquire_external_memory_state_lock_at(&lock_path)
}

fn acquire_external_memory_state_lock_at(lock_path: &Path) -> Result<File> {
    let started = Instant::now();
    loop {
        match ha_core::platform::try_acquire_exclusive_lock(lock_path)
            .context("lock external memory provider state")?
        {
            Some(file) => return Ok(file),
            None if started.elapsed() < PROVIDER_SYNC_LOCK_TIMEOUT => {
                std::thread::sleep(PROVIDER_SYNC_LOCK_POLL)
            }
            None => bail!("timed out waiting for the external memory provider state lock"),
        }
    }
}

async fn acquire_external_memory_state_lock_async() -> Result<File> {
    let lock_path = external_memory_sync_lock_path()?;
    acquire_external_memory_state_lock_at_async(lock_path).await
}

async fn acquire_external_memory_state_lock_at_async(lock_path: PathBuf) -> Result<File> {
    ha_core::blocking::run_blocking(move || acquire_external_memory_state_lock_at(&lock_path)).await
}

fn sync_lock_failure_report(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
    error: String,
) -> ExternalMemoryProviderSyncReport {
    sync_preflight_failure_report(
        config,
        stats,
        stats_error,
        format!("external memory sync lock unavailable: {error}"),
    )
}

fn sync_config_reload_failure_report(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
    error: String,
) -> ExternalMemoryProviderSyncReport {
    sync_preflight_failure_report(
        config,
        stats,
        stats_error,
        format!("external memory sync configuration unavailable: {error}"),
    )
}

fn sync_preflight_failure_report(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
    failure: String,
) -> ExternalMemoryProviderSyncReport {
    let preflight = ha_core::memory::types::external_memory_sync_preflight_with_stats_status(
        &config,
        &stats,
        stats_error,
    );
    let providers = preflight
        .providers
        .iter()
        .cloned()
        .map(|provider| {
            let status = match provider.action {
                ExternalMemoryProviderPreflightAction::Off => ExternalMemoryProviderSyncStatus::Off,
                ExternalMemoryProviderPreflightAction::Blocked => {
                    ExternalMemoryProviderSyncStatus::Blocked
                }
                ExternalMemoryProviderPreflightAction::WouldSync => {
                    ExternalMemoryProviderSyncStatus::Failed
                }
            };
            let error =
                (status == ExternalMemoryProviderSyncStatus::Failed).then(|| failure.clone());
            ExternalMemoryProviderSyncResult {
                id: provider.id.clone(),
                kind: provider.kind,
                display_name: provider.display_name.clone(),
                status,
                external_io_performed: false,
                preflight: provider,
                imported_memory_count: 0,
                exported_memory_count: 0,
                updated_memory_count: 0,
                skipped_memory_count: 0,
                error,
            }
        })
        .collect();
    summarize_sync_report(preflight_summary(providers, preflight))
}

/// Debounced automatic sync trigger for local memory writes. Manual providers
/// are stripped from the execution snapshot, so this can never turn an owner-
/// initiated policy into background network traffic.
pub fn schedule_external_memory_provider_sync() {
    if !ha_core::runtime_lock::is_primary() || !has_automatic_provider() {
        return;
    }
    AUTO_SYNC_DIRTY.store(true, Ordering::Release);
    if AUTO_SYNC_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        AUTO_SYNC_QUEUED.store(false, Ordering::Release);
        return;
    };
    handle.spawn(async {
        while AUTO_SYNC_DIRTY.swap(false, Ordering::AcqRel) {
            tokio::time::sleep(Duration::from_secs(3)).await;
            run_automatic_external_memory_provider_sync().await;
        }
        AUTO_SYNC_QUEUED.store(false, Ordering::Release);
        // Close the race where a write marks dirty after the loop's last swap
        // but before QUEUED becomes false.
        if AUTO_SYNC_DIRTY.load(Ordering::Acquire) {
            schedule_external_memory_provider_sync();
        }
    });
}

/// Primary-only periodic pull/reconcile loop. This covers pull-only providers
/// even when no local memory write occurs and gives transient failures another
/// chance without making chat latency depend on a remote service.
pub fn spawn_external_memory_provider_sync_loop() {
    if !ha_core::runtime_lock::is_primary() {
        return;
    }
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let mut ticker = tokio::time::interval(Duration::from_secs(300));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            run_automatic_external_memory_provider_sync().await;
        }
    });
}

async fn run_automatic_external_memory_provider_sync() {
    let mut config = ha_core::config::cached_config().memory_providers.clone();
    // This snapshot is only a failure-report projection. Do not return early
    // from it: a sibling desktop/server/ACP process may have enabled automatic
    // sync since this process last refreshed its cache. The execution path
    // acquires the shared state lock and reapplies this filter to authoritative
    // config.json before deciding whether any provider is runnable.
    apply_external_memory_sync_origin(&mut config, ExternalMemoryProviderSyncOrigin::Automatic);
    let (stats, stats_error) = ha_core::blocking::run_blocking(
        ha_core::memory::helpers::external_memory_provider_stats_for_planning,
    )
    .await;
    let report = execute_external_memory_provider_sync_for_origin(
        config,
        stats,
        stats_error,
        ExternalMemoryProviderSyncOrigin::Automatic,
    )
    .await;
    if report.failed_provider_count > 0 {
        app_warn!(
            "memory",
            "external_provider_auto_sync_failed",
            "External memory automatic sync completed with {} failed provider(s)",
            report.failed_provider_count
        );
    }
}

fn has_automatic_provider() -> bool {
    let config = ha_core::config::cached_config();
    config.memory_providers.enabled
        && config.memory_providers.providers.iter().any(|provider| {
            provider.enabled
                && ha_core::memory::types::external_provider_capabilities(provider.kind)
                    .adapter_available
                && matches!(
                    provider.sync_policy,
                    ha_core::memory::ExternalMemorySyncPolicy::PullOnly
                        | ha_core::memory::ExternalMemorySyncPolicy::PushOnly
                        | ha_core::memory::ExternalMemorySyncPolicy::Bidirectional
                )
        })
}

struct SyncReportParts {
    generated_at: String,
    global_enabled: bool,
    local_memory_total: usize,
    local_memory_with_embedding: usize,
    stats_unavailable: bool,
    stats_error: Option<String>,
    runnable_provider_count: usize,
    providers: Vec<ExternalMemoryProviderSyncResult>,
}

fn preflight_summary(
    providers: Vec<ExternalMemoryProviderSyncResult>,
    preflight: ha_core::memory::ExternalMemoryProviderPreflightReport,
) -> SyncReportParts {
    SyncReportParts {
        generated_at: preflight.generated_at,
        global_enabled: preflight.global_enabled,
        local_memory_total: preflight.local_memory_total,
        local_memory_with_embedding: preflight.local_memory_with_embedding,
        stats_unavailable: preflight.stats_unavailable,
        stats_error: preflight.stats_error,
        runnable_provider_count: preflight.runnable_provider_count,
        providers,
    }
}

fn summarize_sync_report(parts: SyncReportParts) -> ExternalMemoryProviderSyncReport {
    let external_io_performed = parts
        .providers
        .iter()
        .any(|provider| provider.external_io_performed);
    let executed_provider_count = parts
        .providers
        .iter()
        .filter(|provider| provider.external_io_performed)
        .count();
    let succeeded_provider_count = parts
        .providers
        .iter()
        .filter(|provider| provider.status == ExternalMemoryProviderSyncStatus::Succeeded)
        .count();
    let failed_provider_count = parts
        .providers
        .iter()
        .filter(|provider| provider.status == ExternalMemoryProviderSyncStatus::Failed)
        .count();
    let blocked_provider_count = parts
        .providers
        .iter()
        .filter(|provider| {
            matches!(
                provider.status,
                ExternalMemoryProviderSyncStatus::Blocked
                    | ExternalMemoryProviderSyncStatus::NoRuntimeAdapter
            )
        })
        .count();
    ExternalMemoryProviderSyncReport {
        generated_at: parts.generated_at,
        global_enabled: parts.global_enabled,
        external_io_performed,
        local_memory_total: parts.local_memory_total,
        local_memory_with_embedding: parts.local_memory_with_embedding,
        stats_unavailable: parts.stats_unavailable,
        stats_error: parts.stats_error,
        runnable_provider_count: parts.runnable_provider_count,
        blocked_provider_count,
        executed_provider_count,
        succeeded_provider_count,
        failed_provider_count,
        providers: parts.providers,
    }
}

async fn execute_provider_sync(
    provider: &ExternalMemoryProviderConfig,
    preflight: ha_core::memory::ExternalMemoryProviderPreflight,
) -> ExternalMemoryProviderSyncResult {
    let outcome = match adapter_for(provider.kind) {
        Some(adapter) => {
            debug_assert_eq!(adapter.kind(), provider.kind);
            let deadline = std::time::Instant::now() + PROVIDER_SYNC_TIMEOUT;
            // Do not cancel the adapter future at the aggregate deadline:
            // claim imports and ledger checkpoints use spawn_blocking and can
            // outlive a dropped future. HTTP request boundaries consult this
            // task-local deadline and stop starting new remote operations,
            // while the current request/checkpoint is allowed to finish under
            // the per-request timeout before the global sync lock is released.
            PROVIDER_SYNC_DEADLINE
                .scope(deadline, adapter.sync(provider))
                .await
        }
        None => Err(ExternalMemoryAdapterSyncFailure {
            outcome: ExternalMemoryAdapterSyncOutcome::default(),
            error: anyhow!("external memory provider runtime adapter is not wired"),
        }),
    };
    match outcome {
        Ok(outcome) => ExternalMemoryProviderSyncResult {
            id: provider.id.clone(),
            kind: provider.kind,
            display_name: provider.display_name.clone(),
            status: ExternalMemoryProviderSyncStatus::Succeeded,
            external_io_performed: outcome.external_io_performed,
            preflight,
            imported_memory_count: outcome.imported_memory_count,
            exported_memory_count: outcome.exported_memory_count,
            updated_memory_count: outcome.updated_memory_count,
            skipped_memory_count: outcome.skipped_memory_count,
            error: None,
        },
        Err(failure) => ExternalMemoryProviderSyncResult {
            id: provider.id.clone(),
            kind: provider.kind,
            display_name: provider.display_name.clone(),
            status: if failure.error.to_string().contains("adapter is not wired") {
                ExternalMemoryProviderSyncStatus::NoRuntimeAdapter
            } else {
                ExternalMemoryProviderSyncStatus::Failed
            },
            external_io_performed: failure.outcome.external_io_performed,
            preflight,
            imported_memory_count: failure.outcome.imported_memory_count,
            exported_memory_count: failure.outcome.exported_memory_count,
            updated_memory_count: failure.outcome.updated_memory_count,
            skipped_memory_count: failure.outcome.skipped_memory_count,
            error: Some(truncate_error(&failure.error.to_string())),
        },
    }
}

pub(super) fn ensure_provider_sync_request_budget() -> Result<()> {
    let exceeded = PROVIDER_SYNC_DEADLINE
        .try_with(|deadline| std::time::Instant::now() >= *deadline)
        .unwrap_or(false);
    if exceeded {
        bail!("external memory provider sync reached its request budget");
    }
    Ok(())
}

fn adapter_for(
    kind: ExternalMemoryProviderKind,
) -> Option<&'static dyn ExternalMemoryProviderAdapter> {
    match kind {
        ExternalMemoryProviderKind::Mem0 => Some(&mem0::MEM0_ADAPTER),
        ExternalMemoryProviderKind::Zep => Some(&zep::ZEP_ADAPTER),
        ExternalMemoryProviderKind::Supermemory => Some(&supermemory::SUPERMEMORY_ADAPTER),
        ExternalMemoryProviderKind::Honcho => Some(&honcho::HONCHO_ADAPTER),
        ExternalMemoryProviderKind::Hindsight => Some(&hindsight::HINDSIGHT_ADAPTER),
        ExternalMemoryProviderKind::OpenViking => Some(&open_viking::OPEN_VIKING_ADAPTER),
        ExternalMemoryProviderKind::Custom => Some(&custom::CUSTOM_ADAPTER),
    }
}

/// Owner-triggered network probe. Configuration preflight deliberately never
/// calls this function: version/capability IO happens only after an explicit UI
/// or owner HTTP action.
pub async fn test_external_memory_provider_connection(
    provider_id: String,
) -> Result<ExternalMemoryProviderCompatibilityReport> {
    validate_provider_id(&provider_id)?;
    // A probe reads credentials and publishes compatibility evidence. Keep
    // that full lifecycle in the same transaction as sync and owner mutation,
    // so cleared/replaced credentials cannot be used after their mutation.
    let _cross_process_guard = acquire_external_memory_state_lock_async().await?;
    let config = ha_core::blocking::run_blocking(|| {
        ha_core::config::reload_config_snapshot_from_disk()
            .context("reload external memory provider configuration")
    })
    .await?;
    let provider = config
        .memory_providers
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned()
        .ok_or_else(|| anyhow!("external memory provider not found"))?;
    let (credentials, _) = resolve_external_memory_provider_credentials_async(&provider_id)
        .await?
        .ok_or_else(|| anyhow!("provider credentials are missing"))?;
    let credential_fingerprint = compatibility_credential_fingerprint(provider.kind, &credentials)?;
    let requirement = compatibility_requirement(provider.kind, Some(&credentials));
    let checked_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let endpoint = match http::validated_endpoint(&credentials.endpoint).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let report =
                compatibility_failure_report(&provider, requirement, checked_at, false, &error);
            persist_compatibility_report_async(report.clone(), credential_fingerprint).await?;
            return Ok(report);
        }
    };
    let probe_url = compatibility_probe_url(provider.kind, &endpoint)?;
    http::validated_endpoint(&probe_url).await?;
    let client = http::client()?;
    let request = compatibility_probe_auth(client.get(&probe_url), provider.kind, &credentials);
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        http::send_probe(request, compatibility_version_headers(provider.kind)),
    )
    .await;
    let report = match response {
        Ok(Ok(response)) => {
            let detected_version =
                detect_provider_version(&response.body, &response.version_headers);
            let status = match (requirement, detected_version.as_deref()) {
                (None, _) => ExternalMemoryProviderCompatibilityStatus::NotRequired,
                (Some(requirement), Some(version))
                    if version_meets_minimum(version, requirement.minimum) =>
                {
                    ExternalMemoryProviderCompatibilityStatus::Compatible
                }
                (Some(_), Some(_)) => ExternalMemoryProviderCompatibilityStatus::Blocked,
                (Some(_), None) => ExternalMemoryProviderCompatibilityStatus::Unverified,
            };
            ExternalMemoryProviderCompatibilityReport {
                provider_id: provider.id.clone(),
                kind: provider.kind,
                status,
                checked_at,
                external_io_performed: true,
                detected_version,
                minimum_version: requirement.map(|item| item.minimum.to_string()),
                recommended_version: requirement.map(|item| item.recommended.to_string()),
                capabilities: detected_provider_capabilities(provider.kind, &response.body),
                error: None,
            }
        }
        Ok(Err(error)) => {
            compatibility_failure_report(&provider, requirement, checked_at, true, &error)
        }
        Err(_) => compatibility_failure_report(
            &provider,
            requirement,
            checked_at,
            true,
            &anyhow!("external memory provider compatibility probe timed out"),
        ),
    };
    persist_compatibility_report_async(report.clone(), credential_fingerprint).await?;
    Ok(report)
}

pub fn test_connection_boxed(
    provider_id: String,
) -> ha_core::memory::external_provider::TestConnectionFuture {
    Box::pin(test_external_memory_provider_connection(provider_id))
}

fn compatibility_credential_fingerprint(
    kind: ExternalMemoryProviderKind,
    credentials: &ExternalMemoryProviderCredentials,
) -> Result<String> {
    let material = serde_json::to_vec(&serde_json::json!({
        "kind": kind.as_str(),
        "endpoint": credentials.endpoint,
        "apiKey": credentials.api_key,
        "subjectId": credentials.subject_id,
        "protocol": credentials.protocol,
    }))
    .context("serialize external memory compatibility fingerprint")?;
    Ok(format!("{:x}", Sha256::digest(material)))
}

fn compatibility_failure_report(
    provider: &ExternalMemoryProviderConfig,
    requirement: Option<VersionRequirement>,
    checked_at: String,
    external_io_performed: bool,
    error: &anyhow::Error,
) -> ExternalMemoryProviderCompatibilityReport {
    ExternalMemoryProviderCompatibilityReport {
        provider_id: provider.id.clone(),
        kind: provider.kind,
        status: ExternalMemoryProviderCompatibilityStatus::Unverified,
        checked_at,
        external_io_performed,
        detected_version: None,
        minimum_version: requirement.map(|item| item.minimum.to_string()),
        recommended_version: requirement.map(|item| item.recommended.to_string()),
        capabilities: Vec::new(),
        error: Some(truncate_error(&error.to_string())),
    }
}

fn compatibility_probe_url(kind: ExternalMemoryProviderKind, endpoint: &str) -> Result<String> {
    match kind {
        ExternalMemoryProviderKind::Zep => http::endpoint_with_path(endpoint, &["healthcheck"]),
        ExternalMemoryProviderKind::OpenViking => {
            http::endpoint_with_path(endpoint, &["api", "v1", "health"])
        }
        ExternalMemoryProviderKind::Custom => Ok(endpoint.to_string()),
        _ => http::endpoint_with_path(endpoint, &["health"]),
    }
}

fn compatibility_probe_auth(
    request: reqwest::RequestBuilder,
    kind: ExternalMemoryProviderKind,
    credentials: &ExternalMemoryProviderCredentials,
) -> reqwest::RequestBuilder {
    let Some(api_key) = credentials.api_key.as_deref() else {
        return request;
    };
    match kind {
        ExternalMemoryProviderKind::Mem0
            if matches!(
                credentials.protocol.as_str(),
                "platform" | "platform_v3" | "cloud" | "cloud_v3"
            ) =>
        {
            request.header(reqwest::header::AUTHORIZATION, format!("Token {api_key}"))
        }
        ExternalMemoryProviderKind::Mem0 => request.header("X-API-Key", api_key),
        _ => request.bearer_auth(api_key),
    }
}

fn compatibility_version_headers(kind: ExternalMemoryProviderKind) -> &'static [&'static str] {
    match kind {
        ExternalMemoryProviderKind::Mem0 => &["x-mem0-version"],
        ExternalMemoryProviderKind::Zep => &["x-zep-version", "x-graphiti-version"],
        ExternalMemoryProviderKind::Supermemory => &["x-supermemory-version"],
        ExternalMemoryProviderKind::Honcho => &["x-honcho-version"],
        ExternalMemoryProviderKind::Hindsight => &["x-hindsight-version"],
        ExternalMemoryProviderKind::OpenViking => &["x-openviking-version"],
        ExternalMemoryProviderKind::Custom => &[],
    }
}

fn compatibility_requirement(
    kind: ExternalMemoryProviderKind,
    credentials: Option<&ExternalMemoryProviderCredentials>,
) -> Option<VersionRequirement> {
    match kind {
        ExternalMemoryProviderKind::Zep => Some(VersionRequirement {
            minimum: "0.28.2",
            recommended: "0.29.3",
        }),
        ExternalMemoryProviderKind::Supermemory
            if credentials.is_none_or(supermemory_is_self_hosted) =>
        {
            Some(VersionRequirement {
                minimum: "0.0.8",
                recommended: "0.0.8",
            })
        }
        ExternalMemoryProviderKind::Honcho if credentials.is_none_or(honcho_is_self_hosted) => {
            Some(VersionRequirement {
                minimum: "3.0.12",
                recommended: "3.0.12",
            })
        }
        ExternalMemoryProviderKind::OpenViking => Some(VersionRequirement {
            minimum: "0.4.15",
            recommended: "0.4.15",
        }),
        _ => None,
    }
}

fn supermemory_is_self_hosted(credentials: &ExternalMemoryProviderCredentials) -> bool {
    match credentials.protocol.as_str() {
        // The protocol selector does not authenticate the deployment. Treat a
        // hosted wire shape on any non-official endpoint as self-hosted so it
        // cannot bypass the version/capability gate.
        "platform" | "cloud" => !endpoint_host_ends_with(&credentials.endpoint, "supermemory.ai"),
        "self_hosted" | "self-hosted" => true,
        _ => !endpoint_host_ends_with(&credentials.endpoint, "supermemory.ai"),
    }
}

fn honcho_is_self_hosted(credentials: &ExternalMemoryProviderCredentials) -> bool {
    match credentials.protocol.as_str() {
        "v3" | "cloud" => !endpoint_host_ends_with(&credentials.endpoint, "honcho.dev"),
        "self_hosted" | "self-hosted" => true,
        _ => !endpoint_host_ends_with(&credentials.endpoint, "honcho.dev"),
    }
}

fn endpoint_host_ends_with(endpoint: &str, suffix: &str) -> bool {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == suffix || host.ends_with(&format!(".{suffix}")))
}

fn detect_provider_version(body: &[u8], version_headers: &[String]) -> Option<String> {
    let json = serde_json::from_slice::<serde_json::Value>(body).ok();
    let body_candidates = json.as_ref().into_iter().flat_map(|value| {
        [
            "/version",
            "/data/version",
            "/info/version",
            "/server/version",
            "/build/version",
            "/app/version",
        ]
        .into_iter()
        .filter_map(|pointer| value.pointer(pointer).and_then(serde_json::Value::as_str))
    });
    body_candidates
        .chain(version_headers.iter().map(String::as_str))
        .find_map(extract_version)
}

fn extract_version(value: &str) -> Option<String> {
    static VERSION_RE: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new(r"(?i)\bv?(\d+)\.(\d+)\.(\d+)(?:[-+][0-9a-z.-]+)?\b")
            .expect("valid provider version regex")
    });
    VERSION_RE
        .find(value)
        .map(|matched| matched.as_str().trim_start_matches(['v', 'V']).to_string())
}

fn parse_version(value: &str) -> Option<ParsedVersion> {
    static VERSION_RE: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new(r"(?i)^v?(\d+)\.(\d+)\.(\d+)([-+][0-9a-z.-]+)?$")
            .expect("valid exact provider version regex")
    });
    let captures = VERSION_RE.captures(value.trim())?;
    Some(ParsedVersion {
        major: captures.get(1)?.as_str().parse().ok()?,
        minor: captures.get(2)?.as_str().parse().ok()?,
        patch: captures.get(3)?.as_str().parse().ok()?,
        prerelease: captures
            .get(4)
            .is_some_and(|suffix| suffix.as_str().starts_with('-')),
    })
}

fn version_meets_minimum(detected: &str, minimum: &str) -> bool {
    let (Some(detected), Some(minimum)) = (parse_version(detected), parse_version(minimum)) else {
        return false;
    };
    let detected_core = (detected.major, detected.minor, detected.patch);
    let minimum_core = (minimum.major, minimum.minor, minimum.patch);
    detected_core > minimum_core || (detected_core == minimum_core && !detected.prerelease)
}

fn detected_provider_capabilities(kind: ExternalMemoryProviderKind, body: &[u8]) -> Vec<String> {
    let mut capabilities = vec![
        "pull".to_string(),
        "push".to_string(),
        "bidirectional".to_string(),
        format!("adapter:{}", kind.as_str()),
    ];
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(object) = value
            .get("capabilities")
            .and_then(serde_json::Value::as_object)
        {
            capabilities.extend(
                object
                    .iter()
                    .filter(|(_, value)| value.as_bool() == Some(true) || value.as_str().is_some())
                    .map(|(key, _)| key.chars().take(64).collect::<String>())
                    .take(28),
            );
        }
    }
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

pub fn external_memory_provider_compatibility_snapshot(
    provider: &ExternalMemoryProviderConfig,
) -> ExternalMemoryProviderCompatibilityReport {
    let current_credentials = resolve_external_memory_provider_credentials(&provider.id)
        .ok()
        .flatten()
        .map(|(credentials, _)| credentials);
    let current_fingerprint = current_credentials.as_ref().and_then(|credentials| {
        compatibility_credential_fingerprint(provider.kind, credentials).ok()
    });
    let current_requirement =
        compatibility_requirement(provider.kind, current_credentials.as_ref());
    match load_compatibility_report(&provider.id) {
        Ok(Some(stored))
            if stored.report.provider_id == provider.id
                && stored.report.kind == provider.kind
                && !stored.credential_fingerprint.is_empty()
                && current_fingerprint.as_deref()
                    == Some(stored.credential_fingerprint.as_str())
                && compatibility_report_is_current(
                    &stored.report,
                    current_requirement,
                    chrono::Utc::now(),
                ) =>
        {
            stored.report
        }
        Ok(_) => default_compatibility_report(provider, None),
        Err(error) => default_compatibility_report(provider, Some(&error)),
    }
}

fn compatibility_report_is_current(
    report: &ExternalMemoryProviderCompatibilityReport,
    requirement: Option<VersionRequirement>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if report.minimum_version.as_deref() != requirement.map(|item| item.minimum) {
        return false;
    }

    match report.status {
        ExternalMemoryProviderCompatibilityStatus::Compatible => {
            let (Some(requirement), Some(detected_version)) =
                (requirement, report.detected_version.as_deref())
            else {
                return false;
            };
            if !version_meets_minimum(detected_version, requirement.minimum) {
                return false;
            }
            let Ok(checked_at) = chrono::DateTime::parse_from_rfc3339(&report.checked_at) else {
                return false;
            };
            let checked_at = checked_at.with_timezone(&chrono::Utc);
            checked_at <= now
                && now
                    .signed_duration_since(checked_at)
                    .to_std()
                    .is_ok_and(|age| age <= COMPATIBILITY_GRANT_MAX_AGE)
        }
        ExternalMemoryProviderCompatibilityStatus::NotRequired => requirement.is_none(),
        ExternalMemoryProviderCompatibilityStatus::Unverified
        | ExternalMemoryProviderCompatibilityStatus::Blocked => true,
    }
}

fn default_compatibility_report(
    provider: &ExternalMemoryProviderConfig,
    error: Option<&anyhow::Error>,
) -> ExternalMemoryProviderCompatibilityReport {
    let credentials = resolve_external_memory_provider_credentials(&provider.id)
        .ok()
        .flatten()
        .map(|(credentials, _)| credentials);
    let requirement = compatibility_requirement(provider.kind, credentials.as_ref());
    ExternalMemoryProviderCompatibilityReport {
        provider_id: provider.id.clone(),
        kind: provider.kind,
        status: if requirement.is_some() {
            ExternalMemoryProviderCompatibilityStatus::Unverified
        } else {
            ExternalMemoryProviderCompatibilityStatus::NotRequired
        },
        checked_at: String::new(),
        external_io_performed: false,
        detected_version: None,
        minimum_version: requirement.map(|item| item.minimum.to_string()),
        recommended_version: requirement.map(|item| item.recommended.to_string()),
        capabilities: Vec::new(),
        error: error.map(|error| truncate_error(&error.to_string())),
    }
}

fn load_compatibility_report(
    provider_id: &str,
) -> Result<Option<StoredExternalMemoryProviderCompatibility>> {
    validate_provider_id(provider_id)?;
    let path = external_memory_compatibility_path(provider_id)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(anyhow!("read {}: {error}", path.display())),
    };
    let stored: StoredExternalMemoryProviderCompatibility = serde_json::from_slice(&bytes)
        .map_err(|error| anyhow!("parse {}: {error}", path.display()))?;
    if stored.schema_version != COMPATIBILITY_SCHEMA_VERSION {
        bail!(
            "unsupported external memory compatibility schema version {}",
            stored.schema_version
        );
    }
    Ok(Some(stored))
}

async fn persist_compatibility_report_async(
    report: ExternalMemoryProviderCompatibilityReport,
    credential_fingerprint: String,
) -> Result<()> {
    ha_core::blocking::run_blocking(move || {
        let _provider_write_guard = EXTERNAL_PROVIDER_CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| anyhow!("external memory provider config write lock poisoned"))?;
        validate_provider_id(&report.provider_id)?;
        let live_provider = ha_core::config::cached_config()
            .memory_providers
            .providers
            .iter()
            .find(|provider| provider.id == report.provider_id)
            .cloned()
            .ok_or_else(|| {
                anyhow!("external memory provider changed during compatibility probe")
            })?;
        let live_credentials = resolve_external_memory_provider_credentials(&report.provider_id)?
            .map(|(credentials, _)| credentials)
            .ok_or_else(|| anyhow!("provider credentials changed during compatibility probe"))?;
        let live_fingerprint =
            compatibility_credential_fingerprint(live_provider.kind, &live_credentials)?;
        if live_provider.kind != report.kind || live_fingerprint != credential_fingerprint {
            bail!("external memory provider changed during compatibility probe");
        }
        let path = external_memory_compatibility_path(&report.provider_id)?;
        let stored = StoredExternalMemoryProviderCompatibility {
            schema_version: COMPATIBILITY_SCHEMA_VERSION,
            credential_fingerprint,
            report,
        };
        let bytes = serde_json::to_vec_pretty(&stored)
            .context("serialize external memory compatibility report")?;
        write_secure_file(&path, &bytes)
            .map_err(|error| anyhow!("write {}: {error}", path.display()))
    })
    .await
}

pub fn hydrate_external_memory_provider_config(
    mut config: ExternalMemoryProvidersConfig,
) -> ExternalMemoryProvidersConfig {
    for provider in &mut config.providers {
        match resolve_external_memory_provider_credentials(&provider.id) {
            Ok(Some(_)) => provider.endpoint_configured = true,
            Ok(None) => provider.endpoint_configured = false,
            Err(err) => {
                provider.endpoint_configured = false;
                provider.last_error = Some(truncate_error(&err.to_string()));
            }
        }
    }
    config
}

fn persist_sync_health(results: &[ExternalMemoryProviderSyncResult]) {
    let updates = results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                ExternalMemoryProviderSyncStatus::Succeeded
                    | ExternalMemoryProviderSyncStatus::Failed
            )
        })
        .map(|result| {
            (
                result.id.clone(),
                result.status.clone(),
                result.error.clone(),
            )
        })
        .collect::<Vec<_>>();
    if updates.is_empty() {
        return;
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let persist_result = (|| -> Result<()> {
        // Sync may hold the provider state lock across network IO. Refresh
        // immediately before the config read-modify-write so unrelated
        // settings committed by another process are not overwritten by the
        // old process-local cache.
        ha_core::config::reload_config_snapshot_from_disk()
            .context("reload configuration before external memory sync health update")?;
        ha_core::config::mutate_config(("memory_providers.sync", "owner"), move |store| {
            for (id, status, error) in &updates {
                let Some(provider) = store
                    .memory_providers
                    .providers
                    .iter_mut()
                    .find(|provider| provider.id == *id)
                else {
                    continue;
                };
                match status {
                    ExternalMemoryProviderSyncStatus::Succeeded => {
                        provider.last_sync_at = Some(now.clone());
                        provider.last_error = None;
                    }
                    ExternalMemoryProviderSyncStatus::Failed => {
                        provider.last_error = error.clone();
                    }
                    _ => {}
                }
            }
            Ok(())
        })
    })();
    if let Err(err) = persist_result {
        app_warn!(
            "memory",
            "external_provider_sync_health_persist_failed",
            "Failed to persist external memory provider sync health: {}",
            truncate_error(&err.to_string())
        );
    }
}

pub(crate) fn load_sync_ledger(provider_id: &str) -> Result<ExternalMemoryProviderSyncLedger> {
    validate_provider_id(provider_id)?;
    let path = external_memory_sync_state_path(provider_id)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(ExternalMemoryProviderSyncLedger {
                schema_version: SYNC_STATE_SCHEMA_VERSION,
                ..Default::default()
            })
        }
        Err(err) => return Err(anyhow!("read {}: {err}", path.display())),
    };
    let ledger: ExternalMemoryProviderSyncLedger =
        serde_json::from_slice(&bytes).map_err(|err| anyhow!("parse {}: {err}", path.display()))?;
    if ledger.schema_version != SYNC_STATE_SCHEMA_VERSION {
        bail!(
            "unsupported external memory sync state schema version {}",
            ledger.schema_version
        );
    }
    Ok(ledger)
}

pub(crate) fn persist_sync_ledger(
    provider_id: &str,
    ledger: &ExternalMemoryProviderSyncLedger,
) -> Result<()> {
    validate_provider_id(provider_id)?;
    let path = external_memory_sync_state_path(provider_id)?;
    let bytes = serde_json::to_vec_pretty(ledger).context("serialize provider sync state")?;
    write_secure_file(&path, &bytes).map_err(|err| anyhow!("write {}: {err}", path.display()))
}

pub(crate) async fn resolve_external_memory_provider_credentials_async(
    provider_id: &str,
) -> Result<Option<(ExternalMemoryProviderCredentials, &'static str)>> {
    let provider_id = provider_id.to_string();
    ha_core::blocking::run_blocking(move || {
        resolve_external_memory_provider_credentials(&provider_id)
    })
    .await
}

pub(crate) async fn load_sync_ledger_async(
    provider_id: &str,
) -> Result<ExternalMemoryProviderSyncLedger> {
    let provider_id = provider_id.to_string();
    ha_core::blocking::run_blocking(move || load_sync_ledger(&provider_id)).await
}

pub(crate) async fn persist_sync_ledger_async(
    provider_id: &str,
    ledger: &ExternalMemoryProviderSyncLedger,
) -> Result<()> {
    let provider_id = provider_id.to_string();
    let ledger = ledger.clone();
    ha_core::blocking::run_blocking(move || persist_sync_ledger(&provider_id, &ledger)).await
}

pub(crate) async fn finish_sync_with_ledger_checkpoint(
    provider_id: &str,
    ledger: &ExternalMemoryProviderSyncLedger,
    outcome: ExternalMemoryAdapterSyncOutcome,
    sync_result: std::result::Result<(), ExternalMemoryAdapterSyncFailure>,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let checkpoint_result = persist_sync_ledger_async(provider_id, ledger).await;
    finish_sync_after_checkpoint(outcome, sync_result, checkpoint_result)
}

fn finish_sync_after_checkpoint(
    outcome: ExternalMemoryAdapterSyncOutcome,
    sync_result: std::result::Result<(), ExternalMemoryAdapterSyncFailure>,
    checkpoint_result: Result<()>,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    match (sync_result, checkpoint_result) {
        (Ok(()), Ok(())) => Ok(outcome),
        (Err(failure), Ok(())) => Err(failure),
        (Ok(()), Err(error)) => Err(ExternalMemoryAdapterSyncFailure { outcome, error }),
        (Err(failure), Err(checkpoint_error)) => Err(ExternalMemoryAdapterSyncFailure {
            outcome: failure.outcome,
            error: anyhow!(
                "{}; additionally failed to persist sync ledger: {}",
                failure.error,
                checkpoint_error
            ),
        }),
    }
}

pub(crate) async fn load_local_memory_snapshot(
    scan_limit: usize,
) -> Result<(Vec<ha_core::memory::MemoryEntry>, usize)> {
    let backend =
        ha_core::get_memory_backend().ok_or_else(|| anyhow!("memory backend unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let total = backend.count(None)?;
        let mut entries = Vec::new();
        let mut offset = 0usize;
        while entries.len() < scan_limit {
            let limit = 500usize.min(scan_limit - entries.len());
            let page = backend.list(None, None, limit, offset)?;
            if page.is_empty() {
                break;
            }
            offset += page.len();
            entries.extend(page);
            if entries.len() >= total {
                break;
            }
        }
        Ok::<_, anyhow::Error>((entries, total))
    })
    .await
    .context("join local memory export scan")?
}

pub(crate) async fn import_external_memory_for_review(
    provider: &ExternalMemoryProviderConfig,
    provider_kind: &str,
    remote_id: &str,
    content: &str,
    endpoint: &str,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> Result<bool> {
    let content = truncate_chars(content.trim(), MAX_IMPORTED_CONTENT_CHARS);
    if content.is_empty() {
        outcome.skipped_memory_count += 1;
        return Ok(false);
    }
    let hash = content_fingerprint(&content);
    let old_hash = ledger.imported_hashes.get(remote_id).cloned();
    if old_hash.as_deref() == Some(hash.as_str()) {
        outcome.skipped_memory_count += 1;
        return Ok(false);
    }

    let source_origin = url::Url::parse(endpoint)
        .ok()
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|| "external-memory-provider".to_string());
    let candidate = ha_core::memory::claims::ClaimCandidate {
        claim_type: "reference".to_string(),
        subject: format!("external:{provider_kind}:{}", provider.id),
        predicate: "provided_memory".to_string(),
        object: content.clone(),
        content,
        scope: None,
        evidence_class: Some("assistant_inferred".to_string()),
        salience: Some(0.5),
        temporal: None,
        evidence_refs: vec![format!("url:{source_origin}")],
        tags: vec![
            "external_provider".to_string(),
            provider_kind.to_string(),
            provider.id.clone(),
        ],
    };
    let provider_id = provider.id.clone();
    let remote_id_owned = remote_id.to_string();
    ha_core::blocking::run_blocking(move || {
        ha_core::memory::claims::write_claim_candidate_with_status(
            &candidate,
            &ha_core::memory::MemoryScope::Global,
            &format!("external-sync:{provider_id}"),
            Some(&remote_id_owned),
            Some("needs_review"),
        )
    })
    .await?;

    ledger.imported_hashes.insert(remote_id.to_string(), hash);
    if old_hash.is_some() {
        outcome.updated_memory_count += 1;
    } else {
        outcome.imported_memory_count += 1;
    }
    let changed = outcome.imported_memory_count + outcome.updated_memory_count;
    if changed % IMPORT_LEDGER_CHECKPOINT_EVERY == 0 {
        persist_sync_ledger_async(&provider.id, ledger).await?;
    }
    Ok(true)
}

pub(crate) fn content_fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn local_memory_fingerprint(memory: &ha_core::memory::MemoryEntry) -> String {
    content_fingerprint(
        &serde_json::to_string(&serde_json::json!({
            "content": memory.content,
            "type": memory.memory_type.as_str(),
            "scope": memory.scope,
            "tags": memory.tags,
            "pinned": memory.pinned,
            "updatedAt": memory.updated_at,
        }))
        .unwrap_or_else(|_| memory.content.clone()),
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn truncate_error(error: &str) -> String {
    ha_core::logging::redact_sensitive(error)
        .chars()
        .take(512)
        .collect()
}

pub async fn save_external_memory_provider_credentials(
    input: ExternalMemoryProviderCredentialInput,
) -> Result<ExternalMemoryProviderCredentialStatus> {
    validate_provider_id(&input.provider_id)?;
    // The existing credential values participate in partial-update semantics,
    // so acquire before reading them and retain the guard through config,
    // credential, ledger and compatibility publication/rollback.
    let _cross_process_guard = acquire_external_memory_state_lock_async().await?;
    let fresh_config = ha_core::blocking::run_blocking(|| {
        ha_core::config::reload_config_snapshot_from_disk()
            .context("reload external memory provider configuration")
    })
    .await?;
    ensure_provider_exists(&input.provider_id)?;

    let provider_id_for_load = input.provider_id.clone();
    let existing =
        ha_core::blocking::run_blocking(move || load_credentials_file(&provider_id_for_load))
            .await?;
    let endpoint = if input.endpoint.trim().is_empty() {
        existing
            .as_ref()
            .map(|credentials| credentials.endpoint.clone())
            .ok_or_else(|| anyhow!("external memory provider endpoint is required"))?
    } else {
        normalize_endpoint(&input.endpoint)?
    };
    let ssrf = fresh_config.ssrf.clone();
    ha_core::security::ssrf::check_url(&endpoint, ssrf.default_policy, &ssrf.trusted_hosts)
        .await
        .context("external memory provider endpoint rejected")?;

    let subject_id = if input.subject_id.trim().is_empty() {
        existing
            .as_ref()
            .map(|credentials| credentials.subject_id.clone())
            .ok_or_else(|| anyhow!("external memory provider subject id is required"))?
    } else {
        normalize_required(&input.subject_id, MAX_SUBJECT_ID_CHARS, "subject id")?
    };
    let protocol = match input.protocol.as_deref() {
        None | Some("") => existing
            .as_ref()
            .map(|credentials| credentials.protocol.clone())
            .unwrap_or_else(|| "auto".to_string()),
        value => normalize_protocol(value)?,
    };
    let reset_sync_ledger = existing.as_ref().is_some_and(|credentials| {
        credentials.endpoint != endpoint
            || credentials.subject_id != subject_id
            || credentials.protocol != protocol
    });
    let api_key = match input.api_key {
        None => existing
            .as_ref()
            .and_then(|credentials| credentials.api_key.clone()),
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(value.trim().to_string()),
    };
    let credentials = ExternalMemoryProviderCredentials {
        schema_version: CREDENTIAL_SCHEMA_VERSION,
        endpoint,
        api_key,
        subject_id,
        protocol,
    };
    let provider_id = input.provider_id;
    ha_core::blocking::run_blocking(move || {
        let _provider_write_guard = EXTERNAL_PROVIDER_CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| anyhow!("external memory provider config write lock poisoned"))?;
        // Endpoint validation above may await DNS/network policy work. Reload
        // again immediately before the config mutation so unrelated settings
        // committed by another process during that wait remain intact.
        ha_core::config::reload_config_snapshot_from_disk()
            .context("reload configuration before external memory credential update")?;
        ensure_provider_exists(&provider_id)?;
        let credential_path = external_memory_credential_path(&provider_id)?;
        let ledger_path = external_memory_sync_state_path(&provider_id)?;
        let compatibility_path = external_memory_compatibility_path(&provider_id)?;
        let previous_credential_bytes = read_optional_file(&credential_path)?;
        let previous_ledger_bytes = if reset_sync_ledger {
            read_optional_file(&ledger_path)?
        } else {
            None
        };
        let previous_compatibility_bytes = if reset_sync_ledger {
            read_optional_file(&compatibility_path)?
        } else {
            None
        };
        persist_credentials(&provider_id, &credentials)?;
        if reset_sync_ledger {
            remove_sync_ledger(&provider_id)?;
            remove_compatibility_report(&provider_id)?;
        }

        let provider_id_for_config = provider_id.clone();
        if let Err(err) = ha_core::config::mutate_config(
            ("memory_providers.credentials", "owner"),
            move |store| {
                let provider = store
                    .memory_providers
                    .providers
                    .iter_mut()
                    .find(|provider| provider.id == provider_id_for_config)
                    .ok_or_else(|| anyhow!("external memory provider not found"))?;
                provider.endpoint_configured = true;
                provider.last_error = None;
                Ok(())
            },
        ) {
            restore_optional_secure_file(&credential_path, previous_credential_bytes.as_deref())?;
            if reset_sync_ledger {
                restore_optional_secure_file(&ledger_path, previous_ledger_bytes.as_deref())?;
                restore_optional_secure_file(
                    &compatibility_path,
                    previous_compatibility_bytes.as_deref(),
                )?;
            }
            return Err(err).context("persist external memory provider readiness");
        }

        Ok(status_from_credentials(provider_id, credentials, "file"))
    })
    .await
}

pub fn save_credentials_boxed(
    input: ExternalMemoryProviderCredentialInput,
) -> ha_core::memory::external_provider::SaveCredentialsFuture {
    Box::pin(save_external_memory_provider_credentials(input))
}

pub fn get_external_memory_provider_credential_status(
    provider_id: &str,
) -> Result<ExternalMemoryProviderCredentialStatus> {
    validate_provider_id(provider_id)?;
    ensure_provider_exists(provider_id)?;
    match resolve_external_memory_provider_credentials(provider_id)? {
        Some((credentials, source)) => Ok(status_from_credentials(
            provider_id.to_string(),
            credentials,
            source,
        )),
        None => Ok(ExternalMemoryProviderCredentialStatus {
            provider_id: provider_id.to_string(),
            configured: false,
            endpoint_configured: false,
            api_key_configured: false,
            endpoint_origin: None,
            subject_id: None,
            protocol: None,
            source: None,
        }),
    }
}

pub fn clear_external_memory_provider_credentials(provider_id: &str) -> Result<()> {
    validate_provider_id(provider_id)?;
    let _cross_process_guard = acquire_external_memory_state_lock()?;
    let _provider_write_guard = EXTERNAL_PROVIDER_CONFIG_WRITE_LOCK
        .lock()
        .map_err(|_| anyhow!("external memory provider config write lock poisoned"))?;
    ha_core::config::reload_config_snapshot_from_disk()
        .context("reload external memory provider configuration")?;
    ensure_provider_exists(provider_id)?;
    let path = external_memory_credential_path(provider_id)?;
    let ledger_path = external_memory_sync_state_path(provider_id)?;
    let compatibility_path = external_memory_compatibility_path(provider_id)?;
    let previous_credential_bytes = read_optional_file(&path)?;
    let previous_ledger_bytes = read_optional_file(&ledger_path)?;
    let previous_compatibility_bytes = read_optional_file(&compatibility_path)?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(anyhow!("remove {}: {err}", path.display())),
    }
    remove_sync_ledger(provider_id)?;
    remove_compatibility_report(provider_id)?;

    let provider_id_owned = provider_id.to_string();
    if let Err(err) =
        ha_core::config::mutate_config(("memory_providers.credentials", "owner"), move |store| {
            let provider = store
                .memory_providers
                .providers
                .iter_mut()
                .find(|provider| provider.id == provider_id_owned)
                .ok_or_else(|| anyhow!("external memory provider not found"))?;
            provider.endpoint_configured = false;
            provider.last_sync_at = None;
            provider.last_error = None;
            Ok(())
        })
    {
        restore_optional_secure_file(&path, previous_credential_bytes.as_deref())?;
        restore_optional_secure_file(&ledger_path, previous_ledger_bytes.as_deref())?;
        restore_optional_secure_file(&compatibility_path, previous_compatibility_bytes.as_deref())?;
        return Err(err).context("clear external memory provider readiness");
    }
    Ok(())
}

fn read_optional_file(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(anyhow!("read {}: {err}", path.display())),
    }
}

fn restore_optional_secure_file(path: &std::path::Path, bytes: Option<&[u8]>) -> Result<()> {
    match bytes {
        Some(bytes) => write_secure_file(path, bytes)
            .map_err(|err| anyhow!("restore {}: {err}", path.display())),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(anyhow!("remove {} during rollback: {err}", path.display())),
        },
    }
}

fn remove_sync_ledger(provider_id: &str) -> Result<()> {
    let path = external_memory_sync_state_path(provider_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(anyhow!("remove {}: {err}", path.display())),
    }
}

fn remove_compatibility_report(provider_id: &str) -> Result<()> {
    let path = external_memory_compatibility_path(provider_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(anyhow!("remove {}: {err}", path.display())),
    }
}

pub fn save_external_memory_providers_config(
    config: ExternalMemoryProvidersConfig,
    source: &'static str,
) -> Result<()> {
    let _cross_process_guard = acquire_external_memory_state_lock()?;
    let _provider_write_guard = EXTERNAL_PROVIDER_CONFIG_WRITE_LOCK
        .lock()
        .map_err(|_| anyhow!("external memory provider config write lock poisoned"))?;
    ha_core::config::reload_config_snapshot_from_disk()
        .context("reload configuration before external memory provider update")?;
    let config = config.normalized();
    let valid_ids = config
        .providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<HashSet<_>>();
    ha_core::config::mutate_config(("memory_providers", source), move |store| {
        store.memory_providers = config;
        Ok(())
    })?;
    if let Err(err) = prune_orphan_provider_files(&valid_ids) {
        app_warn!(
            "memory",
            "external_provider_credentials_prune_failed",
            "Failed to prune orphan external memory provider credentials: {}",
            truncate_error(&err.to_string())
        );
    }
    Ok(())
}

/// Atomically merge a partial owner-plane patch into the non-secret external
/// provider config. Provider entries are merged by id; deletion requires an
/// explicit `removeProviderIds` entry. The process-local lifecycle mutex and
/// process-shared state lock remain held through orphan cleanup so neither a
/// concurrent Settings UI save nor a sync can cross the config commit and
/// credential deletion boundary.
pub fn patch_external_memory_providers_config(
    patch: serde_json::Value,
    source: &str,
) -> Result<ExternalMemoryProvidersConfig> {
    let patch: ExternalMemoryProvidersPatch =
        serde_json::from_value(patch).context("parse external memory providers patch")?;
    let _cross_process_guard = acquire_external_memory_state_lock()?;
    let _provider_write_guard = EXTERNAL_PROVIDER_CONFIG_WRITE_LOCK
        .lock()
        .map_err(|_| anyhow!("external memory provider config write lock poisoned"))?;
    ha_core::config::reload_config_snapshot_from_disk()
        .context("reload configuration before external memory provider patch")?;
    let (config, valid_ids) =
        ha_core::config::mutate_config(("memory_providers", source), move |store| -> Result<_> {
            let config = apply_external_memory_providers_patch(&store.memory_providers, patch)?;
            let valid_ids = config
                .providers
                .iter()
                .map(|provider| provider.id.clone())
                .collect::<HashSet<_>>();
            store.memory_providers = config.clone();
            Ok((config, valid_ids))
        })?;
    if let Err(err) = prune_orphan_provider_files(&valid_ids) {
        app_warn!(
            "memory",
            "external_provider_credentials_prune_failed",
            "Failed to prune orphan external memory provider credentials: {}",
            truncate_error(&err.to_string())
        );
    }
    Ok(config)
}

fn apply_external_memory_providers_patch(
    current: &ExternalMemoryProvidersConfig,
    patch: ExternalMemoryProvidersPatch,
) -> Result<ExternalMemoryProvidersConfig> {
    let ExternalMemoryProvidersPatch {
        enabled,
        providers,
        remove_provider_ids,
    } = patch;
    let mut config = current.clone();
    if let Some(enabled) = enabled {
        config.enabled = enabled;
    }

    let mut remove_ids = HashSet::new();
    for provider_id in remove_provider_ids {
        validate_provider_id(&provider_id)?;
        if !remove_ids.insert(provider_id.clone()) {
            bail!("duplicate external memory provider removal id: {provider_id}");
        }
    }

    let mut patched_ids = HashSet::new();
    for provider_patch in providers {
        validate_provider_id(&provider_patch.id)?;
        if !patched_ids.insert(provider_patch.id.clone()) {
            bail!(
                "duplicate external memory provider patch id: {}",
                provider_patch.id
            );
        }
        if remove_ids.contains(&provider_patch.id) {
            bail!(
                "external memory provider '{}' cannot be patched and removed in one request",
                provider_patch.id
            );
        }

        if let Some(existing) = config
            .providers
            .iter_mut()
            .find(|provider| provider.id == provider_patch.id)
        {
            if let Some(kind) = provider_patch.kind {
                if existing.kind != kind {
                    bail!(
                        "external memory provider '{}' kind is immutable; remove it with `removeProviderIds` and add a new provider so credentials and sync state are cleared",
                        provider_patch.id
                    );
                }
            }
            if let Some(display_name) = provider_patch.display_name {
                existing.display_name = display_name;
            }
            if let Some(enabled) = provider_patch.enabled {
                existing.enabled = enabled;
            }
            if let Some(sync_policy) = provider_patch.sync_policy {
                existing.sync_policy = sync_policy;
            }
            continue;
        }

        let kind = provider_patch.kind.ok_or_else(|| {
            anyhow!(
                "new external memory provider '{}' requires `kind`",
                provider_patch.id
            )
        })?;
        config.providers.push(ExternalMemoryProviderConfig {
            id: provider_patch.id,
            kind,
            display_name: provider_patch
                .display_name
                .unwrap_or_else(|| kind.as_str().to_string()),
            enabled: provider_patch.enabled.unwrap_or(false),
            sync_policy: provider_patch.sync_policy.unwrap_or_default(),
            endpoint_configured: false,
            last_sync_at: None,
            last_error: None,
        });
    }

    config
        .providers
        .retain(|provider| !remove_ids.contains(&provider.id));
    Ok(config.normalized())
}

pub(crate) fn resolve_external_memory_provider_credentials(
    provider_id: &str,
) -> Result<Option<(ExternalMemoryProviderCredentials, &'static str)>> {
    validate_provider_id(provider_id)?;
    let prefix = provider_env_prefix(provider_id);
    let env_endpoint = std::env::var(format!("{prefix}_ENDPOINT")).ok();
    let source = if env_endpoint.is_some() {
        "environment"
    } else {
        "file"
    };
    let file = load_credentials_file(provider_id)?;

    if env_endpoint.is_none() && file.is_none() {
        return Ok(None);
    }

    let endpoint = env_endpoint
        .or_else(|| {
            file.as_ref()
                .map(|credentials| credentials.endpoint.clone())
        })
        .ok_or_else(|| anyhow!("external memory provider endpoint is missing"))?;
    let endpoint = normalize_endpoint(&endpoint)?;
    let subject_id = std::env::var(format!("{prefix}_SUBJECT_ID"))
        .ok()
        .or_else(|| {
            file.as_ref()
                .map(|credentials| credentials.subject_id.clone())
        })
        .ok_or_else(|| anyhow!("external memory provider subject id is missing"))?;
    let subject_id = normalize_required(&subject_id, MAX_SUBJECT_ID_CHARS, "subject id")?;
    let protocol = std::env::var(format!("{prefix}_PROTOCOL"))
        .ok()
        .or_else(|| {
            file.as_ref()
                .map(|credentials| credentials.protocol.clone())
        });
    let protocol = normalize_protocol(protocol.as_deref())?;
    let api_key = std::env::var(format!("{prefix}_API_KEY"))
        .ok()
        .or_else(|| file.and_then(|credentials| credentials.api_key))
        .filter(|value| !value.trim().is_empty());
    Ok(Some((
        ExternalMemoryProviderCredentials {
            schema_version: CREDENTIAL_SCHEMA_VERSION,
            endpoint,
            api_key,
            subject_id,
            protocol,
        },
        source,
    )))
}

fn ensure_provider_exists(provider_id: &str) -> Result<()> {
    if ha_core::config::cached_config()
        .memory_providers
        .providers
        .iter()
        .any(|provider| provider.id == provider_id)
    {
        Ok(())
    } else {
        bail!("external memory provider not found")
    }
}

fn prune_orphan_provider_files(valid_ids: &std::collections::HashSet<String>) -> Result<()> {
    let dir = ha_core::paths::external_memory_credentials_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(anyhow!("read {}: {err}", dir.display())),
    };
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(ToString::to_string) else {
            continue;
        };
        let provider_id = name
            .strip_suffix(".compat.json")
            .or_else(|| name.strip_suffix(".sync.json"))
            .or_else(|| name.strip_suffix(".json"));
        let Some(provider_id) = provider_id else {
            continue;
        };
        if validate_provider_id(provider_id).is_err() || valid_ids.contains(provider_id) {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(anyhow!("remove {}: {err}", entry.path().display())),
        }
    }
    Ok(())
}

fn persist_credentials(
    provider_id: &str,
    credentials: &ExternalMemoryProviderCredentials,
) -> Result<()> {
    let path = external_memory_credential_path(provider_id)?;
    let bytes = serde_json::to_vec_pretty(credentials).context("serialize provider credentials")?;
    write_secure_file(&path, &bytes).map_err(|err| anyhow!("write {}: {err}", path.display()))
}

fn load_credentials_file(provider_id: &str) -> Result<Option<ExternalMemoryProviderCredentials>> {
    let path = external_memory_credential_path(provider_id)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(anyhow!("read {}: {err}", path.display())),
    };
    let credentials: ExternalMemoryProviderCredentials =
        serde_json::from_slice(&bytes).map_err(|err| anyhow!("parse {}: {err}", path.display()))?;
    if credentials.schema_version != CREDENTIAL_SCHEMA_VERSION {
        bail!(
            "unsupported external memory credential schema version {}",
            credentials.schema_version
        );
    }
    Ok(Some(credentials))
}

fn status_from_credentials(
    provider_id: String,
    credentials: ExternalMemoryProviderCredentials,
    source: &str,
) -> ExternalMemoryProviderCredentialStatus {
    let endpoint_origin = url::Url::parse(&credentials.endpoint)
        .ok()
        .map(|url| url.origin().ascii_serialization());
    ExternalMemoryProviderCredentialStatus {
        provider_id,
        configured: true,
        endpoint_configured: true,
        api_key_configured: credentials.api_key.is_some(),
        endpoint_origin,
        subject_id: Some(credentials.subject_id),
        protocol: Some(credentials.protocol),
        source: Some(source.to_string()),
    }
}

fn normalize_endpoint(raw: &str) -> Result<String> {
    let value = normalize_required(raw, MAX_ENDPOINT_CHARS, "endpoint")?;
    let parsed = url::Url::parse(&value).context("invalid external memory provider endpoint")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("external memory provider endpoint must use http or https");
    }
    if parsed.host_str().is_none() {
        bail!("external memory provider endpoint has no host");
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("external memory provider endpoint cannot contain credentials, query, or fragment");
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn normalize_protocol(raw: Option<&str>) -> Result<String> {
    let value = raw.unwrap_or("auto").trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok("auto".to_string());
    }
    if value.len() > MAX_PROTOCOL_CHARS
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        bail!("invalid external memory provider protocol");
    }
    Ok(value)
}

fn normalize_required(raw: &str, max_chars: usize, label: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("external memory provider {label} is required");
    }
    if value.chars().count() > max_chars {
        bail!("external memory provider {label} is too long");
    }
    Ok(value.to_string())
}

fn validate_provider_id(provider_id: &str) -> Result<()> {
    if provider_id.is_empty()
        || provider_id.len() > 64
        || !provider_id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        bail!("invalid external memory provider id");
    }
    Ok(())
}

fn provider_env_prefix(provider_id: &str) -> String {
    let id = provider_id.replace('-', "_").to_ascii_uppercase();
    format!("HOPE_AGENT_EXTERNAL_MEMORY_{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider(id: &str) -> ExternalMemoryProviderConfig {
        ExternalMemoryProviderConfig {
            id: id.to_string(),
            kind: ExternalMemoryProviderKind::Mem0,
            display_name: id.to_string(),
            enabled: false,
            sync_policy: ha_core::memory::ExternalMemorySyncPolicy::Off,
            endpoint_configured: true,
            last_sync_at: Some("2026-07-17T00:00:00Z".to_string()),
            last_error: Some("previous error".to_string()),
        }
    }

    #[test]
    fn provider_id_rejects_path_traversal() {
        assert!(validate_provider_id("mem0-main").is_ok());
        assert!(validate_provider_id("../mem0").is_err());
        assert!(validate_provider_id("Mem0").is_err());
    }

    #[tokio::test]
    async fn sync_and_synchronous_mutation_paths_share_one_os_lock() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("external-memory-sync.lock");
        let first = acquire_external_memory_state_lock_at_async(lock_path.clone())
            .await
            .unwrap();
        let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
        let waiter = tokio::task::spawn_blocking(move || {
            let _second = acquire_external_memory_state_lock_at(&lock_path).unwrap();
            entered_tx.send(()).unwrap();
        });

        tokio::task::yield_now().await;
        assert!(entered_rx.try_recv().is_err());
        drop(first);
        tokio::time::timeout(Duration::from_secs(1), entered_rx.recv())
            .await
            .expect("waiting provider sync should enter after release")
            .expect("waiter should report entry");
        waiter.await.unwrap();
    }

    #[test]
    fn automatic_sync_origin_filters_the_live_policy_snapshot() {
        let mut config = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![
                ExternalMemoryProviderConfig {
                    enabled: true,
                    sync_policy: ha_core::memory::ExternalMemorySyncPolicy::PushOnly,
                    ..test_provider("automatic")
                },
                ExternalMemoryProviderConfig {
                    enabled: true,
                    sync_policy: ha_core::memory::ExternalMemorySyncPolicy::Manual,
                    ..test_provider("manual")
                },
            ],
        };

        apply_external_memory_sync_origin(&mut config, ExternalMemoryProviderSyncOrigin::Automatic);

        assert!(config.providers[0].enabled);
        assert!(!config.providers[1].enabled);
    }

    #[test]
    fn owner_sync_origin_preserves_the_live_policy_snapshot() {
        let mut config = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![ExternalMemoryProviderConfig {
                enabled: true,
                sync_policy: ha_core::memory::ExternalMemorySyncPolicy::Manual,
                ..test_provider("manual")
            }],
        };

        apply_external_memory_sync_origin(&mut config, ExternalMemoryProviderSyncOrigin::Owner);

        assert!(config.providers[0].enabled);
    }

    #[test]
    fn automatic_sync_refreshes_live_policy_when_process_cache_is_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        ha_core::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let original = ha_core::config::cached_config();
            struct RestoreCache(std::sync::Arc<ha_core::config::AppConfig>);
            impl Drop for RestoreCache {
                fn drop(&mut self) {
                    ha_core::config::replace_cache_for_test((*self.0).clone());
                }
            }
            let _restore = RestoreCache(original);

            ha_core::config::replace_cache_for_test(ha_core::config::AppConfig::default());
            let live_provider = ExternalMemoryProviderConfig {
                enabled: true,
                sync_policy: ha_core::memory::ExternalMemorySyncPolicy::PushOnly,
                endpoint_configured: false,
                ..test_provider("automatic")
            };
            let live = ha_core::config::AppConfig {
                memory_providers: ExternalMemoryProvidersConfig {
                    enabled: true,
                    providers: vec![live_provider],
                },
                ..ha_core::config::AppConfig::default()
            };
            std::fs::write(
                temp.path().join("config.json"),
                serde_json::to_vec_pretty(&live).expect("serialize live config"),
            )
            .expect("write live config");

            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime")
                .block_on(run_automatic_external_memory_provider_sync());

            let refreshed = ha_core::config::cached_config();
            assert!(refreshed.memory_providers.enabled);
            assert_eq!(refreshed.memory_providers.providers.len(), 1);
            assert_eq!(refreshed.memory_providers.providers[0].id, "automatic");
        });
    }

    #[test]
    fn provider_version_floor_rejects_old_and_prerelease_builds() {
        assert!(!version_meets_minimum("0.28.1", "0.28.2"));
        assert!(!version_meets_minimum("0.28.2-rc.1", "0.28.2"));
        assert!(version_meets_minimum("0.28.2", "0.28.2"));
        assert!(version_meets_minimum("0.29.3", "0.28.2"));
    }

    #[test]
    fn compatibility_grant_requires_fresh_evidence_for_the_current_floor() {
        let now = chrono::Utc::now();
        let requirement = VersionRequirement {
            minimum: "0.28.2",
            recommended: "0.29.3",
        };
        let mut report = ExternalMemoryProviderCompatibilityReport {
            provider_id: "zep-main".to_string(),
            kind: ExternalMemoryProviderKind::Zep,
            status: ExternalMemoryProviderCompatibilityStatus::Compatible,
            checked_at: now.to_rfc3339(),
            external_io_performed: true,
            detected_version: Some("0.29.3".to_string()),
            minimum_version: Some(requirement.minimum.to_string()),
            recommended_version: Some(requirement.recommended.to_string()),
            capabilities: Vec::new(),
            error: None,
        };

        assert!(compatibility_report_is_current(
            &report,
            Some(requirement),
            now
        ));

        report.checked_at = (now - chrono::Duration::hours(25)).to_rfc3339();
        assert!(!compatibility_report_is_current(
            &report,
            Some(requirement),
            now
        ));

        report.checked_at = now.to_rfc3339();
        let raised_floor = VersionRequirement {
            minimum: "0.30.0",
            recommended: "0.30.0",
        };
        assert!(!compatibility_report_is_current(
            &report,
            Some(raised_floor),
            now
        ));
    }

    #[test]
    fn compatibility_fingerprint_changes_with_any_probe_identity_input() {
        let credentials = ExternalMemoryProviderCredentials {
            schema_version: CREDENTIAL_SCHEMA_VERSION,
            endpoint: "https://memory.example.test".to_string(),
            api_key: Some("synthetic-key-a".to_string()),
            subject_id: "subject-a".to_string(),
            protocol: "auto".to_string(),
        };
        let baseline =
            compatibility_credential_fingerprint(ExternalMemoryProviderKind::Mem0, &credentials)
                .unwrap();
        for changed in [
            ExternalMemoryProviderCredentials {
                endpoint: "https://other.example.test".to_string(),
                ..credentials.clone()
            },
            ExternalMemoryProviderCredentials {
                api_key: Some("synthetic-key-b".to_string()),
                ..credentials.clone()
            },
            ExternalMemoryProviderCredentials {
                subject_id: "subject-b".to_string(),
                ..credentials.clone()
            },
            ExternalMemoryProviderCredentials {
                protocol: "platform".to_string(),
                ..credentials.clone()
            },
        ] {
            assert_ne!(
                baseline,
                compatibility_credential_fingerprint(ExternalMemoryProviderKind::Mem0, &changed)
                    .unwrap()
            );
        }
        assert_ne!(
            baseline,
            compatibility_credential_fingerprint(ExternalMemoryProviderKind::Zep, &credentials)
                .unwrap()
        );
    }

    #[test]
    fn hosted_protocols_require_the_official_provider_host() {
        let supermemory_cloud = ExternalMemoryProviderCredentials {
            schema_version: CREDENTIAL_SCHEMA_VERSION,
            endpoint: "https://api.supermemory.ai".to_string(),
            api_key: Some("synthetic-key".to_string()),
            subject_id: "subject-a".to_string(),
            protocol: "cloud".to_string(),
        };
        assert!(compatibility_requirement(
            ExternalMemoryProviderKind::Supermemory,
            Some(&supermemory_cloud)
        )
        .is_none());
        assert!(compatibility_requirement(
            ExternalMemoryProviderKind::Supermemory,
            Some(&ExternalMemoryProviderCredentials {
                endpoint: "https://memory.example.test/supermemory.ai".to_string(),
                ..supermemory_cloud
            })
        )
        .is_some());

        let honcho_cloud = ExternalMemoryProviderCredentials {
            schema_version: CREDENTIAL_SCHEMA_VERSION,
            endpoint: "https://api.honcho.dev".to_string(),
            api_key: Some("synthetic-key".to_string()),
            subject_id: "subject-a".to_string(),
            protocol: "v3".to_string(),
        };
        assert!(
            compatibility_requirement(ExternalMemoryProviderKind::Honcho, Some(&honcho_cloud))
                .is_none()
        );
        assert!(compatibility_requirement(
            ExternalMemoryProviderKind::Honcho,
            Some(&ExternalMemoryProviderCredentials {
                endpoint: "https://honcho.example.test".to_string(),
                ..honcho_cloud
            })
        )
        .is_some());
    }

    #[test]
    fn provider_version_detection_is_bounded_to_version_fields_and_headers() {
        let body = serde_json::json!({
            "status": "ok",
            "info": { "version": "v0.29.3" },
            "secret": "token=synthetic-canary"
        });
        assert_eq!(
            detect_provider_version(
                &serde_json::to_vec(&body).unwrap(),
                &["graphiti/0.28.2".to_string()]
            )
            .as_deref(),
            Some("0.29.3")
        );
        assert_eq!(
            detect_provider_version(&serde_json::to_vec(&body).unwrap(), &[]).as_deref(),
            Some("0.29.3")
        );
    }

    #[test]
    fn protocol_is_bounded_and_normalized() {
        assert_eq!(normalize_protocol(None).unwrap(), "auto");
        assert_eq!(
            normalize_protocol(Some(" Platform_V3 ")).unwrap(),
            "platform_v3"
        );
        assert!(normalize_protocol(Some("platform/v3")).is_err());
    }

    #[test]
    fn endpoint_requires_http_origin() {
        assert_eq!(
            normalize_endpoint("https://api.mem0.ai/").unwrap(),
            "https://api.mem0.ai"
        );
        assert!(normalize_endpoint("file:///tmp/memory").is_err());
        assert!(normalize_endpoint("https://example.com?token=secret").is_err());
    }

    #[test]
    fn provider_metadata_patch_preserves_unmentioned_providers_and_runtime_status() {
        let current = ExternalMemoryProvidersConfig {
            enabled: false,
            providers: vec![test_provider("provider-a"), test_provider("provider-b")],
        };
        let patch: ExternalMemoryProvidersPatch = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "providers": [{
                "id": "provider-a",
                "enabled": true,
                "syncPolicy": "push_only"
            }]
        }))
        .unwrap();

        let updated = apply_external_memory_providers_patch(&current, patch).unwrap();

        assert!(updated.enabled);
        assert_eq!(updated.providers.len(), 2);
        let provider_a = updated
            .providers
            .iter()
            .find(|provider| provider.id == "provider-a")
            .unwrap();
        assert!(provider_a.enabled);
        assert_eq!(
            provider_a.sync_policy,
            ha_core::memory::ExternalMemorySyncPolicy::PushOnly
        );
        assert!(provider_a.endpoint_configured);
        assert_eq!(
            provider_a.last_sync_at.as_deref(),
            Some("2026-07-17T00:00:00Z")
        );
        assert_eq!(provider_a.last_error.as_deref(), Some("previous error"));
        assert!(updated
            .providers
            .iter()
            .any(|provider| provider.id == "provider-b"));
    }

    #[test]
    fn provider_deletion_requires_explicit_remove_id() {
        let current = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![test_provider("provider-a"), test_provider("provider-b")],
        };
        let patch: ExternalMemoryProvidersPatch = serde_json::from_value(serde_json::json!({
            "removeProviderIds": ["provider-b"]
        }))
        .unwrap();

        let updated = apply_external_memory_providers_patch(&current, patch).unwrap();

        assert_eq!(updated.providers.len(), 1);
        assert_eq!(updated.providers[0].id, "provider-a");
    }

    #[test]
    fn existing_provider_kind_change_requires_remove_and_readd() {
        let current = ExternalMemoryProvidersConfig {
            enabled: true,
            providers: vec![test_provider("provider-a")],
        };
        let patch: ExternalMemoryProvidersPatch = serde_json::from_value(serde_json::json!({
            "providers": [{
                "id": "provider-a",
                "kind": "zep"
            }]
        }))
        .unwrap();

        let error = apply_external_memory_providers_patch(&current, patch).unwrap_err();

        assert!(error.to_string().contains("kind is immutable"));
        assert!(error.to_string().contains("removeProviderIds"));
    }

    #[test]
    fn provider_patch_rejects_runtime_owned_status_fields() {
        let result = serde_json::from_value::<ExternalMemoryProvidersPatch>(serde_json::json!({
            "providers": [{
                "id": "provider-a",
                "endpointConfigured": false
            }]
        }));

        assert!(result.is_err());
    }

    #[test]
    fn failed_sync_keeps_partial_outcome_after_successful_checkpoint() {
        let outcome = ExternalMemoryAdapterSyncOutcome {
            imported_memory_count: 3,
            ..Default::default()
        };
        let failure = ExternalMemoryAdapterSyncFailure {
            outcome: outcome.clone(),
            error: anyhow!("next page failed"),
        };

        let result = finish_sync_after_checkpoint(outcome, Err(failure), Ok(())).unwrap_err();

        assert_eq!(result.outcome.imported_memory_count, 3);
        assert_eq!(result.error.to_string(), "next page failed");
    }

    #[test]
    fn failed_sync_reports_checkpoint_failure_without_losing_original_error() {
        let outcome = ExternalMemoryAdapterSyncOutcome {
            imported_memory_count: 2,
            ..Default::default()
        };
        let failure = ExternalMemoryAdapterSyncFailure {
            outcome: outcome.clone(),
            error: anyhow!("push failed"),
        };

        let result =
            finish_sync_after_checkpoint(outcome, Err(failure), Err(anyhow!("disk unavailable")))
                .unwrap_err();

        assert_eq!(result.outcome.imported_memory_count, 2);
        assert!(result.error.to_string().contains("push failed"));
        assert!(result.error.to_string().contains("disk unavailable"));
    }
}
