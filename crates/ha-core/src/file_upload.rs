//! Transport-neutral, resumable file upload leases.
//!
//! Clients stream fixed-size chunks into an opaque UUID lease. Domain services
//! claim completed leases by purpose; no backend path is ever returned to a
//! client.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

pub const FILE_UPLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub const FILE_UPLOAD_TTL_SECS: i64 = 60 * 60;
pub const MAX_PENDING_UPLOAD_LEASES: usize = 256;
pub const MAX_PENDING_UPLOAD_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Serializes directory-wide operations such as quota accounting and cleanup.
/// Chunk/status/complete operations use the per-lease locks below so uploads
/// for different files can make progress concurrently.
static UPLOAD_DIRECTORY_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static UPLOAD_LEASE_LOCKS: Lazy<Mutex<HashMap<uuid::Uuid, Weak<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileUploadPurpose {
    ChatAttachment,
    WorkspaceUpload,
    KnowledgeSource,
    ArtifactSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileUploadState {
    Uploading,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadLease {
    pub upload_id: String,
    pub purpose: FileUploadPurpose,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub received_bytes: u64,
    pub state: FileUploadState,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadStartInput {
    pub purpose: FileUploadPurpose,
    pub file_name: String,
    #[serde(default)]
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeaseMetadata {
    lease: FileUploadLease,
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim_token: Option<String>,
}

/// Opaque reservation proving that one domain operation exclusively owns a
/// completed upload until it either releases or consumes the lease.
#[derive(Debug)]
pub struct FileUploadClaim {
    upload_id: String,
    token: String,
}

fn pending_dir() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("uploads").join("pending"))
}

fn parse_upload_id(upload_id: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(upload_id).context("invalid file upload id")
}

fn metadata_path(upload_id: &str) -> Result<PathBuf> {
    let id = parse_upload_id(upload_id)?;
    Ok(pending_dir()?.join(format!("{id}.json")))
}

fn part_path(upload_id: &str) -> Result<PathBuf> {
    let id = parse_upload_id(upload_id)?;
    Ok(pending_dir()?.join(format!("{id}.part")))
}

fn lease_lock_if_present(upload_id: &str) -> Result<Option<Arc<Mutex<()>>>> {
    let id = parse_upload_id(upload_id)?;
    if !metadata_path(upload_id)?.exists() && !part_path(upload_id)?.exists() {
        return Ok(None);
    }
    let mut locks = UPLOAD_LEASE_LOCKS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&id).and_then(Weak::upgrade) {
        return Ok(Some(lock));
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(id, Arc::downgrade(&lock));
    Ok(Some(lock))
}

fn required_lease_lock(upload_id: &str) -> Result<Arc<Mutex<()>>> {
    lease_lock_if_present(upload_id)?
        .with_context(|| format!("file upload lease not found: {upload_id}"))
}

fn write_metadata(path: &Path, metadata: &LeaseMetadata) -> Result<()> {
    let bytes = serde_json::to_vec(metadata)?;
    crate::platform::write_atomic(path, &bytes)
        .with_context(|| format!("write upload metadata {}", path.display()))
}

fn read_metadata(upload_id: &str) -> Result<LeaseMetadata> {
    let meta_path = metadata_path(upload_id)?;
    let bytes = std::fs::read(&meta_path)
        .with_context(|| format!("file upload lease not found: {upload_id}"))?;
    let mut metadata: LeaseMetadata = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid upload metadata: {upload_id}"))?;
    let actual_len = std::fs::metadata(part_path(upload_id)?)
        .with_context(|| format!("file upload data not found: {upload_id}"))?
        .len();
    if actual_len > metadata.lease.size_bytes {
        bail!("file upload data exceeds declared size");
    }
    metadata.lease.received_bytes = actual_len;
    Ok(metadata)
}

fn ensure_not_expired(lease: &FileUploadLease) -> Result<()> {
    let expires_at = DateTime::parse_from_rfc3339(&lease.expires_at)
        .context("invalid upload expiration")?
        .with_timezone(&Utc);
    if expires_at <= Utc::now() {
        bail!("file upload lease expired");
    }
    Ok(())
}

pub fn purpose_limit_bytes(purpose: FileUploadPurpose) -> u64 {
    let config = crate::config::cached_config();
    match purpose {
        FileUploadPurpose::ChatAttachment => config.filesystem.max_chat_attachment_bytes() as u64,
        FileUploadPurpose::WorkspaceUpload => config.filesystem.max_workspace_upload_bytes(),
        // The generic lease protocol intentionally knows only the purpose,
        // not the source kind. Admit the larger of the two file categories
        // here; knowledge import rechecks the selected kind's exact text or
        // binary limit before doing any durable work.
        FileUploadPurpose::KnowledgeSource => config
            .knowledge_source_limits
            .max_binary_source_bytes()
            .max(config.knowledge_source_limits.max_text_source_bytes()),
        FileUploadPurpose::ArtifactSource => config.filesystem.max_artifact_import_bytes(),
    }
}

pub fn ensure_purpose_size(purpose: FileUploadPurpose, size_bytes: u64) -> Result<()> {
    let max_bytes = purpose_limit_bytes(purpose);
    if size_bytes > max_bytes {
        bail!(
            "file exceeds the configured {} MiB limit for {}",
            max_bytes / 1024 / 1024,
            match purpose {
                FileUploadPurpose::ChatAttachment => "chat attachments",
                FileUploadPurpose::WorkspaceUpload => "workspace uploads",
                FileUploadPurpose::KnowledgeSource => "knowledge sources",
                FileUploadPurpose::ArtifactSource => "Artifact sources",
            }
        );
    }
    Ok(())
}

fn pending_usage(dir: &Path) -> Result<(usize, u64)> {
    let mut count = 0usize;
    let mut declared_bytes = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_slice::<LeaseMetadata>(&bytes) else {
            continue;
        };
        count = count.saturating_add(1);
        declared_bytes = declared_bytes.saturating_add(metadata.lease.size_bytes);
    }
    Ok((count, declared_bytes))
}

pub fn start_upload(input: FileUploadStartInput) -> Result<FileUploadLease> {
    let _guard = UPLOAD_DIRECTORY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cleanup_expired_locked()?;
    ensure_purpose_size(input.purpose, input.size_bytes)?;
    let file_name = input.file_name.trim();
    if file_name.is_empty() || file_name.chars().count() > 512 {
        bail!("file name must contain between 1 and 512 characters");
    }
    if input.mime_type.chars().count() > 256 {
        bail!("MIME type is too long");
    }
    let dir = pending_dir()?;
    std::fs::create_dir_all(&dir)?;
    let (count, pending_bytes) = pending_usage(&dir)?;
    if count >= MAX_PENDING_UPLOAD_LEASES {
        bail!("too many pending file uploads");
    }
    if pending_bytes.saturating_add(input.size_bytes) > MAX_PENDING_UPLOAD_BYTES {
        bail!("pending file upload quota exceeded");
    }

    let upload_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let lease = FileUploadLease {
        upload_id: upload_id.clone(),
        purpose: input.purpose,
        file_name: file_name.to_string(),
        mime_type: input.mime_type,
        size_bytes: input.size_bytes,
        received_bytes: 0,
        state: FileUploadState::Uploading,
        expires_at: (now + ChronoDuration::seconds(FILE_UPLOAD_TTL_SECS)).to_rfc3339(),
        content_hash: None,
    };
    let part = part_path(&upload_id)?;
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&part)
        .with_context(|| format!("create upload data {}", part.display()))?;
    let metadata = LeaseMetadata {
        lease: lease.clone(),
        created_at: now.to_rfc3339(),
        claim_token: None,
    };
    if let Err(error) = write_metadata(&metadata_path(&upload_id)?, &metadata) {
        let _ = std::fs::remove_file(part);
        return Err(error);
    }
    Ok(lease)
}

