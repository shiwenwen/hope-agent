//! Additive external memory provider runtime.
//!
//! The local SQLite/claim stores remain authoritative. Credentials live in a
//! separate restricted file and are never returned by owner read APIs. Pulls
//! from a provider must enter the local review path before they can influence
//! prompts; concrete adapters own only network protocol translation.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::{external_memory_credential_path, external_memory_sync_state_path};
use crate::platform::write_secure_file;

use super::{
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
const MAX_ENDPOINT_CHARS: usize = 2_048;
const MAX_SUBJECT_ID_CHARS: usize = 256;
const MAX_PROTOCOL_CHARS: usize = 48;
const SYNC_STATE_SCHEMA_VERSION: u32 = 1;
const MAX_IMPORTED_CONTENT_CHARS: usize = 16_000;
const IMPORT_LEDGER_CHECKPOINT_EVERY: usize = 100;
const PROVIDER_SYNC_TIMEOUT: Duration = Duration::from_secs(120);
tokio::task_local! {
    static PROVIDER_SYNC_DEADLINE: std::time::Instant;
}
static AUTO_SYNC_QUEUED: AtomicBool = AtomicBool::new(false);
static AUTO_SYNC_DIRTY: AtomicBool = AtomicBool::new(false);
static EXTERNAL_PROVIDER_SYNC_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

/// Owner write shape for one provider's secret runtime configuration.
/// `api_key=None` preserves an existing key; `Some("")` clears it.
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

/// Safe owner read shape. It deliberately exposes neither the API key nor the
/// endpoint path/query, which can contain tenant identifiers or legacy tokens.
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

pub async fn execute_external_memory_provider_sync(
    config: ExternalMemoryProvidersConfig,
    stats: MemoryStats,
    stats_error: Option<String>,
) -> ExternalMemoryProviderSyncReport {
    // Credentials and sync ledgers are process-shared files. Serializing all
    // provider runs avoids duplicate exports and lost ledger updates when a
    // manual run overlaps the debounce or periodic scheduler.
    let _sync_guard = EXTERNAL_PROVIDER_SYNC_LOCK.lock().await;
    let config =
        crate::blocking::run_blocking(move || hydrate_external_memory_provider_config(config))
            .await;
    let preflight = config.sync_preflight_with_stats_status(&stats, stats_error);
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
    crate::blocking::run_blocking(move || persist_sync_health(&health_results)).await;
    summarize_sync_report(preflight_summary(results, preflight_report))
}

/// Debounced automatic sync trigger for local memory writes. Manual providers
/// are stripped from the execution snapshot, so this can never turn an owner-
/// initiated policy into background network traffic.
pub fn schedule_external_memory_provider_sync() {
    if !crate::runtime_lock::is_primary() || !has_automatic_provider() {
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
    if !crate::runtime_lock::is_primary() {
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
    let mut config = crate::config::cached_config().memory_providers.clone();
    for provider in &mut config.providers {
        if !matches!(
            provider.sync_policy,
            super::ExternalMemorySyncPolicy::PullOnly
                | super::ExternalMemorySyncPolicy::PushOnly
                | super::ExternalMemorySyncPolicy::Bidirectional
        ) {
            provider.enabled = false;
        }
    }
    if !config.enabled || !config.providers.iter().any(|provider| provider.enabled) {
        return;
    }
    let (stats, stats_error) =
        crate::blocking::run_blocking(super::helpers::external_memory_provider_stats_for_planning)
            .await;
    let report = execute_external_memory_provider_sync(config, stats, stats_error).await;
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
    let config = crate::config::cached_config();
    config.memory_providers.enabled
        && config.memory_providers.providers.iter().any(|provider| {
            provider.enabled
                && provider.kind.capabilities().adapter_available
                && matches!(
                    provider.sync_policy,
                    super::ExternalMemorySyncPolicy::PullOnly
                        | super::ExternalMemorySyncPolicy::PushOnly
                        | super::ExternalMemorySyncPolicy::Bidirectional
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
    preflight: super::ExternalMemoryProviderPreflightReport,
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
    preflight: super::ExternalMemoryProviderPreflight,
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
    if let Err(err) =
        crate::config::mutate_config(("memory_providers.sync", "owner"), move |store| {
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
    {
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
    crate::blocking::run_blocking(move || {
        resolve_external_memory_provider_credentials(&provider_id)
    })
    .await
}

pub(crate) async fn load_sync_ledger_async(
    provider_id: &str,
) -> Result<ExternalMemoryProviderSyncLedger> {
    let provider_id = provider_id.to_string();
    crate::blocking::run_blocking(move || load_sync_ledger(&provider_id)).await
}

pub(crate) async fn persist_sync_ledger_async(
    provider_id: &str,
    ledger: &ExternalMemoryProviderSyncLedger,
) -> Result<()> {
    let provider_id = provider_id.to_string();
    let ledger = ledger.clone();
    crate::blocking::run_blocking(move || persist_sync_ledger(&provider_id, &ledger)).await
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
) -> Result<(Vec<super::MemoryEntry>, usize)> {
    let backend =
        crate::get_memory_backend().ok_or_else(|| anyhow!("memory backend unavailable"))?;
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
    let candidate = crate::memory::claims::ClaimCandidate {
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
    crate::blocking::run_blocking(move || {
        crate::memory::claims::write_claim_candidate_with_status(
            &candidate,
            &super::MemoryScope::Global,
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

pub(crate) fn local_memory_fingerprint(memory: &super::MemoryEntry) -> String {
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
    crate::logging::redact_sensitive(error)
        .chars()
        .take(512)
        .collect()
}

pub async fn save_external_memory_provider_credentials(
    input: ExternalMemoryProviderCredentialInput,
) -> Result<ExternalMemoryProviderCredentialStatus> {
    validate_provider_id(&input.provider_id)?;
    ensure_provider_exists(&input.provider_id)?;

    let provider_id_for_load = input.provider_id.clone();
    let existing =
        crate::blocking::run_blocking(move || load_credentials_file(&provider_id_for_load)).await?;
    let endpoint = if input.endpoint.trim().is_empty() {
        existing
            .as_ref()
            .map(|credentials| credentials.endpoint.clone())
            .ok_or_else(|| anyhow!("external memory provider endpoint is required"))?
    } else {
        normalize_endpoint(&input.endpoint)?
    };
    let ssrf = crate::config::cached_config().ssrf.clone();
    crate::security::ssrf::check_url(&endpoint, ssrf.default_policy, &ssrf.trusted_hosts)
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
    crate::blocking::run_blocking(move || {
        let credential_path = external_memory_credential_path(&provider_id)?;
        let ledger_path = external_memory_sync_state_path(&provider_id)?;
        let previous_credential_bytes = read_optional_file(&credential_path)?;
        let previous_ledger_bytes = if reset_sync_ledger {
            read_optional_file(&ledger_path)?
        } else {
            None
        };
        persist_credentials(&provider_id, &credentials)?;
        if reset_sync_ledger {
            remove_sync_ledger(&provider_id)?;
        }

        let provider_id_for_config = provider_id.clone();
        if let Err(err) =
            crate::config::mutate_config(("memory_providers.credentials", "owner"), move |store| {
                let provider = store
                    .memory_providers
                    .providers
                    .iter_mut()
                    .find(|provider| provider.id == provider_id_for_config)
                    .ok_or_else(|| anyhow!("external memory provider not found"))?;
                provider.endpoint_configured = true;
                provider.last_error = None;
                Ok(())
            })
        {
            restore_optional_secure_file(&credential_path, previous_credential_bytes.as_deref())?;
            if reset_sync_ledger {
                restore_optional_secure_file(&ledger_path, previous_ledger_bytes.as_deref())?;
            }
            return Err(err).context("persist external memory provider readiness");
        }

        Ok(status_from_credentials(provider_id, credentials, "file"))
    })
    .await
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
    ensure_provider_exists(provider_id)?;
    let path = external_memory_credential_path(provider_id)?;
    let ledger_path = external_memory_sync_state_path(provider_id)?;
    let previous_credential_bytes = read_optional_file(&path)?;
    let previous_ledger_bytes = read_optional_file(&ledger_path)?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(anyhow!("remove {}: {err}", path.display())),
    }
    remove_sync_ledger(provider_id)?;

    let provider_id_owned = provider_id.to_string();
    if let Err(err) =
        crate::config::mutate_config(("memory_providers.credentials", "owner"), move |store| {
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

pub fn save_external_memory_providers_config(
    config: ExternalMemoryProvidersConfig,
    source: &'static str,
) -> Result<()> {
    let config = config.normalized();
    let valid_ids = config
        .providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<std::collections::HashSet<_>>();
    crate::config::mutate_config(("memory_providers", source), move |store| {
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
    if crate::config::cached_config()
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
    let dir = crate::paths::external_memory_credentials_dir()?;
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
            .strip_suffix(".sync.json")
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

    #[test]
    fn provider_id_rejects_path_traversal() {
        assert!(validate_provider_id("mem0-main").is_ok());
        assert!(validate_provider_id("../mem0").is_err());
        assert!(validate_provider_id("Mem0").is_err());
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
