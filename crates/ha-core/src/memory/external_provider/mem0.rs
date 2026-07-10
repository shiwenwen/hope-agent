use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};

use crate::memory::{ExternalMemoryProviderConfig, MemoryEntry};

use super::http::{client as external_http_client, send_json, validated_endpoint};
use super::{
    content_fingerprint, import_external_memory_for_review, load_local_memory_snapshot,
    load_sync_ledger, persist_sync_ledger, resolve_external_memory_provider_credentials,
    ExternalMemoryAdapterSyncFailure, ExternalMemoryAdapterSyncOutcome,
    ExternalMemoryProviderAdapter, ExternalMemoryProviderCredentials,
    ExternalMemoryProviderSyncLedger,
};

pub(super) static MEM0_ADAPTER: Mem0Adapter = Mem0Adapter;

pub(super) struct Mem0Adapter;

const MAX_REMOTE_MEMORIES_PER_RUN: usize = 5_000;
const MAX_LOCAL_MEMORIES_PER_RUN: usize = 500;
const LOCAL_MEMORY_SCAN_LIMIT: usize = 20_000;
const PUSH_BATCH_SIZE: usize = 50;
const EVENT_POLL_ATTEMPTS: usize = 40;
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mem0Protocol {
    PlatformV3,
    Oss,
}

#[derive(Debug, Clone)]
struct RemoteMemory {
    id: String,
    content: String,
    metadata: Value,
}

#[async_trait::async_trait]
impl ExternalMemoryProviderAdapter for Mem0Adapter {
    fn kind(&self) -> crate::memory::ExternalMemoryProviderKind {
        crate::memory::ExternalMemoryProviderKind::Mem0
    }

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>
    {
        sync_mem0(provider).await
    }
}

async fn sync_mem0(
    provider: &ExternalMemoryProviderConfig,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
    let (credentials, _) = resolve_external_memory_provider_credentials(&provider.id)
        .map_err(|error| failure(outcome.clone(), error))?
        .ok_or_else(|| failure(outcome.clone(), anyhow!("provider credentials are missing")))?;
    let protocol =
        resolve_protocol(&credentials).map_err(|error| failure(outcome.clone(), error))?;
    if protocol == Mem0Protocol::PlatformV3 && credentials.api_key.is_none() {
        return Err(failure(
            outcome,
            anyhow!("Mem0 Platform requires an API key"),
        ));
    }

    let endpoint = validated_endpoint(&credentials.endpoint)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let client = external_http_client().map_err(|error| failure(outcome.clone(), error))?;
    let mut ledger =
        load_sync_ledger(&provider.id).map_err(|error| failure(outcome.clone(), error))?;

    if provider.sync_policy.imports_external_memory() {
        pull_memories(
            provider,
            &credentials,
            protocol,
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
            protocol,
            &endpoint,
            &client,
            &mut ledger,
            &mut outcome,
        )
        .await?;
    }

    Ok(outcome)
}