pub fn upload_status(upload_id: &str) -> Result<FileUploadLease> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let metadata = read_metadata(upload_id)?;
    ensure_not_expired(&metadata.lease)?;
    Ok(metadata.lease)
}

pub fn upload_chunk(upload_id: &str, offset: u64, data: &[u8]) -> Result<FileUploadLease> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    if data.len() > FILE_UPLOAD_CHUNK_BYTES {
        bail!("upload chunk exceeds the 4 MiB limit");
    }
    let mut metadata = read_metadata(upload_id)?;
    ensure_not_expired(&metadata.lease)?;
    if metadata.lease.state != FileUploadState::Uploading {
        bail!("file upload is already complete");
    }
    if offset != metadata.lease.received_bytes {
        bail!(
            "upload offset mismatch: expected {}, got {}",
            metadata.lease.received_bytes,
            offset
        );
    }
    if data.is_empty() && metadata.lease.size_bytes != 0 {
        bail!("upload chunk cannot be empty");
    }
    if offset.saturating_add(data.len() as u64) > metadata.lease.size_bytes {
        bail!("upload chunk exceeds declared file size");
    }
    let part = part_path(upload_id)?;
    let mut file = OpenOptions::new().append(true).open(&part)?;
    file.write_all(data)?;
    file.sync_data()?;
    metadata.lease.received_bytes = offset + data.len() as u64;
    write_metadata(&metadata_path(upload_id)?, &metadata)?;
    Ok(metadata.lease)
}

