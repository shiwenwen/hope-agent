use anyhow::{anyhow, bail, Result};
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use ha_core::memory::{ExternalMemoryProviderConfig, ExternalMemoryProviderKind};

use super::http::{
    client as external_http_client, endpoint_with_path, send_json, validated_endpoint,
};
use super::{
    compatibility_credential_fingerprint, compatible_provider_version_for_sync,
    content_fingerprint, finish_sync_with_ledger_checkpoint, import_external_memory_for_review,
    load_local_memory_snapshot, load_sync_ledger_async, local_memory_fingerprint, parse_version,
    persist_sync_ledger_async, resolve_external_memory_provider_credentials_async,
    version_meets_minimum, ExternalMemoryAdapterSyncFailure, ExternalMemoryAdapterSyncOutcome,
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

/// A write-ahead fence, not proof that any remote mutation completed. No text
/// or credentials are stored. A missing task ID requires owner reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingExport {
    credential_fingerprint: String,
    hashes: BTreeMap<String, String>,
    task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenVikingProtocol {
    V1LegacyCurrentUser,
    V1TildeCurrentUser,
}

impl OpenVikingProtocol {
    fn current_user_memories_uri(self) -> &'static str {
        match self {
            Self::V1LegacyCurrentUser => "viking://user/memories/",
            Self::V1TildeCurrentUser => "viking://~/memories",
        }
    }
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
    let detected_version = compatible_provider_version_for_sync(provider.clone())
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let protocol = resolve_protocol(&credentials, &detected_version)
        .map_err(|error| failure(outcome.clone(), error))?;
    let endpoint = validated_endpoint(&credentials.endpoint)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    let client = external_http_client().map_err(|error| failure(outcome.clone(), error))?;
    let mut ledger = load_sync_ledger_async(&provider.id)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;

    let sync_result = sync_operations(
        provider,
        &credentials,
        protocol,
        &endpoint,
        &client,
        &mut ledger,
        &mut outcome,
    )
    .await;
    finish_sync_with_ledger_checkpoint(&provider.id, &ledger, outcome, sync_result).await
}

async fn sync_operations(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    protocol: OpenVikingProtocol,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    // Existing writes must reach a terminal state even if the owner switches
    // to PullOnly. The live policy still controls all new memory writes.
    for session_id in ledger
        .open_viking_pending_exports
        .keys()
        .cloned()
        .collect::<Vec<_>>()
    {
        reconcile_pending_export(credentials, endpoint, client, ledger, outcome, &session_id)
            .await?;
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    if provider.sync_policy.imports_external_memory() {
        pull_memory_files(
            provider,
            credentials,
            protocol,
            endpoint,
            client,
            ledger,
            outcome,
        )
        .await?;
    }
    if provider.sync_policy.sends_local_memory() {
        push_memory_sessions(provider, credentials, endpoint, client, ledger, outcome).await?;
    }
    Ok(())
}

async fn pull_memory_files(
    provider: &ExternalMemoryProviderConfig,
    credentials: &ExternalMemoryProviderCredentials,
    protocol: OpenVikingProtocol,
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
            ("uri", protocol.current_user_memories_uri()),
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
    let files = parse_remote_files(&value).map_err(|error| failure(outcome.clone(), error))?;
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
        let commit_url =
            endpoint_with_path(endpoint, &["api", "v1", "sessions", &session_id, "commit"])
                .map_err(|error| failure(outcome.clone(), error))?;
        validated_endpoint(&commit_url)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        ledger.open_viking_pending_exports.insert(
            session_id.clone(),
            PendingExport {
                credential_fingerprint: compatibility_credential_fingerprint(
                    ExternalMemoryProviderKind::OpenViking,
                    credentials,
                )
                .map_err(|error| failure(outcome.clone(), error))?,
                hashes: batch
                    .iter()
                    .map(|(memory, hash)| (memory.id.to_string(), hash.clone()))
                    .collect(),
                task_id: None,
            },
        );
        // Any failure/cancellation after this checkpoint must never re-add the
        // messages or re-commit the session automatically, even after restart.
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        let response = send_json(request, outcome).await?;
        ensure_ok_envelope(&response).map_err(|error| failure(outcome.clone(), error))?;

        let request = apply_auth(
            client
                .post(&commit_url)
                .json(&json!({"keep_recent_count": 0})),
            credentials,
        );
        let response = send_json(request, outcome).await?;
        let task_id = accepted_commit_task(&response, &session_id)
            .map_err(|error| failure(outcome.clone(), error))?;
        if let Some(pending) = ledger.open_viking_pending_exports.get_mut(&session_id) {
            pending.task_id = Some(task_id);
        }
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
        reconcile_pending_export(credentials, endpoint, client, ledger, outcome, &session_id)
            .await?;
        persist_sync_ledger_async(&provider.id, ledger)
            .await
            .map_err(|error| failure(outcome.clone(), error))?;
    }
    Ok(())
}

