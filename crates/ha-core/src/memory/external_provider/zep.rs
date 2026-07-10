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

pub(super) static ZEP_ADAPTER: ZepAdapter = ZepAdapter;

pub(super) struct ZepAdapter;

const MAX_REMOTE_EPISODES_PER_RUN: usize = 5_000;
const MAX_LOCAL_MEMORIES_PER_RUN: usize = 500;
const LOCAL_MEMORY_SCAN_LIMIT: usize = 20_000;
const PUSH_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZepProtocol {
    GraphitiHttp,
}

#[derive(Debug, Clone)]
struct RemoteEpisode {
    id: String,
    content: String,
}

#[async_trait::async_trait]
impl ExternalMemoryProviderAdapter for ZepAdapter {
    fn kind(&self) -> ExternalMemoryProviderKind {
        ExternalMemoryProviderKind::Zep
    }

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>
    {
        sync_zep(provider).await
    }
}

async fn sync_zep(
    provider: &ExternalMemoryProviderConfig,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
    let (credentials, _) = resolve_external_memory_provider_credentials(&provider.id)
        .map_err(|error| failure(outcome.clone(), error))?
        .ok_or_else(|| failure(outcome.clone(), anyhow!("provider credentials are missing")))?;
    let _protocol =
        resolve_protocol(&credentials).map_err(|error| failure(outcome.clone(), error))?;
    let endpoint = validated_endpoint(&credentials.endpoint)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let client = external_http_client().map_err(|error| failure(outcome.clone(), error))?;
    let mut ledger =
        load_sync_ledger(&provider.id).map_err(|error| failure(outcome.clone(), error))?;

    if provider.sync_policy.imports_external_memory() {
        pull_graphiti_episodes(
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
        push_graphiti_episodes(
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

async fn pull_graphiti_episodes(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let url = endpoint_with_path(endpoint, &["episodes", &credentials.subject_id])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(
        client
            .get(&url)
            .query(&[("last_n", MAX_REMOTE_EPISODES_PER_RUN)]),
        credentials,
    );
    let value = send_json(request, outcome).await?;
    for episode in parse_remote_episodes(&value)
        .into_iter()
        .take(MAX_REMOTE_EPISODES_PER_RUN)
    {
        if episode.content.trim().is_empty() || is_own_export(provider, &episode.id) {
            outcome.skipped_memory_count += 1;
            continue;
        }
        import_external_memory_for_review(
            provider,
            "zep_graphiti",
            &episode.id,
            &episode.content,
            endpoint,
            ledger,
            outcome,
        )
        .map_err(|error| failure(outcome.clone(), error))?;
    }
    emit_import_event(outcome);
    Ok(())
}

async fn push_graphiti_episodes(
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

    let url = endpoint_with_path(endpoint, &["messages"])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    for batch in changed.chunks(PUSH_BATCH_SIZE) {
        let messages = batch
            .iter()
            .map(|(memory, hash)| {
                json!({
                    "uuid": export_episode_id(provider, &memory.id.to_string(), hash),
                    "name": "Hope Agent memory",
                    "role_type": "user",
                    "role": "hope-agent-memory",
                    "content": memory.content,
                    "timestamp": memory.updated_at,
                    "source_description": "Hope Agent local memory sync"
                })
            })
            .collect::<Vec<_>>();
        let request = apply_auth(
            client.post(&url).json(&json!({
                "group_id": credentials.subject_id,
                "messages": messages
            })),
            credentials,
        );
        let _ = send_json(request, outcome).await?;
        for (memory, hash) in batch {
            let old = ledger
                .exported_hashes
                .insert(memory.id.to_string(), hash.clone());
            ledger.exported_remote_ids.insert(
                memory.id.to_string(),
                export_episode_id(provider, &memory.id.to_string(), hash),
            );
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

fn resolve_protocol(credentials: &ExternalMemoryProviderCredentials) -> Result<ZepProtocol> {
    match credentials.protocol.as_str() {
        "auto" | "graphiti" | "graphiti_http" | "graph_service" | "self_hosted" => {
            Ok(ZepProtocol::GraphitiHttp)
        }
        other => bail!("unsupported Zep protocol: {other}; use the official Graphiti HTTP sidecar"),
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

fn parse_remote_episodes(value: &Value) -> Vec<RemoteEpisode> {
    let items = value
        .as_array()
        .or_else(|| value.get("episodes").and_then(Value::as_array))
        .or_else(|| value.get("results").and_then(Value::as_array));
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("uuid")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)?
                .to_string();
            let content = item
                .get("content")
                .or_else(|| item.get("episode_body"))
                .or_else(|| item.get("body"))
                .and_then(Value::as_str)?
                .to_string();
            Some(RemoteEpisode { id, content })
        })
        .collect()
}

fn export_episode_id(
    provider: &ExternalMemoryProviderConfig,
    local_id: &str,
    hash: &str,
) -> String {
    format!(
        "hope-agent-{}-{}-{}",
        provider.id,
        local_id,
        hash.chars().take(12).collect::<String>()
    )
}

fn is_own_export(provider: &ExternalMemoryProviderConfig, remote_id: &str) -> bool {
    remote_id.starts_with(&format!("hope-agent-{}-", provider.id))
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
            id: "zep-main".into(),
            kind: ExternalMemoryProviderKind::Zep,
            display_name: "Graphiti".into(),
            enabled: true,
            sync_policy: ExternalMemorySyncPolicy::Bidirectional,
            endpoint_configured: true,
            last_sync_at: None,
            last_error: None,
        }
    }

    #[test]
    fn parses_graphiti_episode_envelopes() {
        let direct = json!([{"uuid": "e1", "content": "alpha"}]);
        let wrapped = json!({"episodes": [{"id": "e2", "episode_body": "beta"}]});
        assert_eq!(parse_remote_episodes(&direct)[0].content, "alpha");
        assert_eq!(parse_remote_episodes(&wrapped)[0].id, "e2");
    }

    #[test]
    fn deterministic_exports_are_not_reimported() {
        let provider = provider();
        let id = export_episode_id(&provider, "42", "abcdef0123456789");
        assert!(is_own_export(&provider, &id));
    }
}
