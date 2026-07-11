use anyhow::{anyhow, bail, Result};
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};

use crate::memory::{ExternalMemoryProviderConfig, ExternalMemoryProviderKind};

use super::http::{
    client as external_http_client, endpoint_with_path, send_json, validated_endpoint,
};
use super::{
    content_fingerprint, finish_sync_with_ledger_checkpoint, import_external_memory_for_review,
    load_local_memory_snapshot, load_sync_ledger_async, local_memory_fingerprint,
    persist_sync_ledger_async, resolve_external_memory_provider_credentials_async,
    ExternalMemoryAdapterSyncFailure, ExternalMemoryAdapterSyncOutcome,
    ExternalMemoryProviderAdapter, ExternalMemoryProviderCredentials,
    ExternalMemoryProviderSyncLedger,
};

pub(super) static OPEN_VIKING_ADAPTER: OpenVikingAdapter = OpenVikingAdapter;

pub(super) struct OpenVikingAdapter;

const MAX_REMOTE_FILES_PER_RUN: usize = 5_000;
const MAX_REMOTE_FILE_READS_PER_RUN: usize = 200;
const MAX_LOCAL_MEMORIES_PER_RUN: usize = 500;
const LOCAL_MEMORY_SCAN_LIMIT: usize = 20_000;
const PUSH_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenVikingProtocol {
    V1,
}

#[derive(Debug, Clone)]
struct RemoteFile {
    uri: String,
    version: String,
}

#[async_trait::async_trait]
impl ExternalMemoryProviderAdapter for OpenVikingAdapter {
    fn kind(&self) -> ExternalMemoryProviderKind {
        ExternalMemoryProviderKind::OpenViking
    }

    async fn sync(
        &self,
        provider: &ExternalMemoryProviderConfig,
    ) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure>
    {
        sync_open_viking(provider).await
    }
}

