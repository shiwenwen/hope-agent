use anyhow::{anyhow, bail, Result};
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};

use crate::memory::{ExternalMemoryProviderConfig, ExternalMemoryProviderKind};

use super::http::{
    client as external_http_client, endpoint_with_path, send_json, validated_endpoint,
};
use super::{
    import_external_memory_for_review, load_local_memory_snapshot, load_sync_ledger_async,
    local_memory_fingerprint, persist_sync_ledger_async,
    resolve_external_memory_provider_credentials_async, ExternalMemoryAdapterSyncFailure,
    ExternalMemoryAdapterSyncOutcome, ExternalMemoryProviderAdapter,
    ExternalMemoryProviderCredentials, ExternalMemoryProviderSyncLedger,
};

pub(super) static CUSTOM_ADAPTER: CustomAdapter = CustomAdapter;

pub(super) struct CustomAdapter;

const MAX_REMOTE_MEMORIES_PER_RUN: usize = 5_000;
const MAX_LOCAL_MEMORIES_PER_RUN: usize = 500;
const LOCAL_MEMORY_SCAN_LIMIT: usize = 20_000;
const PUSH_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone)]
struct RemoteMemory {
    id: String,
    content: String,
    external_id: Option<String>,
    metadata: Value,
    version: Option<String>,
}

#[async_trait::async_trait]
impl ExternalMemoryProviderAdapter for CustomAdapter {
    fn kind(&self) -> ExternalMemoryProviderKind {
        ExternalMemoryProviderKind::Custom
    }

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>
    {
        sync_custom(provider).await
    }
}