fn accepted_commit_task(value: &Value, session_id: &str) -> Result<String> {
    ensure_ok_envelope(value)?;
    let result = &value["result"];
    if result["session_id"].as_str() != Some(session_id)
        || result["status"].as_str() != Some("accepted")
        || result["archived"].as_bool() != Some(true)
    {
        bail!("OpenViking commit is not an accepted archive; owner reconciliation required");
    }
    let task_id = result["task_id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        })
        .ok_or_else(|| {
            anyhow!("OpenViking commit omitted a valid task ID; owner reconciliation required")
        })?;
    Ok(task_id.to_owned())
}

fn ensure_completed_task(value: &Value, task_id: &str, session_id: &str) -> Result<()> {
    ensure_ok_envelope(value)?;
    let task = &value["result"];
    if task["task_id"].as_str() != Some(task_id)
        || task["task_type"].as_str() != Some("session_commit")
        || task["resource_id"].as_str() != Some(session_id)
    {
        bail!("OpenViking task identity mismatch; owner reconciliation required");
    }
    match task["status"].as_str() {
        Some("pending" | "running" | "cancelling") => bail!("OpenViking export is awaiting a terminal task; next sync will only check its status"),
        Some("completed") => {},
        _ => bail!("OpenViking task failed, cancelled or has an unknown status; owner reconciliation required"),
    }
    let result = &task["result"];
    let counts_valid = result["memories_extracted"]
        .as_object()
        .is_some_and(|counts| counts.values().all(|v| v.as_u64().is_some()));
    // Legacy task receipts expose only a memory-diff URI. Counts alone do not
    // prove that every operation succeeded: missing skip evidence stays fenced
    // until the owner reconciles it, rather than publishing every input hash.
    let skipped_valid = result.get("memory_extraction").is_some_and(|extraction| {
        extraction["skipped"].as_u64() == Some(0)
            && extraction["skipped_operations"]
                .as_array()
                .is_some_and(Vec::is_empty)
    });
    if !task["error"].is_null()
        || result["session_id"].as_str() != Some(session_id)
        || !counts_valid
        || !skipped_valid
        || !result["user_config_error"].is_null()
    {
        bail!("OpenViking completed task has invalid or incomplete extraction results; owner reconciliation required");
    }
    // An empty counts object is a legitimate completed zero-change extraction.
    Ok(())
}

fn pending_task_id<'a>(
    pending: &'a PendingExport,
    credentials: &ExternalMemoryProviderCredentials,
) -> Result<&'a str> {
    let fingerprint =
        compatibility_credential_fingerprint(ExternalMemoryProviderKind::OpenViking, credentials)?;
    if pending.credential_fingerprint != fingerprint {
        bail!("OpenViking pending export credentials changed; owner reconciliation required");
    }
    pending.task_id.as_deref().ok_or_else(|| {
        anyhow!("OpenViking export has an uncertain remote write; owner reconciliation required")
    })
}

async fn reconcile_pending_export(
    credentials: &ExternalMemoryProviderCredentials,
    endpoint: &str,
    client: &Client,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
    session_id: &str,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let pending = ledger
        .open_viking_pending_exports
        .get(session_id)
        .ok_or_else(|| {
            failure(
                outcome.clone(),
                anyhow!("OpenViking pending export missing"),
            )
        })?;
    let task_id =
        pending_task_id(pending, credentials).map_err(|error| failure(outcome.clone(), error))?;
    let task_url = endpoint_with_path(endpoint, &["api", "v1", "tasks", task_id])
        .map_err(|error| failure(outcome.clone(), error))?;
    validated_endpoint(&task_url)
        .await
        .map_err(|error| failure(outcome.clone(), error))?;
    poll_pending_export(credentials, client, &task_url, ledger, outcome, session_id).await
}

// The production caller validates the URL before entering this wire operation.
async fn poll_pending_export(
    credentials: &ExternalMemoryProviderCredentials,
    client: &Client,
    task_url: &str,
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
    session_id: &str,
) -> std::result::Result<(), ExternalMemoryAdapterSyncFailure> {
    let pending = ledger
        .open_viking_pending_exports
        .get(session_id)
        .ok_or_else(|| {
            failure(
                outcome.clone(),
                anyhow!("OpenViking pending export missing"),
            )
        })?;
    let task_id =
        pending_task_id(pending, credentials).map_err(|error| failure(outcome.clone(), error))?;
    let value = send_json(apply_auth(client.get(task_url), credentials), outcome).await?;
    ensure_completed_task(&value, task_id, session_id)
        .map_err(|error| failure(outcome.clone(), error))?;
    publish_completed_export(ledger, outcome, session_id);
    Ok(())
}

