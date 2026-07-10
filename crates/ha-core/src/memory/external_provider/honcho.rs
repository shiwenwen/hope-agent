use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};

use crate::memory::{ExternalMemoryProviderConfig, ExternalMemoryProviderKind};

use super::http::{
    client as external_http_client, endpoint_with_path, send_json, validated_endpoint,
};
use super::{
    import_external_memory_for_review, load_local_memory_snapshot, load_sync_ledger,
    local_memory_fingerprint, persist_sync_ledger, resolve_external_memory_provider_credentials,
    ExternalMemoryAdapterSyncFailure, ExternalMemoryAdapterSyncOutcome,
    ExternalMemoryProviderAdapter, ExternalMemoryProviderCredentials,
    ExternalMemoryProviderSyncLedger,
};

pub(super) static HONCHO_ADAPTER: HonchoAdapter = HonchoAdapter;

pub(super) struct HonchoAdapter;

const MAX_REMOTE_CONCLUSIONS_PER_RUN: usize = 5_000;
const MAX_LOCAL_MEMORIES_PER_RUN: usize = 500;
const LOCAL_MEMORY_SCAN_LIMIT: usize = 20_000;
const PAGE_SIZE: usize = 100;
const PUSH_BATCH_SIZE: usize = 100;
const OBSERVER_ID: &str = "hope_agent";
const OBSERVED_ID: &str = "hope_agent_user";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HonchoProtocol {
    V3,
}

#[derive(Debug, Clone)]
struct RemoteConclusion {
    id: String,
    content: String,
    version: Option<String>,
}

#[async_trait::async_trait]
impl ExternalMemoryProviderAdapter for HonchoAdapter {
    fn kind(&self) -> ExternalMemoryProviderKind {
        ExternalMemoryProviderKind::Honcho
    }

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>
    {
        sync_honcho(provider).await
    }
}

async fn sync_honcho(
    provider: &ExternalMemoryProviderConfig,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
    let (credentials, _) = resolve_external_memory_provider_credentials(&provider.id)
        .map_err(|error| failure(outcome.clone(), error))?
        .ok_or_else(|| failure(outcome.clone(), anyhow!("provider credentials are missing")))?;
    let _protocol =
        resolve_protocol(&credentials).map_err(|error| failure(outcome.clone(), error))?;
    validate_workspace_id(&credentials.subject_id)
        .map_err(|error| failure(outcome.clone(), error))?;
    let endpoint = validated_endpoint(&credentials.endpoint)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let client = external_http_client().map_err(|error| failure(outcome.clone(), error))?;
    let mut ledger =
        load_sync_ledger(&provider.id).map_err(|error| failure(outcome.clone(), error))?;

    if provider.sync_policy.sends_local_memory() {
        ensure_workspace_and_peers(&credentials, &endpoint, &client, &mut outcome).await?;
    }
    if provider.sync_policy.imports_external_memory() {
        pull_conclusions(
            provider,
            &credentials,
            &endpoint,
            &client,
            &mut ledger,
            &mut outcome,
        )
        .await?;
    }
    if provider.sync_policy.sends_local_memory() {
        push_conclusions(
            provider,
            &credentials,
            &endpoint,
            &client,
            &mut ledger,
            &mut outcome,
        )
        .await?;
    }
    Ok(outcome)
}

async fn pull_conclusions(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let exported_ids = ledger
        .exported_remote_ids
        .values()
        .cloned()
        .collect::<HashSet<_>>();
    let url = endpoint_with_path(
        endpoint,
        &[
            "v3",
            "workspaces",
            &credentials.subject_id,
            "conclusions",
            "list",
        ],
    )
    .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let mut page = 1usize;
    let mut seen = 0usize;
    while seen < MAX_REMOTE_CONCLUSIONS_PER_RUN {
        let request = apply_auth(
            client
                .post(&url)
                .query(&[("page", page), ("size", PAGE_SIZE)])
                .json(&json!({})),
            credentials,
        );
        let value = send_json(request, outcome).await?;
        let conclusions = parse_conclusions(&value);
        if conclusions.is_empty() {
            break;
        }
        let page_len = conclusions.len();
        for conclusion in conclusions {
            if seen >= MAX_REMOTE_CONCLUSIONS_PER_RUN {
                break;
            }
            seen += 1;
            if exported_ids.contains(&conclusion.id) || conclusion.content.trim().is_empty() {
                outcome.skipped_memory_count += 1;
                continue;
            }
            if conclusion
                .version
                .as_ref()
                .is_some_and(|version| ledger.remote_versions.get(&conclusion.id) == Some(version))
            {
                outcome.skipped_memory_count += 1;
                continue;
            }
            import_external_memory_for_review(
                provider,
                "honcho",
                &conclusion.id,
                &conclusion.content,
                endpoint,
                ledger,
                outcome,
            )
            .map_err(|error| failure(outcome.clone(), error))?;
            if let Some(version) = conclusion.version {
                ledger.remote_versions.insert(conclusion.id, version);
                persist_sync_ledger(&provider.id, ledger)
                    .map_err(|error| failure(outcome.clone(), error))?;
            }
        }
        let pages = value.get("pages").and_then(Value::as_u64).unwrap_or(0) as usize;
        if page_len < PAGE_SIZE || (pages > 0 && page >= pages) {
            break;
        }
        page += 1;
    }
    emit_import_event(outcome);
    Ok(())
}

