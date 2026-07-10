use anyhow::{anyhow, bail, Result};
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};

use crate::memory::ExternalMemoryProviderConfig;

use super::http::{
    client as external_http_client, endpoint_with_path, send_json, validated_endpoint,
};
use super::{
    content_fingerprint, import_external_memory_for_review, load_local_memory_snapshot,
    load_sync_ledger_async, local_memory_fingerprint, persist_sync_ledger_async,
    resolve_external_memory_provider_credentials_async, ExternalMemoryAdapterSyncFailure,
    ExternalMemoryAdapterSyncOutcome, ExternalMemoryProviderAdapter,
    ExternalMemoryProviderCredentials, ExternalMemoryProviderSyncLedger,
};

pub(super) static SUPERMEMORY_ADAPTER: SupermemoryAdapter = SupermemoryAdapter;

pub(super) struct SupermemoryAdapter;

const LOCAL_MEMORY_SCAN_LIMIT: usize = 20_000;
const MAX_LOCAL_MEMORIES_PER_RUN: usize = 200;
const MAX_REMOTE_LIST_PER_RUN: usize = 5_000;
const MAX_REMOTE_FETCH_PER_RUN: usize = 200;
const REMOTE_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone)]
struct ListedMemory {
    id: String,
    custom_id: Option<String>,
    summary: Option<String>,
    metadata: Value,
    version: String,
}

#[async_trait::async_trait]
impl ExternalMemoryProviderAdapter for SupermemoryAdapter {
    fn kind(&self) -> crate::memory::ExternalMemoryProviderKind {
        crate::memory::ExternalMemoryProviderKind::Supermemory
    }

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>
    {
        sync_supermemory(provider).await
    }
}