fn publish_completed_export(
    ledger: &mut ExternalMemoryProviderSyncLedger,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
    session_id: &str,
) {
    if let Some(pending) = ledger.open_viking_pending_exports.remove(session_id) {
        for (id, hash) in pending.hashes {
            let old = ledger.exported_hashes.insert(id.clone(), hash);
            ledger.exported_remote_ids.insert(id, session_id.to_owned());
            if old.is_some() {
                outcome.updated_memory_count += 1;
            } else {
                outcome.exported_memory_count += 1;
            }
        }
    }
}

fn resolve_protocol(
    credentials: &ExternalMemoryProviderCredentials,
    detected_version: &str,
) -> Result<OpenVikingProtocol> {
    if !matches!(
        credentials.protocol.as_str(),
        "auto" | "v1" | "rest" | "self_hosted"
    ) {
        bail!("unsupported OpenViking protocol: {}", credentials.protocol);
    }
    parse_version(detected_version)
        .ok_or_else(|| anyhow!("OpenViking version is not a supported semantic version"))?;
    Ok(if version_meets_minimum(detected_version, "0.4.17") {
        OpenVikingProtocol::V1TildeCurrentUser
    } else {
        OpenVikingProtocol::V1LegacyCurrentUser
    })
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

fn parse_remote_files(value: &Value) -> Result<Vec<RemoteFile>> {
    let items = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenViking listing omitted its entries array"))?;
    let has_more = match value.get("has_more") {
        None => None,
        Some(Value::Bool(has_more)) => Some(*has_more),
        Some(_) => bail!("OpenViking listing has an invalid continuation flag"),
    };
    // Old servers omit has_more. Reaching their node cap is ambiguous even
    // when all returned files are already in the ledger. Check raw nodes,
    // including directories/hidden files, before filtering or reading any.
    // Offset paging without a stable snapshot can skip moving entries; do not
    // guess a continuation or mark a truncated prefix as a successful sync.
    if has_more == Some(true)
        || items.len() > MAX_REMOTE_FILES_PER_RUN
        || (has_more.is_none() && items.len() == MAX_REMOTE_FILES_PER_RUN)
    {
        bail!("OpenViking memory listing is incomplete; bounded continuation is not supported");
    }
    Ok(items
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
        .collect())
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
        ha_core::memory::emit_claim_changed("external_provider_import", None, Some(changed));
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn accepted() -> Value {
        json!({"status":"ok","result":{"session_id":"owner-hope-batch","status":"accepted","archived":true,"task_id":"task-1"}})
    }

    fn completed() -> Value {
        json!({"status":"ok","result":{"task_id":"task-1","task_type":"session_commit","resource_id":"owner-hope-batch","status":"completed","error":null,"result":{"session_id":"owner-hope-batch","memories_extracted":{},"memory_extraction":{"skipped":0,"skipped_operations":[]}}}})
    }

    fn pending_ledger() -> ExternalMemoryProviderSyncLedger {
        let pending = PendingExport {
            credential_fingerprint: compatibility_credential_fingerprint(
                ExternalMemoryProviderKind::OpenViking,
                &credentials(),
            )
            .unwrap(),
            hashes: BTreeMap::from([("7".to_owned(), "input-hash".to_owned())]),
            task_id: Some(accepted_commit_task(&accepted(), "owner-hope-batch").unwrap()),
        };
        ExternalMemoryProviderSyncLedger {
            open_viking_pending_exports: BTreeMap::from([("owner-hope-batch".to_owned(), pending)]),
            ..Default::default()
        }
    }

    #[test]
    fn accepted_archive_is_never_terminal_evidence() {
        assert!(ensure_completed_task(&accepted(), "task-1", "owner-hope-batch").is_err());
        for invalid in [
            Value::Null,
            json!({"status":"ok"}),
            json!({"status":"ok","result":{"status":"skipped"}}),
        ] {
            assert!(accepted_commit_task(&invalid, "owner-hope-batch").is_err());
        }
        let mut value = accepted();
        value["result"]["session_id"] = json!("other");
        assert!(accepted_commit_task(&value, "owner-hope-batch").is_err());
        let mut value = accepted();
        value["result"]["task_id"] = json!("../other");
        assert!(accepted_commit_task(&value, "owner-hope-batch").is_err());
    }

    #[test]
    fn completed_extraction_rejects_malformed_partial_and_wrong_identity_results() {
        assert!(ensure_completed_task(&completed(), "task-1", "owner-hope-batch").is_ok());
        for (pointer, replacement) in [
            ("/result/task_id", json!("other")),
            ("/result/task_type", json!("add_resource")),
            ("/result/resource_id", json!("other")),
            ("/result/error", json!("empty_response")),
            ("/result/result/session_id", json!("other")),
            ("/result/result/memories_extracted", Value::Null),
            ("/result/result/memories_extracted", json!({"facts":-1})),
            ("/result/result/memory_extraction", Value::Null),
            ("/result/result/memory_extraction", json!({})),
            ("/result/result/memory_extraction/skipped", json!(1)),
            (
                "/result/result/memory_extraction/skipped_operations",
                json!([{"reason":"parse_error"}]),
            ),
        ] {
            let mut value = completed();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                ensure_completed_task(&value, "task-1", "owner-hope-batch").is_err(),
                "{pointer}"
            );
        }
        let mut value = completed();
        value["result"]["result"]["user_config_error"] = json!("bad config");
        assert!(ensure_completed_task(&value, "task-1", "owner-hope-batch").is_err());
        let mut legacy = completed();
        legacy["result"]["result"] = json!({
            "session_id":"owner-hope-batch",
            "memories_extracted":{"facts":1},
            "memory_diff_uri":"viking://user/owner/sessions/owner-hope-batch/history/archive_001/memory_diff.json"
        });
        assert!(ensure_completed_task(&legacy, "task-1", "owner-hope-batch").is_err());
    }

    #[test]
    fn pending_fence_survives_restart_without_body_and_rejects_identity_changes() {
        let ledger: ExternalMemoryProviderSyncLedger =
            serde_json::from_slice(&serde_json::to_vec(&pending_ledger()).unwrap()).unwrap();
        let pending = &ledger.open_viking_pending_exports["owner-hope-batch"];
        assert!(ledger.exported_hashes.is_empty());
        assert_eq!(pending_task_id(pending, &credentials()).unwrap(), "task-1");
        for field in ["endpoint", "apiKey", "subjectId", "protocol"] {
            let mut changed = serde_json::to_value(credentials()).unwrap();
            changed[field] = json!("changed");
            let changed = serde_json::from_value(changed).unwrap();
            assert!(pending_task_id(pending, &changed).is_err(), "{field}");
        }
        let mut uncertain = pending.clone();
        uncertain.task_id = None;
        assert!(pending_task_id(&uncertain, &credentials()).is_err());
        // Old ledgers remain readable without a schema rewrite.
        let old: ExternalMemoryProviderSyncLedger =
            serde_json::from_value(json!({"schemaVersion":1})).unwrap();
        assert!(old.open_viking_pending_exports.is_empty());
    }

    #[test]
    fn owner_lifecycle_preserves_fences_and_pull_only_reconciles_without_posts() {
        let temp = tempfile::tempdir().unwrap();
        ha_core::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            struct RestoreCache(std::sync::Arc<ha_core::config::AppConfig>);
            impl Drop for RestoreCache {
                fn drop(&mut self) {
                    ha_core::config::replace_cache_for_test((*self.0).clone());
                }
            }
            let _restore = RestoreCache(ha_core::config::cached_config());
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let server = MockServer::start().await;
                    let credentials = ExternalMemoryProviderCredentials {
                        endpoint: server.uri(),
                        ..credentials()
                    };
                    let provider = ExternalMemoryProviderConfig {
                        id: "pending-openviking".to_owned(),
                        kind: ExternalMemoryProviderKind::OpenViking,
                        display_name: "OpenViking".to_owned(),
                        enabled: true,
                        sync_policy: ha_core::memory::ExternalMemorySyncPolicy::PullOnly,
                        endpoint_configured: true,
                        last_sync_at: None,
                        last_error: None,
                    };
                    let mut config = ha_core::config::AppConfig::default();
                    config.ssrf.trusted_hosts = vec!["127.0.0.1".to_owned()];
                    config.memory_providers.enabled = true;
                    config.memory_providers.providers = vec![provider.clone()];
                    std::fs::write(
                        temp.path().join("config.json"),
                        serde_json::to_vec(&config).unwrap(),
                    )
                    .unwrap();
                    ha_core::config::replace_cache_for_test(config);
                    super::super::persist_credentials(&provider.id, &credentials).unwrap();
                    let mut ledger = pending_ledger();
                    ledger.schema_version = super::super::SYNC_STATE_SCHEMA_VERSION;
                    ledger.open_viking_pending_exports
                        .get_mut("owner-hope-batch")
                        .unwrap()
                        .credential_fingerprint = compatibility_credential_fingerprint(
                        ExternalMemoryProviderKind::OpenViking,
                        &credentials,
                    )
                    .unwrap();
                    persist_sync_ledger_async(&provider.id, &ledger).await.unwrap();
                    let original = serde_json::to_vec(&ledger).unwrap();
                    // Both owner-plane deletion paths must revoke credentials
                    // but keep the fence through deterministic-ID recreation.
                    let configured = ha_core::config::cached_config().memory_providers.clone();
                    for patch in [false, true] {
                        let compatibility_path = ha_core::paths::external_memory_compatibility_path(&provider.id).unwrap();
                        if patch {
                            // Auxiliary cleanup failure must not trap a key.
                            std::fs::create_dir(&compatibility_path).unwrap();
                            super::super::patch_external_memory_providers_config(
                                json!({"removeProviderIds":[provider.id]}), "test"
                            ).unwrap();
                            std::fs::remove_dir(&compatibility_path).unwrap();
                        } else {
                            std::fs::write(&compatibility_path, b"{}").unwrap();
                            let mut removed = configured.clone();
                            removed.providers.clear();
                            super::super::save_external_memory_providers_config(removed, "test").unwrap();
                            assert!(!compatibility_path.exists());
                        }
                        assert!(ha_core::config::cached_config().memory_providers.providers.is_empty());
                        assert!(super::super::load_credentials_file(&provider.id).unwrap().is_none());
                        assert_eq!(serde_json::to_vec(&load_sync_ledger_async(&provider.id).await.unwrap()).unwrap(), original);
                        super::super::save_external_memory_providers_config(configured.clone(), "test").unwrap();
                        super::super::save_external_memory_provider_credentials(
                            ha_core::memory::external_provider::ExternalMemoryProviderCredentialInput {
                                provider_id: provider.id.clone(), endpoint: server.uri(), api_key: None,
                                subject_id: "owner".to_owned(), protocol: Some("auto".to_owned()),
                            }
                        ).await.unwrap();
                        assert_eq!(serde_json::to_vec(&load_sync_ledger_async(&provider.id).await.unwrap()).unwrap(), original);
                        assert_eq!(pending_task_id(&load_sync_ledger_async(&provider.id).await.unwrap().open_viking_pending_exports["owner-hope-batch"], &credentials).unwrap(), "task-1");
                    }
                    // Real save/clear paths, not only the pending validator.
                    for (endpoint, subject, protocol) in [
                        (server.uri(), "other", "auto"),
                        (format!("{}/other", server.uri()), "owner", "v1"),
                    ] {
                        super::super::save_external_memory_provider_credentials(
                            ha_core::memory::external_provider::ExternalMemoryProviderCredentialInput {
                                provider_id: provider.id.clone(),
                                endpoint,
                                api_key: None,
                                subject_id: subject.to_owned(),
                                protocol: Some(protocol.to_owned()),
                            },
                        )
                        .await
                        .unwrap();
                        let preserved = load_sync_ledger_async(&provider.id).await.unwrap();
                        assert_eq!(serde_json::to_vec(&preserved).unwrap(), original);
                    }
                    super::super::clear_external_memory_provider_credentials(&provider.id).unwrap();
                    assert!(super::super::load_credentials_file(&provider.id).unwrap().is_none());
                    let mut ledger = load_sync_ledger_async(&provider.id).await.unwrap();
                    assert_eq!(serde_json::to_vec(&ledger).unwrap(), original);
                    super::super::persist_credentials(&provider.id, &credentials).unwrap();
                    Mock::given(method("GET"))
                        .and(path("/api/v1/tasks/task-1"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(completed()))
                        .expect(1)
                        .mount(&server)
                        .await;
                    Mock::given(method("GET"))
                        .and(path("/api/v1/fs/ls"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"ok","result":[]})))
                        .expect(1)
                        .mount(&server)
                        .await;
                    let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
                    sync_operations(
                        &provider,
                        &credentials,
                        OpenVikingProtocol::V1TildeCurrentUser,
                        &credentials.endpoint,
                        &external_http_client().unwrap(),
                        &mut ledger,
                        &mut outcome,
                    )
                    .await
                    .unwrap();
                    assert_eq!(outcome.exported_memory_count, 1);
                    assert!(ledger.open_viking_pending_exports.is_empty());
                    assert!(load_sync_ledger_async(&provider.id)
                        .await.unwrap().open_viking_pending_exports.is_empty());
                    let requests = server.received_requests().await.unwrap();
                    assert_eq!(requests.len(), 2);
                    assert!(requests.iter().all(|r| r.method == "GET"));
                    // Normal clearing resumes only after terminal publication.
                    super::super::clear_external_memory_provider_credentials(&provider.id).unwrap();
                    assert!(load_sync_ledger_async(&provider.id).await.unwrap().exported_hashes.is_empty());
                    let mut removable = load_sync_ledger_async(&provider.id).await.unwrap();
                    removable.schema_version = super::super::SYNC_STATE_SCHEMA_VERSION;
                    persist_sync_ledger_async(&provider.id, &removable).await.unwrap();
                    let mut removed = configured.clone();
                    removed.providers.clear();
                    super::super::save_external_memory_providers_config(removed, "test").unwrap();
                    assert!(!ha_core::paths::external_memory_sync_state_path(&provider.id).unwrap().exists());
                    super::super::save_external_memory_providers_config(configured.clone(), "test").unwrap();
                    // Broken auxiliary state must not trap an owner's key.
                    let ledger_path = ha_core::paths::external_memory_sync_state_path(&provider.id).unwrap();
                    let compatibility_path = ha_core::paths::external_memory_compatibility_path(&provider.id).unwrap();
                    for invalid in [b"not JSON".as_slice(), b"{\"schemaVersion\":99}".as_slice(), b"directory".as_slice()] {
                        if invalid == b"directory" {
                            std::fs::create_dir(&ledger_path).unwrap();
                            std::fs::create_dir(&compatibility_path).unwrap();
                        } else {
                            std::fs::write(&ledger_path, invalid).unwrap();
                        }
                        super::super::persist_credentials(&provider.id, &credentials).unwrap();
                        super::super::save_external_memory_provider_credentials(
                            ha_core::memory::external_provider::ExternalMemoryProviderCredentialInput {
                                provider_id: provider.id.clone(),
                                endpoint: server.uri(),
                                api_key: Some("new-test-key".to_owned()),
                                subject_id: "changed-owner".to_owned(),
                                protocol: Some("v1".to_owned()),
                            },
                        ).await.unwrap();
                        assert_eq!(super::super::load_credentials_file(&provider.id).unwrap().unwrap().api_key.as_deref(), Some("new-test-key"));
                        super::super::clear_external_memory_provider_credentials(&provider.id).unwrap();
                        assert!(super::super::load_credentials_file(&provider.id).unwrap().is_none());
                        super::super::persist_credentials(&provider.id, &credentials).unwrap();
                        let mut removed = configured.clone();
                        removed.providers.clear();
                        super::super::save_external_memory_providers_config(removed, "test").unwrap();
                        assert!(super::super::load_credentials_file(&provider.id).unwrap().is_none());
                        super::super::save_external_memory_providers_config(configured.clone(), "test").unwrap();
                        if invalid == b"directory" {
                            assert!(ledger_path.is_dir());
                            assert!(compatibility_path.is_dir());
                            std::fs::remove_dir(&ledger_path).unwrap();
                            std::fs::remove_dir(&compatibility_path).unwrap();
                        } else {
                            assert_eq!(std::fs::read(&ledger_path).unwrap(), invalid);
                            std::fs::remove_file(&ledger_path).unwrap();
                        }
                    }
                });
        });
    }

    #[tokio::test]
    async fn resumed_wire_only_polls_and_publishes_after_valid_terminal_result() {
        // This tests wire receipt shapes, not a real deployment/version matrix.
        let server = MockServer::start().await;
        let mut ledger = pending_ledger();
        let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
        let client = external_http_client().unwrap();
        let task_url = format!("{}/api/v1/tasks/task-1", server.uri());
        for status in [
            "pending",
            "running",
            "cancelling",
            "failed",
            "cancelled",
            "unknown",
        ] {
            let mut value = completed();
            value["result"]["status"] = json!(status);
            Mock::given(method("GET"))
                .and(path("/api/v1/tasks/task-1"))
                .respond_with(ResponseTemplate::new(200).set_body_json(value))
                .expect(1)
                .mount(&server)
                .await;
            assert!(poll_pending_export(
                &credentials(),
                &client,
                &task_url,
                &mut ledger,
                &mut outcome,
                "owner-hope-batch"
            )
            .await
            .is_err());
            assert!(ledger.exported_hashes.is_empty());
            assert_eq!(outcome.exported_memory_count, 0);
            assert_eq!(ledger.open_viking_pending_exports.len(), 1);
            server.reset().await;
        }
        let mut legacy = completed();
        legacy["result"]["result"]
            .as_object_mut()
            .unwrap()
            .remove("memory_extraction");
        legacy["result"]["result"]["memories_extracted"] = json!({"facts":1});
        legacy["result"]["result"]["memory_diff_uri"] = json!(
            "viking://user/owner/sessions/owner-hope-batch/history/archive_001/memory_diff.json"
        );
        let mut skipped = completed();
        skipped["result"]["result"]["memory_extraction"] =
            json!({"skipped":1,"skipped_operations":[{"reason_code":"invalid_ranges"}]});
        for value in [legacy, skipped] {
            Mock::given(method("GET"))
                .and(path("/api/v1/tasks/task-1"))
                .respond_with(ResponseTemplate::new(200).set_body_json(value))
                .expect(1)
                .mount(&server)
                .await;
            assert!(poll_pending_export(
                &credentials(),
                &client,
                &task_url,
                &mut ledger,
                &mut outcome,
                "owner-hope-batch"
            )
            .await
            .is_err());
            assert!(ledger.exported_hashes.is_empty());
            assert_eq!(outcome.exported_memory_count, 0);
            assert_eq!(ledger.open_viking_pending_exports.len(), 1);
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].method, "GET");
            server.reset().await;
        }
        for response in [
            ResponseTemplate::new(404),
            ResponseTemplate::new(204),
            ResponseTemplate::new(200).set_body_string("not JSON"),
        ] {
            Mock::given(method("GET"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            assert!(poll_pending_export(
                &credentials(),
                &client,
                &task_url,
                &mut ledger,
                &mut outcome,
                "owner-hope-batch"
            )
            .await
            .is_err());
            assert!(ledger.exported_hashes.is_empty());
            server.reset().await;
        }
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/task-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(completed()))
            .expect(1)
            .mount(&server)
            .await;
        poll_pending_export(
            &credentials(),
            &client,
            &task_url,
            &mut ledger,
            &mut outcome,
            "owner-hope-batch",
        )
        .await
        .unwrap();
        assert_eq!(outcome.exported_memory_count, 1);
        assert_eq!(ledger.exported_hashes["7"], "input-hash");
        assert!(ledger.open_viking_pending_exports.is_empty());
    }

    fn credentials() -> ExternalMemoryProviderCredentials {
        ExternalMemoryProviderCredentials {
            schema_version: 1,
            endpoint: "https://memory.example.test".to_string(),
            api_key: None,
            subject_id: "owner".to_string(),
            protocol: "auto".to_string(),
        }
    }

    #[test]
    fn current_user_uri_is_selected_from_verified_server_version() {
        assert_eq!(
            resolve_protocol(&credentials(), "0.4.16")
                .unwrap()
                .current_user_memories_uri(),
            "viking://user/memories/"
        );
        assert_eq!(
            resolve_protocol(&credentials(), "0.4.17")
                .unwrap()
                .current_user_memories_uri(),
            "viking://~/memories"
        );
        assert_eq!(
            resolve_protocol(&credentials(), "0.4.17.1")
                .unwrap()
                .current_user_memories_uri(),
            "viking://~/memories"
        );
    }

    #[test]
    fn current_user_uri_rejects_unknown_or_prerelease_versions() {
        assert!(resolve_protocol(&credentials(), "unknown").is_err());
        assert_eq!(
            resolve_protocol(&credentials(), "0.4.17-rc.1")
                .unwrap()
                .current_user_memories_uri(),
            "viking://user/memories/"
        );
    }

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
        let files = parse_remote_files(&value).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].version, "2026-01-01:12");
    }

    fn listing_nodes(count: usize) -> Vec<Value> {
        (0..count)
            .map(|n| json!({"uri":format!("viking://~/memories/{n}.md"),"isDir":false,"modTime":"old","size":1}))
            .collect()
    }

    #[test]
    fn listing_completeness_is_checked_before_filtering_or_truncation() {
        let nodes = listing_nodes(MAX_REMOTE_FILES_PER_RUN);
        assert_eq!(
            parse_remote_files(&json!({"result":nodes,"has_more":false}))
                .unwrap()
                .len(),
            MAX_REMOTE_FILES_PER_RUN
        );
        assert!(parse_remote_files(&json!({"result":nodes})).is_err());
        assert!(parse_remote_files(&json!({"result":nodes,"has_more":true})).is_err());
        let mut overflow = listing_nodes(MAX_REMOTE_FILES_PER_RUN + 1);
        overflow.last_mut().unwrap()["modTime"] = json!("changed");
        assert!(parse_remote_files(&json!({"result":overflow,"has_more":false})).is_err());
        // Directories count against the server's node limit too.
        assert!(parse_remote_files(
            &json!({"result":vec![json!({"isDir":true});MAX_REMOTE_FILES_PER_RUN]})
        )
        .is_err());
        assert!(parse_remote_files(&json!({"result":[],"has_more":true})).is_err());
        for invalid in [Value::Null, json!("false"), json!(0), json!({})] {
            assert!(parse_remote_files(&json!({"result":[],"has_more":invalid})).is_err());
        }
        for invalid in [
            json!({}),
            json!({"result":null}),
            json!({"result":{"entries":[],"has_more":false}}),
        ] {
            assert!(parse_remote_files(&invalid).is_err());
        }
        assert!(parse_remote_files(&json!({"result":[]}))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn incomplete_listing_never_reads_content_advances_ledger_or_starts_export() {
        let temp = tempfile::tempdir().unwrap();
        ha_core::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            struct RestoreCache(std::sync::Arc<ha_core::config::AppConfig>);
            impl Drop for RestoreCache {
                fn drop(&mut self) {
                    ha_core::config::replace_cache_for_test((*self.0).clone());
                }
            }
            let _restore = RestoreCache(ha_core::config::cached_config());
            let mut config = ha_core::config::AppConfig::default();
            config.ssrf.trusted_hosts = vec!["127.0.0.1".to_owned()];
            ha_core::config::replace_cache_for_test(config);
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let server = MockServer::start().await;
                    let credentials = ExternalMemoryProviderCredentials {
                        endpoint: server.uri(),
                        ..credentials()
                    };
                    let provider = ExternalMemoryProviderConfig {
                        id: "incomplete-openviking".to_owned(),
                        kind: ExternalMemoryProviderKind::OpenViking,
                        display_name: "OpenViking".to_owned(),
                        enabled: true,
                        sync_policy: ha_core::memory::ExternalMemorySyncPolicy::Bidirectional,
                        endpoint_configured: true,
                        last_sync_at: None,
                        last_error: None,
                    };
                    let nodes = listing_nodes(MAX_REMOTE_FILES_PER_RUN);
                    let mut ledger = ExternalMemoryProviderSyncLedger {
                        remote_versions: nodes.iter().map(|n| (n["uri"].as_str().unwrap().to_owned(), "old:1".to_owned())).collect(),
                        ..Default::default()
                    };
                    let before = serde_json::to_vec(&ledger).unwrap();
                    let client = external_http_client().unwrap();
                    // A complete first scan has no changed prefix. Later the
                    // list grows beyond the cap and only the omitted tail
                    // changes: neither scan may invent a successful suffix.
                    for protocol in [OpenVikingProtocol::V1LegacyCurrentUser, OpenVikingProtocol::V1TildeCurrentUser] {
                        for value in [
                            json!({"status":"ok","result":nodes,"has_more":false}),
                            json!({"status":"ok","result":nodes,"has_more":true}),
                            json!({"status":"ok","result":nodes}),
                            json!({"status":"ok","result":listing_nodes(MAX_REMOTE_FILES_PER_RUN+1),"has_more":false}),
                            json!({"status":"ok","result":{},"has_more":false}),
                        ] {
                            server.reset().await;
                            Mock::given(method("GET"))
                                .and(path("/api/v1/fs/ls"))
                                .and(wiremock::matchers::query_param("uri", protocol.current_user_memories_uri()))
                                .and(wiremock::matchers::query_param("node_limit", "5000"))
                                .respond_with(ResponseTemplate::new(200).set_body_json(value.clone()))
                                .expect(1)
                                .mount(&server)
                                .await;
                            let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
                            if value["has_more"] == json!(false) && value["result"].as_array().is_some_and(|v| v.len() == MAX_REMOTE_FILES_PER_RUN) {
                                pull_memory_files(&provider, &credentials, protocol, &credentials.endpoint, &client, &mut ledger, &mut outcome).await.unwrap();
                            } else {
                                assert!(sync_operations(&provider, &credentials, protocol, &credentials.endpoint, &client, &mut ledger, &mut outcome).await.is_err());
                            }
                            assert_eq!(serde_json::to_vec(&ledger).unwrap(), before);
                            assert_eq!(outcome.imported_memory_count + outcome.updated_memory_count + outcome.exported_memory_count, 0);
                            assert_eq!(server.received_requests().await.unwrap().len(), 1);
                        }
                    }
                });
        });
    }

    #[test]
    fn session_ids_are_bounded_and_path_safe() {
        let id = export_session_id("user/with spaces", "abcdef0123456789");
        assert_eq!(id, "user-with-spaces-hope-abcdef0123456789");
    }
}