async fn sync_open_viking(
    provider: &ExternalMemoryProviderConfig,
) -> std::result::Result<ExternalMemoryAdapterSyncOutcome, ExternalMemoryAdapterSyncFailure> {
    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
    let (credentials, _) = resolve_external_memory_provider_credentials_async(&provider.id)
        .await
        .map_err(|error| failure(outcome.clone(), error))?
        .ok_or_else(|| failure(outcome.clone(), anyhow!("provider credentials are missing")))?;
    let _protocol =
        resolve_protocol(&credentials).map_err(|error| failure(outcome.clone(), error))?;
    let endpoint = validated_endpoint(&credentials.endpoint)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let client = external_http_client().map_err(|error| failure(outcome.clone(), error))?;
    let mut ledger = load_sync_ledger_async(&provider.id)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;

    let sync_result = async {
        if provider.sync_policy.imports_external_memory() {
            pull_memory_files(
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
            push_memory_sessions(
                provider,
                &credentials,
                &endpoint,
                &client,
                &mut ledger,
                &mut outcome,
            )
            .await?;
        }
        Ok(())
    }
    .await;
    finish_sync_with_ledger_checkpoint(&provider.id, &ledger, outcome, sync_result).await
}

async fn pull_memory_files(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let list_url = endpoint_with_path(endpoint, &["api", "v1", "fs", "ls"])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&list_url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let request = apply_auth(
        client.get(&list_url).query(&[
            ("uri", "viking://user/memories/"),
            ("recursive", "true"),
            ("simple", "false"),
            ("output", "original"),
            ("show_all_hidden", "false"),
            ("node_limit", "5000"),
        ]),
        credentials,
    );
    let value = send_json(request, outcome).await?;
    ensure_ok_envelope(&value).map_err(|error| failure(outcome.clone(), error))?;
    let files = parse_remote_files(&value);
    let mut changed = files
        .into_iter()
        .filter(|file| ledger.remote_versions.get(&file.uri) != Some(&file.version))
        .collect::<Vec<_>>();
    if changed.len() > MAX_REMOTE_FILE_READS_PER_RUN {
        outcome.skipped_memory_count += changed.len() - MAX_REMOTE_FILE_READS_PER_RUN;
        changed.truncate(MAX_REMOTE_FILE_READS_PER_RUN);
    }

    let read_url = endpoint_with_path(endpoint, &["api", "v1", "content", "read"])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&read_url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    for file in changed {
        let request = apply_auth(
            client
                .get(&read_url)
                .query(&[("uri", file.uri.as_str()), ("raw", "false")]),
            credentials,
        );
        let value = send_json(request, outcome).await?;
        ensure_ok_envelope(&value).map_err(|error| failure(outcome.clone(), error))?;
        let content = value
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default();
        import_external_memory_for_review(
            provider,
            "open_viking",
            &file.uri,
            content,
            endpoint,
            ledger,
            outcome,
        )
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
        ledger.remote_versions.insert(file.uri, file.version);
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    emit_import_event(outcome);
    Ok(())
}

async fn push_memory_sessions(
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

    for batch in changed.chunks(PUSH_BATCH_SIZE) {
        let batch_hash = content_fingerprint(
            &batch
                .iter()
                .map(|(memory, hash)| format!("{}:{hash}", memory.id))
                .collect::<Vec<_>>()
                .join("|"),
        );
        let session_id = export_session_id(&credentials.subject_id, &batch_hash);
        let messages_url = endpoint_with_path(
            endpoint,
            &["api", "v1", "sessions", &session_id, "messages", "batch"],
        )
        .map_err(|error| failure(outcome.clone(), error))?;
        validated_endpoint(&messages_url)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        let messages = batch
            .iter()
            .map(|(memory, _)| {
                json!({
                    "role": "user",
                    "content": memory.content,
                    "created_at": memory.updated_at
                })
            })
            .collect::<Vec<_>>();
        let request = apply_auth(
            client
                .post(&messages_url)
                .json(&json!({"messages": messages})),
            credentials,
        );
        let response = send_json(request, outcome).await?;
        ensure_ok_envelope(&response).map_err(|error| failure(outcome.clone(), error))?;

        let commit_url =
            endpoint_with_path(endpoint, &["api", "v1", "sessions", &session_id, "commit"])
                .map_err(|error| failure(outcome.clone(), error))?;
        validated_endpoint(&commit_url)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        let request = apply_auth(
            client
                .post(&commit_url)
                .json(&json!({"keep_recent_count": 0})),
            credentials,
        );
        let response = send_json(request, outcome).await?;
        ensure_ok_envelope(&response).map_err(|error| failure(outcome.clone(), error))?;

        for (memory, hash) in batch {
            let old = ledger
                .exported_hashes
                .insert(memory.id.to_string(), hash.clone());
            ledger
                .exported_remote_ids
                .insert(memory.id.to_string(), session_id.clone());
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

fn resolve_protocol(credentials: &ExternalMemoryProviderCredentials) -> Result<OpenVikingProtocol> {
    match credentials.protocol.as_str() {
        "auto" | "v1" | "rest" | "self_hosted" => Ok(OpenVikingProtocol::V1),
        other => bail!("unsupported OpenViking protocol: {other}"),
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

fn ensure_ok_envelope(value: &Value) -> Result<()> {
    match value.get("status").and_then(Value::as_str) {
        Some("ok") => Ok(()),
        Some("error") => {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("OpenViking operation failed");
            bail!("OpenViking operation failed: {message}")
        }
        _ => bail!("OpenViking response omitted the status envelope"),
    }
}

fn parse_remote_files(value: &Value) -> Vec<RemoteFile> {
    let Some(items) = value.get("result").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.get("isDir").and_then(Value::as_bool) != Some(true))
        .filter_map(|item| {
            let uri = item.get("uri")?.as_str()?.to_string();
            if uri
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with('.'))
            {
                return None;
            }
            let version = format!(
                "{}:{}",
                item.get("modTime").and_then(Value::as_str).unwrap_or(""),
                item.get("size").and_then(Value::as_u64).unwrap_or(0)
            );
            Some(RemoteFile { uri, version })
        })
        .take(MAX_REMOTE_FILES_PER_RUN)
        .collect()
}

fn export_session_id(subject_id: &str, hash: &str) -> String {
    let safe_subject = subject_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!(
        "{}-hope-{}",
        safe_subject.chars().take(80).collect::<String>(),
        hash.chars().take(16).collect::<String>()
    )
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
    fn parses_only_visible_memory_files() {
        let value = json!({
            "status": "ok",
            "result": [
                {"uri": "viking://user/memories/profile/alice.md", "isDir": false, "modTime": "2026-01-01", "size": 12},
                {"uri": "viking://user/memories/profile/.abstract.md", "isDir": false},
                {"uri": "viking://user/memories/profile/", "isDir": true}
            ]
        });
        let files = parse_remote_files(&value);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].version, "2026-01-01:12");
    }

    #[test]
    fn session_ids_are_bounded_and_path_safe() {
        let id = export_session_id("user/with spaces", "abcdef0123456789");
        assert_eq!(id, "user-with-spaces-hope-abcdef0123456789");
    }
}