pub fn complete_upload(upload_id: &str) -> Result<FileUploadLease> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let mut metadata = read_metadata(upload_id)?;
    ensure_not_expired(&metadata.lease)?;
    ensure_purpose_size(metadata.lease.purpose, metadata.lease.size_bytes)?;
    if metadata.lease.state == FileUploadState::Complete {
        return Ok(metadata.lease);
    }
    if metadata.lease.received_bytes != metadata.lease.size_bytes {
        bail!(
            "file upload is incomplete: received {} of {} bytes",
            metadata.lease.received_bytes,
            metadata.lease.size_bytes
        );
    }
    let mut file = File::open(part_path(upload_id)?)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    metadata.lease.state = FileUploadState::Complete;
    metadata.lease.content_hash = Some(hasher.finalize().to_hex().to_string());
    write_metadata(&metadata_path(upload_id)?, &metadata)?;
    Ok(metadata.lease)
}

fn validate_completed_locked(
    upload_id: &str,
    purpose: FileUploadPurpose,
) -> Result<(LeaseMetadata, PathBuf)> {
    let metadata = read_metadata(upload_id)?;
    ensure_not_expired(&metadata.lease)?;
    if metadata.lease.purpose != purpose {
        bail!("file upload purpose mismatch");
    }
    if metadata.lease.state != FileUploadState::Complete
        || metadata.lease.received_bytes != metadata.lease.size_bytes
    {
        bail!("file upload is not complete");
    }
    if metadata.claim_token.is_some() {
        bail!("file upload is already claimed");
    }
    ensure_purpose_size(purpose, metadata.lease.size_bytes)?;
    Ok((metadata, part_path(upload_id)?))
}

/// Read a completed upload while holding the service lock. The lease is kept
/// for retry; call [`discard_upload`] only after the domain operation succeeds.
pub fn read_completed_upload(
    upload_id: &str,
    purpose: FileUploadPurpose,
) -> Result<(FileUploadLease, Vec<u8>)> {
    read_completed_upload_with_limit(upload_id, purpose, u64::MAX)
}

/// Read a completed upload only when its declared size is within the domain's
/// exact limit. The limit is checked while holding the lease lock and before
/// allocating a buffer for the file contents.
pub fn read_completed_upload_with_limit(
    upload_id: &str,
    purpose: FileUploadPurpose,
    max_bytes: u64,
) -> Result<(FileUploadLease, Vec<u8>)> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let (metadata, path) = validate_completed_locked(upload_id, purpose)?;
    if metadata.lease.size_bytes > max_bytes {
        bail!(
            "file upload exceeds the domain limit: {} bytes (max {} bytes)",
            metadata.lease.size_bytes,
            max_bytes
        );
    }
    let bytes = std::fs::read(path)?;
    Ok((metadata.lease, bytes))
}