async fn pull_memories(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    protocol: Mem0Protocol,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let remote = match protocol {
        Mem0Protocol::PlatformV3 => {
            pull_platform_memories(credentials, endpoint, client, outcome).await?
        }
        Mem0Protocol::Oss => pull_oss_memories(credentials, endpoint, client, outcome).await?,
    };

    for memory in remote.into_iter().take(MAX_REMOTE_MEMORIES_PER_RUN) {
        if memory.content.trim().is_empty() || memory_is_own_export(provider, &memory) {
            outcome.skipped_memory_count += 1;
            continue;
        }
        import_external_memory_for_review(
            provider,
            "mem0",
            &memory.id,
            &memory.content,
            endpoint,
            ledger,
            outcome,
        )
        .map_err(|error| failure(outcome.clone(), error))?;
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

async fn pull_platform_memories(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<Vec<RemoteMemory>, ExternalMemoryAdapterSyncFailure> {
    let mut memories = Vec::new();
    let mut page = 1usize;
    while memories.len() < MAX_REMOTE_MEMORIES_PER_RUN {
        let url = format!("{endpoint}/v3/memories/?page={page}&page_size=200");
        validated_endpoint(&url)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        let request = apply_auth(
            client
                .post(&url)
                .json(&json!({"filters": {"user_id": credentials.subject_id}})),
            credentials,
            Mem0Protocol::PlatformV3,
        )
        .map_err(|error| failure(outcome.clone(), error))?;
        let value = send_json(request, outcome).await?;
        let page_memories = parse_remote_memories(&value);
        memories.extend(page_memories);
        if value.get("next").is_none_or(Value::is_null)
            || memories.len() >= MAX_REMOTE_MEMORIES_PER_RUN
        {
            break;
        }
        page += 1;
    }
    memories.truncate(MAX_REMOTE_MEMORIES_PER_RUN);
    Ok(memories)
}

async fn pull_oss_memories(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<Vec<RemoteMemory>, ExternalMemoryAdapterSyncFailure> {
    let url = format!("{endpoint}/memories");
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(
        client
            .get(&url)
            .query(&[("user_id", credentials.subject_id.as_str())]),
        credentials,
        Mem0Protocol::Oss,
    )
    .map_err(|error| failure(outcome.clone(), error))?;
    let value = send_json(request, outcome).await?;
    let mut memories = parse_remote_memories(&value);
    memories.truncate(MAX_REMOTE_MEMORIES_PER_RUN);
    Ok(memories)
}

async fn push_memories(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    protocol: Mem0Protocol,
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
        let hash = local_memory_hash(&memory);
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

    for batch in changed.chunks(PUSH_BATCH_SIZE) {
        let messages = batch
            .iter()
            .map(|(memory, _)| json!({"role": "user", "content": memory.content}))
            .collect::<Vec<_>>();
        let local_ids = batch
            .iter()
            .map(|(memory, _)| memory.id.to_string())
            .collect::<Vec<_>>();
        let sync_batch = content_fingerprint(
            &batch
                .iter()
                .map(|(memory, hash)| format!("{}:{hash}", memory.id))
                .collect::<Vec<_>>()
                .join("|"),
        );
        let body = json!({
            "messages": messages,
            "user_id": credentials.subject_id,
            "infer": false,
            "metadata": {
                "hope_agent_provider_id": provider.id,
                "hope_agent_sync_batch": sync_batch,
                "hope_agent_local_ids": local_ids,
                "hope_agent_source": "local_memory"
            }
        });
        match protocol {
            Mem0Protocol::PlatformV3 => {
                push_platform_batch(credentials, endpoint, client, body, outcome).await?
            }
            Mem0Protocol::Oss => {
                push_oss_batch(credentials, endpoint, client, body, outcome).await?
            }
        }

        for (memory, hash) in batch {
            let old = ledger
                .exported_hashes
                .insert(memory.id.to_string(), hash.clone());
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

async fn push_platform_batch(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    body: Value,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let url = format!("{endpoint}/v3/memories/add/");
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(
        client.post(&url).json(&body),
        credentials,
        Mem0Protocol::PlatformV3,
    )
    .map_err(|error| failure(outcome.clone(), error))?;
    let response = send_json(request, outcome).await?;
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("PENDING");
    if status.eq_ignore_ascii_case("SUCCEEDED") {
        return Ok(());
    }
    if status.eq_ignore_ascii_case("FAILED") {
        return Err(failure(outcome.clone(), anyhow!("Mem0 add event failed")));
    }
    let event_id = response
        .get("event_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            failure(
                outcome.clone(),
                anyhow!("Mem0 add response omitted event_id"),
            )
        })?;
    poll_platform_event(credentials, endpoint, client, event_id, outcome).await
}

async fn poll_platform_event(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    event_id: &str,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    for _ in 0..EVENT_POLL_ATTEMPTS {
        tokio::time::sleep(EVENT_POLL_INTERVAL).await;
        let url = format!("{endpoint}/v1/event/{event_id}/");
        validated_endpoint(&url)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        let request = apply_auth(client.get(&url), credentials, Mem0Protocol::PlatformV3)
            .map_err(|error| failure(outcome.clone(), error))?;
        let event = send_json(request, outcome).await?;
        match event
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("PENDING")
        {
            "SUCCEEDED" => return Ok(()),
            "FAILED" => return Err(failure(outcome.clone(), anyhow!("Mem0 add event failed"))),
            _ => {}
        }
    }
    Err(failure(
        outcome.clone(),
        anyhow!("Mem0 add event timed out"),
    ))
}

async fn push_oss_batch(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    body: Value,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let url = format!("{endpoint}/memories");
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(
        client.post(&url).json(&body),
        credentials,
        Mem0Protocol::Oss,
    )
    .map_err(|error| failure(outcome.clone(), error))?;
    let _ = send_json(request, outcome).await?;
    Ok(())
}

fn apply_auth(
    request: RequestBuilder,
    credentials: &ExternalMemoryProviderCredentials,
    protocol: Mem0Protocol,
) -> Result<RequestBuilder> {
    match (protocol, credentials.api_key.as_deref()) {
        (Mem0Protocol::PlatformV3, Some(api_key)) => {
            Ok(request.header(reqwest::header::AUTHORIZATION, format!("Token {api_key}")))
        }
        (Mem0Protocol::PlatformV3, None) => bail!("Mem0 Platform requires an API key"),
        (Mem0Protocol::Oss, Some(api_key)) => Ok(request.header("X-API-Key", api_key)),
        (Mem0Protocol::Oss, None) => Ok(request),
    }
}

fn resolve_protocol(credentials: &ExternalMemoryProviderCredentials) -> Result<Mem0Protocol> {
    match credentials.protocol.as_str() {
        "platform" | "platform_v3" | "cloud" | "cloud_v3" => Ok(Mem0Protocol::PlatformV3),
        "oss" | "self_hosted" | "self-hosted" => Ok(Mem0Protocol::Oss),
        "auto" => {
            let host = url::Url::parse(&credentials.endpoint)
                .ok()
                .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
            if host.as_deref() == Some("api.mem0.ai") {
                Ok(Mem0Protocol::PlatformV3)
            } else {
                Ok(Mem0Protocol::Oss)
            }
        }
        other => bail!("unsupported Mem0 protocol: {other}"),
    }
}

fn parse_remote_memories(value: &Value) -> Vec<RemoteMemory> {
    let items = value
        .as_array()
        .or_else(|| value.get("results").and_then(Value::as_array))
        .or_else(|| value.get("memories").and_then(Value::as_array));
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("memory_id"))
                .and_then(value_to_id)?;
            let content = item
                .get("memory")
                .or_else(|| item.get("content"))
                .or_else(|| item.get("text"))
                .and_then(Value::as_str)?
                .to_string();
            Some(RemoteMemory {
                id,
                content,
                metadata: item.get("metadata").cloned().unwrap_or(Value::Null),
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

fn memory_is_own_export(provider: &ExternalMemoryProviderConfig, memory: &RemoteMemory) -> bool {
    memory
        .metadata
        .get("hope_agent_provider_id")
        .and_then(Value::as_str)
        == Some(provider.id.as_str())
}

fn local_memory_hash(memory: &MemoryEntry) -> String {
    content_fingerprint(
        &serde_json::to_string(&json!({
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

    #[test]
    fn parses_platform_and_oss_memory_envelopes() {
        let platform = json!({"results": [{"id": "m1", "memory": "alpha", "metadata": {}}]});
        let oss = json!([{"id": 2, "content": "beta"}]);
        assert_eq!(parse_remote_memories(&platform)[0].id, "m1");
        assert_eq!(parse_remote_memories(&oss)[0].id, "2");
    }

    #[test]
    fn own_exports_are_not_reimported() {
        let provider = ExternalMemoryProviderConfig {
            id: "mem0-main".into(),
            kind: crate::memory::ExternalMemoryProviderKind::Mem0,
            display_name: "Mem0".into(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::Bidirectional,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        };
        let memory = RemoteMemory {
            id: "m1".into(),
            content: "alpha".into(),
            metadata: json!({"hope_agent_provider_id": "mem0-main"}),
        };
        assert!(memory_is_own_export(&provider, &memory));
    }
}