async fn sync_custom(
    provider: &ExternalMemoryProviderConfig,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
    let (credentials, _) = resolve_external_memory_provider_credentials_async(&provider.id)
        .await
        .map_err(|error| failure(outcome.clone(), error))?
        .ok_or_else(|| failure(outcome.clone(), anyhow!("provider credentials are missing")))?;
    validate_protocol(&credentials).map_err(|error| failure(outcome.clone(), error))?;
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
    let url = endpoint_with_path(endpoint, &["v1", "memories"])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let mut cursor: Option<String> = None;
    let mut seen = 0usize;
    while seen < MAX_REMOTE_MEMORIES_PER_RUN {
        let mut request = client.get(&url).query(&[
            ("subject_id", credentials.subject_id.as_str()),
            ("limit", "200"),
        ]);
        if let Some(cursor) = cursor.as_deref() {
            request = request.query(&[("cursor", cursor)]);
        }
        let value = send_json(apply_auth(request, credentials), outcome).await?;
        let memories = parse_remote_memories(&value);
        if memories.is_empty() {
            break;
        }
        for memory in memories {
            if seen >= MAX_REMOTE_MEMORIES_PER_RUN {
                break;
            }
            seen += 1;
            if memory.content.trim().is_empty() || is_own_export(provider, &memory) {
                outcome.skipped_memory_count += 1;
                continue;
            }
            if memory
                .version
                .as_ref()
                .is_some_and(|version| ledger.remote_versions.get(&memory.id) == Some(version))
            {
                outcome.skipped_memory_count += 1;
                continue;
            }
            import_external_memory_for_review(
                provider,
                "custom",
                &memory.id,
                &memory.content,
                endpoint,
                ledger,
                outcome,
            )
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
            if let Some(version) = memory.version {
                ledger.remote_versions.insert(memory.id, version);
                persist_sync_ledger_async(&provider.id, ledger)
                    .await
                    .map_err(|error| failure(outcome.clone(), error))?;
            }
        }
        cursor = value
            .get("next_cursor")
            .or_else(|| value.get("nextCursor"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if cursor.is_none() {
            break;
        }
    }
    emit_import_event(outcome);
    Ok(())
}

async fn push_memories(
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

    let url = endpoint_with_path(endpoint, &["v1", "memories", "upsert"])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    for batch in changed.chunks(PUSH_BATCH_SIZE) {
        let memories = batch
            .iter()
            .map(|(memory, _)| {
                json!({
                    "external_id": export_external_id(provider, &memory.id.to_string()),
                    "content": memory.content,
                    "memory_type": memory.memory_type.as_str(),
                    "scope": memory.scope,
                    "tags": memory.tags,
                    "pinned": memory.pinned,
                    "updated_at": memory.updated_at,
                    "metadata": {
                        "hope_agent_provider_id": provider.id,
                        "hope_agent_local_id": memory.id.to_string(),
                        "hope_agent_source": "local_memory"
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = apply_auth(
            client.post(&url).json(&json!({
                "subject_id": credentials.subject_id,
                "memories": memories
            })),
            credentials,
        );
        let value = send_json(request, outcome).await?;
        let items = value
            .get("items")
            .or_else(|| value.get("memories"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                failure(
                    outcome.clone(),
                    anyhow!("Hope Sync v1 upsert response omitted items"),
                )
            })?;
        if items.len() != batch.len() {
            return Err(failure(
                outcome.clone(),
                anyhow!("Hope Sync v1 returned an incomplete upsert batch"),
            ));
        }
        for ((memory, hash), item) in batch.iter().zip(items) {
            let expected_external_id = export_external_id(provider, &memory.id.to_string());
            let external_id = item
                .get("external_id")
                .or_else(|| item.get("externalId"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if external_id != expected_external_id {
                return Err(failure(
                    outcome.clone(),
                    anyhow!("Hope Sync v1 upsert response external_id mismatch"),
                ));
            }
            let remote_id = item
                .get("id")
                .or_else(|| item.get("remote_id"))
                .or_else(|| item.get("remoteId"))
                .and_then(value_to_id)
                .unwrap_or_else(|| expected_external_id.clone());
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
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    Ok(())
}

fn validate_protocol(credentials: &ExternalMemoryProviderCredentials) -> Result<()> {
    match credentials.protocol.as_str() {
        "auto" | "hope_sync_v1" | "hope-sync-v1" => Ok(()),
        other => bail!("unsupported custom provider protocol: {other}; use Hope Sync v1"),
    }
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

fn parse_remote_memories(value: &Value) -> Vec<RemoteMemory> {
    let items = value
        .as_array()
        .or_else(|| value.get("memories").and_then(Value::as_array))
        .or_else(|| value.get("items").and_then(Value::as_array));
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("remote_id"))
                .or_else(|| item.get("remoteId"))
                .and_then(value_to_id)?;
            Some(RemoteMemory {
                id,
                content: item.get("content")?.as_str()?.to_string(),
                external_id: item
                    .get("external_id")
                    .or_else(|| item.get("externalId"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                metadata: item.get("metadata").cloned().unwrap_or(Value::Null),
                version: item
                    .get("updated_at")
                    .or_else(|| item.get("updatedAt"))
                    .or_else(|| item.get("version"))
                    .and_then(|value| {
                        value
                            .as_str()
                            .map(ToString::to_string)
                            .or_else(|| value.as_i64().map(|v| v.to_string()))
                            .or_else(|| value.as_u64().map(|v| v.to_string()))
                    }),
            })
        })
        .collect()
}

fn value_to_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(ToString::to_string)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
        .or_else(|| value.as_u64().map(|id| id.to_string()))
}

fn export_external_id(provider: &ExternalMemoryProviderConfig, local_id: &str) -> String {
    format!("hope-agent:{}:{local_id}", provider.id)
}

fn is_own_export(provider: &ExternalMemoryProviderConfig, memory: &RemoteMemory) -> bool {
    memory
        .external_id
        .as_deref()
        .is_some_and(|id| id.starts_with(&format!("hope-agent:{}:", provider.id)))
        || memory
            .metadata
            .get("hope_agent_provider_id")
            .and_then(Value::as_str)
            == Some(provider.id.as_str())
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
    use crate::memory::ExternalMemorySyncPolicy;

    fn provider() -> ExternalMemoryProviderConfig {
        ExternalMemoryProviderConfig {
            id: "custom-main".into(),
            kind: ExternalMemoryProviderKind::Custom,
            display_name: "Private memory".into(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::Bidirectional,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        }
    }

    #[test]
    fn parses_versioned_custom_memories() {
        let value = json!({
            "memories": [{
                "id": "m1",
                "external_id": "external:1",
                "content": "alpha",
                "updated_at": "2026-01-01T00:00:00Z"
            }]
        });
        let memories = parse_remote_memories(&value);
        assert_eq!(memories[0].id, "m1");
        assert_eq!(memories[0].version.as_deref(), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn own_external_ids_are_not_reimported() {
        let provider = provider();
        let memory = RemoteMemory {
            id: "m1".into(),
            content: "alpha".into(),
            external_id: Some(export_external_id(&provider, "7")),
            metadata: Value::Null,
            version: None,
        };
        assert!(is_own_export(&provider, &memory));
    }

    #[test]
    fn only_versioned_custom_protocol_is_accepted() {
        let credentials = ExternalMemoryProviderCredentials {
            schema_version: 1,
            endpoint: "https://example.com".into(),
            api_key: None,
            subject_id: "user".into(),
            protocol: "arbitrary".into(),
        };
        assert!(validate_protocol(&credentials).is_err());
    }
}