/// Copy a completed upload to a caller-owned staging path. The lease is not
/// consumed, which allows multi-file domain claims to remain all-or-nothing.
pub fn copy_completed_upload(
    upload_id: &str,
    purpose: FileUploadPurpose,
    destination: &Path,
) -> Result<FileUploadLease> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let (metadata, source) = validate_completed_locked(upload_id, purpose)?;
    std::fs::copy(&source, destination).with_context(|| {
        format!(
            "copy completed upload {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(metadata.lease)
}

/// Copy a completed upload to a newly-created caller-owned staging path.
/// `create_new` makes a pre-existing file or symlink fail closed instead of
/// following it. The lease is retained for domain-level retry semantics.
pub fn copy_completed_upload_create_new(
    upload_id: &str,
    purpose: FileUploadPurpose,
    destination: &Path,
) -> Result<FileUploadLease> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let (metadata, source) = validate_completed_locked(upload_id, purpose)?;
    copy_completed_to_new_locked(&metadata, &source, destination)?;
    Ok(metadata.lease)
}

fn copy_completed_to_new_locked(
    metadata: &LeaseMetadata,
    source: &Path,
    destination: &Path,
) -> Result<()> {
    let mut source_file = File::open(source)
        .with_context(|| format!("open completed upload {}", source.display()))?;
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .with_context(|| format!("create upload staging file {}", destination.display()))?;

    let copy_result = (|| -> Result<()> {
        let copied = std::io::copy(&mut source_file, &mut destination_file).with_context(|| {
            format!(
                "copy completed upload {} to {}",
                source.display(),
                destination.display()
            )
        })?;
        if copied != metadata.lease.size_bytes {
            bail!(
                "completed upload changed while copying: copied {} bytes, expected {} bytes",
                copied,
                metadata.lease.size_bytes
            );
        }
        destination_file.flush()?;
        destination_file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = copy_result {
        drop(destination_file);
        let _ = std::fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
}

/// Reserve and stage one completed upload for an exclusive domain claim.
/// The persisted token closes the gap between staging and the domain commit:
/// other readers/claimers fail until the owner releases or consumes it.
pub fn claim_completed_upload_create_new(
    upload_id: &str,
    purpose: FileUploadPurpose,
    destination: &Path,
) -> Result<FileUploadClaim> {
    let lock = required_lease_lock(upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let (mut metadata, source) = validate_completed_locked(upload_id, purpose)?;
    let token = uuid::Uuid::new_v4().to_string();
    metadata.claim_token = Some(token.clone());
    write_metadata(&metadata_path(upload_id)?, &metadata)?;
    if let Err(copy_error) = copy_completed_to_new_locked(&metadata, &source, destination) {
        metadata.claim_token = None;
        if let Err(release_error) = write_metadata(&metadata_path(upload_id)?, &metadata) {
            crate::app_warn!(
                "file_upload",
                "claim_release",
                "failed to release upload claim after staging error: {}",
                release_error
            );
        }
        return Err(copy_error);
    }
    Ok(FileUploadClaim {
        upload_id: upload_id.to_string(),
        token,
    })
}

/// Release a failed domain claim so the completed lease can be retried.
pub fn release_upload_claim(claim: &FileUploadClaim) -> Result<()> {
    let lock = required_lease_lock(&claim.upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let mut metadata = read_metadata(&claim.upload_id)?;
    if metadata.claim_token.as_deref() != Some(claim.token.as_str()) {
        bail!("file upload claim token mismatch");
    }
    metadata.claim_token = None;
    write_metadata(&metadata_path(&claim.upload_id)?, &metadata)
}

/// Consume a successful domain claim. Only the current reservation token may
/// remove the lease, so a stale claimant cannot discard another retry.
pub fn consume_upload_claim(claim: &FileUploadClaim) -> Result<()> {
    let lock = required_lease_lock(&claim.upload_id)?;
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    let metadata = read_metadata(&claim.upload_id)?;
    if metadata.claim_token.as_deref() != Some(claim.token.as_str()) {
        bail!("file upload claim token mismatch");
    }
    discard_locked(&claim.upload_id)
}

pub fn discard_upload(upload_id: &str) -> Result<()> {
    let Some(lock) = lease_lock_if_present(upload_id)? else {
        return Ok(());
    };
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    if metadata_path(upload_id)?.exists() && read_metadata(upload_id)?.claim_token.is_some() {
        bail!("file upload is already claimed");
    }
    discard_locked(upload_id)
}

fn discard_locked(upload_id: &str) -> Result<()> {
    parse_upload_id(upload_id)?;
    for path in [metadata_path(upload_id)?, part_path(upload_id)?] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("discard upload {}", path.display()))
            }
        }
    }
    Ok(())
}

pub fn cleanup_expired_uploads() -> Result<usize> {
    let _guard = UPLOAD_DIRECTORY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cleanup_expired_locked()
}

fn cleanup_expired_locked() -> Result<usize> {
    let dir = pending_dir()?;
    std::fs::create_dir_all(&dir)?;
    let mut removed = 0usize;
    let now = Utc::now();
    let mut known_ids = std::collections::HashSet::new();
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Ok(id) = uuid::Uuid::parse_str(stem) else {
            let _ = std::fs::remove_file(path);
            continue;
        };
        known_ids.insert(id.to_string());
        let expired = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LeaseMetadata>(&bytes).ok())
            .and_then(|metadata| DateTime::parse_from_rfc3339(&metadata.lease.expires_at).ok())
            .map(|expires_at| expires_at.with_timezone(&Utc) <= now)
            .unwrap_or(true);
        if expired {
            if let Ok(Some(lock)) = lease_lock_if_present(stem) {
                let _lease_guard = lock.lock().unwrap_or_else(|error| error.into_inner());
                // Recheck after acquiring the lease lock: a concurrent chunk
                // or complete may have refreshed metadata before we got here.
                let still_expired = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<LeaseMetadata>(&bytes).ok())
                    .and_then(|metadata| {
                        DateTime::parse_from_rfc3339(&metadata.lease.expires_at).ok()
                    })
                    .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
                    .unwrap_or(true);
                if still_expired {
                    let _ = discard_locked(stem);
                    removed += 1;
                }
            }
        }
    }
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("part") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if !known_ids.contains(stem) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_upload_root<T>(f: impl FnOnce(&Path) -> T) -> T {
        let root = tempfile::tempdir().expect("temp upload root");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || f(root.path()))
    }

    #[test]
    fn purpose_serialization_is_stable() {
        assert_eq!(
            serde_json::to_string(&FileUploadPurpose::ChatAttachment).unwrap(),
            "\"chat_attachment\""
        );
        assert_eq!(
            serde_json::to_string(&FileUploadPurpose::ArtifactSource).unwrap(),
            "\"artifact_source\""
        );
    }

    #[test]
    fn chunked_upload_recovers_status_and_hashes_content() {
        with_upload_root(|_| {
            let lease = start_upload(FileUploadStartInput {
                purpose: FileUploadPurpose::ChatAttachment,
                file_name: "note.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 11,
            })
            .expect("start");

            let first = upload_chunk(&lease.upload_id, 0, b"hello ").expect("first chunk");
            assert_eq!(first.received_bytes, 6);
            assert_eq!(upload_status(&lease.upload_id).unwrap().received_bytes, 6);
            let resumed = upload_chunk(&lease.upload_id, 6, b"world").expect("resume");
            assert_eq!(resumed.received_bytes, 11);

            let completed = complete_upload(&lease.upload_id).expect("complete");
            assert_eq!(completed.state, FileUploadState::Complete);
            let expected_hash = blake3::hash(b"hello world").to_hex().to_string();
            assert_eq!(
                completed.content_hash.as_deref(),
                Some(expected_hash.as_str())
            );
            let (_, bytes) =
                read_completed_upload(&lease.upload_id, FileUploadPurpose::ChatAttachment)
                    .expect("claim read");
            assert_eq!(bytes, b"hello world");

            discard_upload(&lease.upload_id).expect("discard");
            assert!(upload_status(&lease.upload_id).is_err());
        });
    }

    #[test]
    fn rejects_duplicate_out_of_order_and_purpose_mismatch() {
        with_upload_root(|_| {
            let lease = start_upload(FileUploadStartInput {
                purpose: FileUploadPurpose::WorkspaceUpload,
                file_name: "data.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
                size_bytes: 4,
            })
            .expect("start");
            upload_chunk(&lease.upload_id, 0, b"ab").expect("first chunk");
            assert!(upload_chunk(&lease.upload_id, 0, b"ab").is_err());
            assert!(upload_chunk(&lease.upload_id, 3, b"c").is_err());
            assert!(complete_upload(&lease.upload_id).is_err());
            upload_chunk(&lease.upload_id, 2, b"cd").expect("second chunk");
            complete_upload(&lease.upload_id).expect("complete");
            assert!(
                read_completed_upload(&lease.upload_id, FileUploadPurpose::KnowledgeSource,)
                    .is_err()
            );
        });
    }

    #[test]
    fn exact_domain_limit_is_checked_before_reading_completed_upload() {
        with_upload_root(|_| {
            let lease = start_upload(FileUploadStartInput {
                purpose: FileUploadPurpose::KnowledgeSource,
                file_name: "large.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 4,
            })
            .expect("start");
            upload_chunk(&lease.upload_id, 0, b"text").expect("chunk");
            complete_upload(&lease.upload_id).expect("complete");

            let error = read_completed_upload_with_limit(
                &lease.upload_id,
                FileUploadPurpose::KnowledgeSource,
                3,
            )
            .expect_err("exact domain limit should reject the lease");
            assert!(error.to_string().contains("domain limit"));
            assert!(upload_status(&lease.upload_id).is_ok());
        });
    }

    #[cfg(unix)]
    #[test]
    fn create_new_copy_does_not_follow_existing_symlink() {
        use std::os::unix::fs::symlink;

        with_upload_root(|root| {
            let lease = start_upload(FileUploadStartInput {
                purpose: FileUploadPurpose::WorkspaceUpload,
                file_name: "data.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
                size_bytes: 4,
            })
            .expect("start");
            upload_chunk(&lease.upload_id, 0, b"data").expect("chunk");
            complete_upload(&lease.upload_id).expect("complete");

            let outside = root.join("outside.txt");
            std::fs::write(&outside, b"original").expect("outside file");
            let destination = root.join("workspace-upload.tmp");
            symlink(&outside, &destination).expect("staging symlink");

            copy_completed_upload_create_new(
                &lease.upload_id,
                FileUploadPurpose::WorkspaceUpload,
                &destination,
            )
            .expect_err("pre-existing symlink must fail closed");
            assert_eq!(std::fs::read(&outside).unwrap(), b"original");
            assert!(std::fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink());
        });
    }

    #[test]
    fn exclusive_claim_blocks_replay_and_can_be_released_or_consumed() {
        with_upload_root(|root| {
            let lease = start_upload(FileUploadStartInput {
                purpose: FileUploadPurpose::ArtifactSource,
                file_name: "report.md".to_string(),
                mime_type: "text/markdown".to_string(),
                size_bytes: 6,
            })
            .expect("start");
            upload_chunk(&lease.upload_id, 0, b"report").expect("chunk");
            complete_upload(&lease.upload_id).expect("complete");

            let first_path = root.join("first.md");
            let first = claim_completed_upload_create_new(
                &lease.upload_id,
                FileUploadPurpose::ArtifactSource,
                &first_path,
            )
            .expect("first claim");
            let replay_error = claim_completed_upload_create_new(
                &lease.upload_id,
                FileUploadPurpose::ArtifactSource,
                &root.join("replay.md"),
            )
            .expect_err("concurrent claim must fail");
            assert!(replay_error.to_string().contains("already claimed"));
            assert!(discard_upload(&lease.upload_id).is_err());

            release_upload_claim(&first).expect("release failed domain claim");
            let second = claim_completed_upload_create_new(
                &lease.upload_id,
                FileUploadPurpose::ArtifactSource,
                &root.join("second.md"),
            )
            .expect("retry claim");
            consume_upload_claim(&second).expect("consume successful claim");
            assert!(upload_status(&lease.upload_id).is_err());
        });
    }

    #[test]
    fn cleanup_removes_expired_and_orphaned_upload_files() {
        with_upload_root(|root| {
            let lease = start_upload(FileUploadStartInput {
                purpose: FileUploadPurpose::ChatAttachment,
                file_name: "expired.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 0,
            })
            .expect("start");
            let meta_path = metadata_path(&lease.upload_id).unwrap();
            let mut metadata: LeaseMetadata =
                serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
            metadata.lease.expires_at = (Utc::now() - ChronoDuration::seconds(1)).to_rfc3339();
            write_metadata(&meta_path, &metadata).unwrap();

            let orphan = root
                .join("uploads/pending")
                .join(format!("{}.part", uuid::Uuid::new_v4()));
            std::fs::write(&orphan, b"orphan").unwrap();
            assert_eq!(cleanup_expired_uploads().unwrap(), 1);
            assert!(!meta_path.exists());
            assert!(!orphan.exists());
        });
    }
}