async fn ensure_workspace_and_peers(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let workspace_url = endpoint_with_path(endpoint, &["v3", "workspaces"])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&workspace_url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(
        client.post(&workspace_url).json(&json!({
            "id": credentials.subject_id,
            "metadata": {"source": "hope-agent"}
        })),
        credentials,
    );
    let _ = send_json(request, outcome).await?;

    let peer_url = endpoint_with_path(
        endpoint,
        &["v3", "workspaces", &credentials.subject_id, "peers"],
    )
    .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&peer_url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    for (peer_id, role) in [(OBSERVER_ID, "assistant"), (OBSERVED_ID, "user")] {
        let request = apply_auth(
            client.post(&peer_url).json(&json!({
                "id": peer_id,
                "metadata": {"source": "hope-agent", "role": role}
            })),
            credentials,
        );
        let _ = send_json(request, outcome).await?;
    }
    Ok(())
}

async fn push_conclusions(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let (local_memories, total) = load_local_memory_snapshot(LOCAL_MEMORY_SCAN_LIMIT)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let mut changed = Vec::new();
    for memory in local_memories {
        if memory.content.trim().is_empty() || memory.source.starts_with("external_provider") {
            outcome.skipped_memory_count += 1;
            continue;
        }
        let hash = local_memory_fingerprint(&memory);
        if ledger.exported_hashes.get(&memory.id.to_string()) == Some(&hash) {
            outcome.skipped_memory_count += 1;
            continue;
        }
        changed.push((memory, hash));
    }
    if total > LOCAL_MEMORY_SCAN_LIMIT {
        outcome.skipped_memory_count += total - LOCAL_MEMORY_SCAN_LIMIT;
    }
    if changed.len() > MAX_LOCAL_MEMORIES_PER_RUN {
        outcome.skipped_memory_count += changed.len() - MAX_LOCAL_MEMORIES_PER_RUN;
        changed.truncate(MAX_LOCAL_MEMORIES_PER_RUN);
    }

    let url = endpoint_with_path(
        endpoint,
        &["v3", "workspaces", &credentials.subject_id, "conclusions"],
    )
    .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    for batch in changed.chunks(PUSH_BATCH_SIZE) {
        let conclusions = batch
            .iter()
            .map(|(memory, _)| {
                json!({
                    "content": memory.content,
                    "observer_id": OBSERVER_ID,
                    "observed_id": OBSERVED_ID
                })
            })
            .collect::<Vec<_>>();
        let request = apply_auth(
            client.post(&url).json(&json!({"conclusions": conclusions})),
            credentials,
        );
        let response = send_json(request, outcome).await?;
        let remote_ids = response
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if remote_ids.len() != batch.len() {
            return Err(failure(
                outcome.clone(),
                anyhow!("Honcho returned an incomplete conclusions batch"),
            ));
        }
        for ((memory, hash), remote_id) in batch.iter().zip(remote_ids) {
            let old = ledger
                .exported_hashes
                .insert(memory.id.to_string(), hash.clone());
            ledger
                .exported_remote_ids
                .insert(memory.id.to_string(), remote_id);
            if old.is_some() {
                outcome.updated_memory_count += 1;
            } else {
                outcome.exported_memory_count += 1;
            }
        }
        persist_sync_ledger(&provider.id, ledger)
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    Ok(())
}

fn resolve_protocol(credentials: &ExternalMemoryProviderCredentials) -> Result<HonchoProtocol> {
    match credentials.protocol.as_str() {
        "auto" | "v3" | "cloud" | "self_hosted" => Ok(HonchoProtocol::V3),
        other => bail!("unsupported Honcho protocol: {other}"),
    }
}

fn validate_workspace_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        bail!("Honcho workspace ID may contain only letters, numbers, underscores, and hyphens");
    }
    Ok(())
}

fn apply_auth(
    request: RequestBuilder,
    credentials: &ExternalMemoryProviderCredentials,
) -> RequestBuilder {
    match credentials.api_key.as_deref() {
        Some(api_key) => request.bearer_auth(api_key),
        None => request,
    }
}

fn parse_conclusions(value: &Value) -> Vec<RemoteConclusion> {
    let items = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
        .or_else(|| value.get("conclusions").and_then(Value::as_array));
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(RemoteConclusion {
                id: item.get("id")?.as_str()?.to_string(),
                content: item.get("content")?.as_str()?.to_string(),
                version: item
                    .get("updated_at")
                    .or_else(|| item.get("created_at"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            })
        })
        .collect()
}

fn emit_import_event(outcome: &ExternalMemoryAdapterSyncOutcome) {
    let changed = outcome.imported_memory_count + outcome.updated_memory_count;
    if changed > 0 {
        crate::memory::emit_claim_changed("external_provider_import", None, Some(changed));
    }
}

fn failure(
    outcome: ExternalMemoryAdapterSyncOutcome,
    error: anyhow::Error,
) -> ExternalMemoryAdapterSyncFailure {
    ExternalMemoryAdapterSyncFailure { outcome, error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paginated_conclusions() {
        let value = json!({
            "items": [{"id": "c1", "content": "alpha", "created_at": "2026-01-01T00:00:00Z"}],
            "page": 1,
            "pages": 1
        });
        let conclusions = parse_conclusions(&value);
        assert_eq!(conclusions[0].id, "c1");
        assert_eq!(
            conclusions[0].version.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
    }

    #[test]
    fn workspace_ids_follow_honcho_contract() {
        assert!(validate_workspace_id("hope-agent_01").is_ok());
        assert!(validate_workspace_id("hope/agent").is_err());
    }
}