async fn sync_supermemory(
    provider: &ExternalMemoryProviderConfig,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
    let (credentials, _) = resolve_external_memory_provider_credentials_async(&provider.id)
        .await
        .map_err(|error| failure(outcome.clone(), error))?
        .ok_or_else(|| failure(outcome.clone(), anyhow!("provider credentials are missing")))?;
    validate_protocol(&credentials).map_err(|error| failure(outcome.clone(), error))?;
    if credentials.api_key.is_none() {
        return Err(failure(outcome, anyhow!("Supermemory requires an API key")));
    }
    let endpoint = validated_endpoint(&credentials.endpoint)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let client = external_http_client().map_err(|error| failure(outcome.clone(), error))?;
    let mut ledger = load_sync_ledger_async(&provider.id)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;

    if provider.sync_policy.imports_external_memory() {
        pull_memories(
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
        push_memories(
            provider,
            &credentials,
            &endpoint,
            &client,
            &mut ledger,
            &mut outcome,
        )
        .await?;
    }
    persist_sync_ledger_async(&provider.id, &ledger)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    Ok(outcome)
}

async fn pull_memories(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let mut page = 1usize;
    let mut scanned = 0usize;
    let mut fetched = 0usize;
    loop {
        let url = endpoint_with_path(endpoint, &["v3", "documents", "list"])
            .map_err(|error| failure(outcome.clone(), error))?;
        validated_endpoint(&url)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        let request = apply_auth(
            client.post(&url).json(&json!({
                "page": page,
                "limit": REMOTE_PAGE_SIZE,
                "containerTags": [credentials.subject_id]
            })),
            credentials,
        )
        .map_err(|error| failure(outcome.clone(), error))?;
        let value = send_json(request, outcome).await?;
        let listed = parse_listed_memories(&value);
        if listed.is_empty() {
            break;
        }
        scanned += listed.len();

        for memory in listed {
            if memory_is_own_export(provider, &memory) {
                outcome.skipped_memory_count += 1;
                continue;
            }
            if ledger.remote_versions.get(&memory.id) == Some(&memory.version) {
                outcome.skipped_memory_count += 1;
                continue;
            }
            if fetched >= MAX_REMOTE_FETCH_PER_RUN {
                outcome.skipped_memory_count += 1;
                continue;
            }
            fetched += 1;
            let detail =
                get_memory_detail(credentials, endpoint, client, &memory.id, outcome).await?;
            let content = detail
                .get("content")
                .or_else(|| detail.get("raw"))
                .or_else(|| detail.get("summary"))
                .and_then(Value::as_str)
                .or(memory.summary.as_deref())
                .unwrap_or("");
            import_external_memory_for_review(
                provider,
                "supermemory",
                &memory.id,
                content,
                endpoint,
                ledger,
                outcome,
            )
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
            ledger.remote_versions.insert(memory.id, memory.version);
            persist_sync_ledger_async(&provider.id, ledger)
                .await
                .map_err(|error| failure(outcome.clone(), error))?;
        }

        let total_pages = value
            .pointer("/pagination/totalPages")
            .and_then(Value::as_u64)
            .unwrap_or(page as u64) as usize;
        if page >= total_pages || scanned >= MAX_REMOTE_LIST_PER_RUN {
            break;
        }
        page += 1;
    }

    if outcome.imported_memory_count + outcome.updated_memory_count > 0 {
        crate::memory::emit_claim_changed(
            "external_provider_import",
            None,
            Some(outcome.imported_memory_count + outcome.updated_memory_count),
        );
    }
    Ok(())
}

async fn get_memory_detail(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    memory_id: &str,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<Value, ExternalMemoryAdapterSyncFailure> {
    let url = endpoint_with_path(endpoint, &["v3", "documents", memory_id])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(client.get(&url), credentials)
        .map_err(|error| failure(outcome.clone(), error))?;
    send_json(request, outcome).await
}

async fn push_memories(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    reconcile_pending_exports(provider, credentials, endpoint, client, ledger, outcome).await?;
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
        if ledger.pending_export_hashes.get(&memory.id.to_string()) == Some(&hash) {
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

    for (memory, hash) in changed {
        let local_id = memory.id.to_string();
        let existing_remote_id = ledger.exported_remote_ids.get(&local_id).cloned();
        let custom_id = format!("hope-agent:{}:{}", provider.id, memory.id);
        let body = json!({
            "content": memory.content,
            "customId": custom_id,
            "containerTags": [credentials.subject_id],
            "metadata": {
                "hope_agent_provider_id": provider.id,
                "hope_agent_memory_id": memory.id,
                "hope_agent_memory_type": memory.memory_type.as_str(),
                "hope_agent_source": "local_memory"
            }
        });
        let response = if let Some(remote_id) = existing_remote_id.as_deref() {
            let url = endpoint_with_path(endpoint, &["v3", "documents", remote_id])
                .map_err(|error| failure(outcome.clone(), error))?;
            validated_endpoint(&url)
                .await
                .map_err(|error| failure(outcome.clone(), error))?;
            let request = apply_auth(
                client.patch(&url).json(&json!({
                    "content": memory.content,
                    "metadata": body["metadata"].clone()
                })),
                credentials,
            )
            .map_err(|error| failure(outcome.clone(), error))?;
            send_json(request, outcome).await?
        } else {
            let url = endpoint_with_path(endpoint, &["v3", "documents"])
                .map_err(|error| failure(outcome.clone(), error))?;
            validated_endpoint(&url)
                .await
                .map_err(|error| failure(outcome.clone(), error))?;
            let request = apply_auth(client.post(&url).json(&body), credentials)
                .map_err(|error| failure(outcome.clone(), error))?;
            send_json(request, outcome).await?
        };

        let remote_id = response
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or(existing_remote_id.clone())
            .ok_or_else(|| {
                failure(
                    outcome.clone(),
                    anyhow!("Supermemory document response omitted id"),
                )
            })?;
        ledger
            .exported_remote_ids
            .insert(local_id.clone(), remote_id);
        if document_status(&response) == DocumentStatus::Done {
            promote_export(local_id, hash, ledger, outcome);
        } else if document_status(&response) == DocumentStatus::Failed {
            ledger.pending_export_hashes.remove(&local_id);
            return Err(failure(
                outcome.clone(),
                anyhow!("Supermemory document processing failed"),
            ));
        } else {
            ledger.pending_export_hashes.insert(local_id, hash);
        }
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    Ok(())
}

async fn reconcile_pending_exports(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let pending = ledger
        .pending_export_hashes
        .iter()
        .map(|(local_id, hash)| (local_id.clone(), hash.clone()))
        .take(MAX_LOCAL_MEMORIES_PER_RUN)
        .collect::<Vec<_>>();
    for (local_id, hash) in pending {
        let Some(remote_id) = ledger.exported_remote_ids.get(&local_id).cloned() else {
            ledger.pending_export_hashes.remove(&local_id);
            continue;
        };
        let detail = get_memory_detail(credentials, endpoint, client, &remote_id, outcome).await?;
        match document_status(&detail) {
            DocumentStatus::Done => promote_export(local_id, hash, ledger, outcome),
            DocumentStatus::Failed => {
                ledger.pending_export_hashes.remove(&local_id);
            }
            DocumentStatus::Pending => outcome.skipped_memory_count += 1,
        }
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentStatus {
    Pending,
    Done,
    Failed,
}

fn document_status(value: &Value) -> DocumentStatus {
    match value.get("status").and_then(Value::as_str).unwrap_or("") {
        "done" | "completed" | "success" => DocumentStatus::Done,
        "failed" | "error" => DocumentStatus::Failed,
        _ => DocumentStatus::Pending,
    }
}

fn promote_export(
    local_id: String,
    hash: String,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) {
    ledger.pending_export_hashes.remove(&local_id);
    let old = ledger.exported_hashes.insert(local_id, hash);
    if old.is_some() {
        outcome.updated_memory_count += 1;
    } else {
        outcome.exported_memory_count += 1;
    }
}

fn apply_auth(
    request: RequestBuilder,
    credentials: &ExternalMemoryProviderCredentials,
) -> Result<RequestBuilder> {
    let api_key = credentials
        .api_key
        .as_deref()
        .ok_or_else(|| anyhow!("Supermemory requires an API key"))?;
    Ok(request.bearer_auth(api_key))
}

fn validate_protocol(credentials: &ExternalMemoryProviderCredentials) -> Result<()> {
    match credentials.protocol.as_str() {
        "auto" | "platform" | "cloud" | "self_hosted" | "self-hosted" => Ok(()),
        other => bail!("unsupported Supermemory protocol: {other}"),
    }
}

fn parse_listed_memories(value: &Value) -> Vec<ListedMemory> {
    let Some(items) = value.get("memories").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.to_string();
            let updated_at = item
                .get("updatedAt")
                .or_else(|| item.get("updated_at"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let status = item.get("status").and_then(Value::as_str).unwrap_or("");
            Some(ListedMemory {
                id: id.clone(),
                custom_id: item
                    .get("customId")
                    .or_else(|| item.get("custom_id"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                summary: item
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                metadata: item.get("metadata").cloned().unwrap_or(Value::Null),
                version: content_fingerprint(&format!("{id}|{updated_at}|{status}")),
            })
        })
        .collect()
}

fn memory_is_own_export(provider: &ExternalMemoryProviderConfig, memory: &ListedMemory) -> bool {
    memory
        .custom_id
        .as_deref()
        .is_some_and(|custom_id| custom_id.starts_with(&format!("hope-agent:{}:", provider.id)))
        || memory
            .metadata
            .get("hope_agent_provider_id")
            .and_then(Value::as_str)
            == Some(provider.id.as_str())
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
    fn parses_list_envelope_and_detects_own_export() {
        let value = json!({
            "memories": [{
                "id": "sm-1",
                "customId": "hope-agent:super-main:7",
                "summary": "hello",
                "status": "done",
                "updatedAt": "2026-07-10T00:00:00Z",
                "metadata": {"hope_agent_provider_id": "super-main"}
            }]
        });
        let listed = parse_listed_memories(&value);
        assert_eq!(listed.len(), 1);
        let provider = ExternalMemoryProviderConfig {
            id: "super-main".into(),
            kind: crate::memory::ExternalMemoryProviderKind::Supermemory,
            display_name: "Supermemory".into(),
            enabled: true,
            sync_policy: crate::memory::ExternalMemorySyncPolicy::Bidirectional,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        };
        assert!(memory_is_own_export(&provider, &listed[0]));
    }
}
