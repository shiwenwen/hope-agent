//! Attachment helpers shared by Tauri commands and HTTP routes.
//!
//! Writes uploaded bytes to the per-session attachments directory (or a
//! temporary bucket when the session hasn't been created yet) and returns
//! the absolute path so the caller can hand it to the agent/chat engine.

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::agent::Attachment;
use crate::paths;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenMentionAttachment {
    pub target_id: String,
    pub resource_ref: String,
    /// Session-attachment-store basename for durable recovery. Never an
    /// absolute path; incognito snapshots have no durable materialization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_name: Option<String>,
    pub file_name: String,
    pub mime_type: String,
    pub source_bytes: u64,
    pub durable: bool,
    /// Installation-keyed identity of the object actually opened (inode/device
    /// on Unix). This distinguishes path replacement without exposing a host
    /// path or a raw enumerable identifier.
    pub object_identity_fingerprint: String,
    /// Integrity evidence for the protected journal/materialization path. This
    /// value is not returned by public attachment metadata or logged.
    pub content_fingerprint: String,
    #[serde(skip_serializing)]
    pub bytes: std::sync::Arc<[u8]>,
}

impl std::fmt::Debug for FrozenMentionAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrozenMentionAttachment")
            .field("resource_ref", &self.resource_ref)
            .field("mime_type", &self.mime_type)
            .field("source_bytes", &self.source_bytes)
            .field("durable", &self.durable)
            .field("bytes_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Read-only acquisition result for one typed local resource. The chat engine
/// prepares the complete batch before it creates an execution ledger, then
/// records these refs in that ledger before publishing any durable bytes.
#[doc(hidden)]
pub struct PreparedMentionAttachment {
    target_id: String,
    resource_ref: String,
    snapshot_name: Option<String>,
    attachment_index: usize,
    bytes: std::sync::Arc<[u8]>,
    object_identity_fingerprint: String,
    content_fingerprint: String,
}

impl std::fmt::Debug for PreparedMentionAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedMentionAttachment")
            .field("resource_ref", &self.resource_ref)
            .field("attachment_index", &self.attachment_index)
            .field("bytes_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
#[doc(hidden)]
pub struct PreparedTypedResourceMentions {
    candidates: Vec<PreparedMentionAttachment>,
}

impl std::fmt::Debug for PreparedTypedResourceMentions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedTypedResourceMentions")
            .field("candidate_count", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

impl PreparedTypedResourceMentions {
    /// Bind every planned basename to the backend-generated persistence run.
    /// The run UUID becomes the filesystem ownership boundary used by live and
    /// startup crash reconciliation; no client-controlled path participates.
    pub fn bind_persistence_run(&mut self, run_id: &str) -> Result<()> {
        let prefix = typed_resource_run_prefix(run_id)?;
        for candidate in &mut self.candidates {
            // The durable basename is an internal ownership key, not a display
            // name. Keeping user filenames out makes the component fixed-size,
            // portable across NAME_MAX variants, and immune to path syntax.
            candidate.snapshot_name = Some(format!("{prefix}{}", candidate.resource_ref));
        }
        Ok(())
    }

    pub fn durable_snapshot_names(&self) -> Result<Vec<String>> {
        self.candidates
            .iter()
            .map(|candidate| {
                candidate
                    .snapshot_name
                    .clone()
                    .context("durable typed-resource snapshot has no basename")
            })
            .collect()
    }
}

fn typed_resource_run_prefix(run_id: &str) -> Result<String> {
    let owner = uuid::Uuid::parse_str(run_id)
        .context("typed-resource persistence owner is not a UUID")?
        .simple()
        .to_string();
    Ok(format!("context-snapshot-run_{owner}-"))
}

fn typed_resource_snapshot_owner(name: &str) -> Option<String> {
    let suffix = name.strip_prefix("context-snapshot-run_")?;
    let (owner, remainder) = suffix.split_once('-')?;
    if owner.len() != 32
        || remainder.is_empty()
        || !owner.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    uuid::Uuid::parse_str(owner)
        .ok()
        .map(|value| value.to_string())
}

/// Pseudo-session id for pre-session attachments (uploads that predate a
/// chat session). Maps to `~/.hope-agent/attachments/_temp/`.
pub const TEMP_SESSION_ID: &str = "_temp";
pub const PASTED_TEXT_SOURCE: &str = "pasted_text";
pub const MESSAGE_QUOTE_SOURCE: &str = "message_quote";
/// Files staged by the backend-owned IM FIFO. They behave like normal user
/// attachments after the queue row is consumed, but remain safely removable
/// while the row is only a durable draft.
pub const CHANNEL_QUEUE_SOURCE: &str = "channel_queue";
pub const MAX_CHAT_ATTACHMENTS: usize = 64;
pub const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;
/// Static compatibility ceiling for pre-chunked chat uploads and Base64 wire
/// payloads. Only the generic upload-lease protocol can use a configured
/// limit above 20 MiB.
pub const LEGACY_MAX_CHAT_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
/// Hard resident-memory ceiling for the complete typed-resource batch. The
/// projected charge includes the frozen raw bytes, the provider-compatible
/// Base64 string retained on `Attachment`, and one decode buffer used by the
/// existing content adapters. Ordinary path-backed uploads are unaffected.
pub const MAX_TYPED_RESOURCE_TURN_MEMORY_BYTES: usize = 256 * 1024 * 1024;
/// A direct image's Base64 bytes can coexist in the canonical conversation,
/// its provider-normalized round, intermediate API input, provider request,
/// diagnostic/body serialization, and the HTTP request body. Codex's request
/// builder is the worst case at seven copies beyond `Attachment.data`. This
/// deliberately excludes small JSON/object overhead but includes a
/// conservative per-image envelope below.
pub(crate) const TYPED_RESOURCE_PROVIDER_PAYLOAD_COPIES: usize = 7;
const TYPED_RESOURCE_PROVIDER_IMAGE_ENVELOPE_BYTES: usize = 256;
/// A frozen resource may have to fall back to a short provider-visible
/// reference envelope even when no extraction allowance remains. Reserve this
/// projection in the immutable baseline so fail-visible delivery never spends
/// bytes after the 256 MiB ceiling is already exhausted.
const MAX_TYPED_RESOURCE_FILE_NAME_BYTES: usize = 256;
const MAX_TYPED_RESOURCE_MIME_BYTES: usize = 256;
const MAX_TYPED_RESOURCE_CLIENT_PATH_BYTES: usize = 32 * 1024;
// A normal reference envelope renders the escaped filename twice (`name` and
// opaque display `path`). The provider text projection applies the 6x escape
// expansion; its fixed 2 KiB overhead covers the static envelope text.
const TYPED_RESOURCE_REFERENCE_MATERIALIZED_BYTES: usize = MAX_TYPED_RESOURCE_FILE_NAME_BYTES * 2;
/// Batch-global reserve that initial extraction cannot consume. It is enough
/// for a small Base64 page plus its provider copies, without reserving a page
/// per resource or reducing the existing 20 MiB single-image capability.
pub(crate) const TYPED_RESOURCE_CONTINUATION_FLOOR_BYTES: usize = 256 * 1024;
const UPLOAD_LEASE_TTL: Duration = Duration::from_secs(60 * 60);

pub fn max_chat_attachment_mb() -> u32 {
    crate::config::cached_config()
        .filesystem
        .max_chat_attachment_mb()
}

pub fn max_chat_attachment_bytes() -> usize {
    crate::config::cached_config()
        .filesystem
        .max_chat_attachment_bytes()
}

pub fn ensure_chat_attachment_size(size_bytes: usize) -> Result<()> {
    if size_bytes > max_chat_attachment_bytes() {
        anyhow::bail!(
            "attachment exceeds the configured {} MB limit",
            max_chat_attachment_mb()
        );
    }
    Ok(())
}

pub fn legacy_chat_attachment_bytes() -> usize {
    max_chat_attachment_bytes().min(LEGACY_MAX_CHAT_ATTACHMENT_BYTES)
}

pub fn ensure_legacy_chat_attachment_size(size_bytes: usize) -> Result<()> {
    if size_bytes > legacy_chat_attachment_bytes() {
        anyhow::bail!(
            "legacy attachment exceeds the {} MiB compatibility limit",
            legacy_chat_attachment_bytes() / 1024 / 1024
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadLease {
    pub upload_id: String,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

fn pending_upload_dir() -> Result<PathBuf> {
    Ok(paths::root_dir()?.join("attachments").join(TEMP_SESSION_ID))
}

fn pending_upload_path(upload_id: &str) -> Result<PathBuf> {
    let parsed = uuid::Uuid::parse_str(upload_id).context("invalid attachment upload id")?;
    Ok(pending_upload_dir()?.join(format!("lease-{parsed}")))
}

/// Stage an opaque, expiring upload without exposing a backend filesystem path.
pub fn stage_chat_attachment(
    file_name: &str,
    mime_type: &str,
    data: &[u8],
) -> Result<AttachmentUploadLease> {
    ensure_legacy_chat_attachment_size(data.len())?;
    cleanup_expired_chat_attachment_uploads()?;
    let upload_id = uuid::Uuid::new_v4().to_string();
    let path = pending_upload_path(&upload_id)?;
    crate::platform::write_atomic(&path, data)
        .with_context(|| format!("stage attachment {}", path.display()))?;
    Ok(AttachmentUploadLease {
        upload_id,
        name: file_name.to_string(),
        mime_type: mime_type.to_string(),
        size_bytes: data.len() as u64,
    })
}

/// Stage a streamed upload from disk. The HTTP adapter uses this path so a
/// configured large attachment never has to be materialized as one `Vec<u8>`.
pub fn stage_chat_attachment_file(
    file_name: &str,
    mime_type: &str,
    source_path: &Path,
) -> Result<AttachmentUploadLease> {
    let size_bytes = std::fs::metadata(source_path)
        .with_context(|| format!("stat staged upload {}", source_path.display()))?
        .len();
    let size = usize::try_from(size_bytes).context("attachment size exceeds this platform")?;
    ensure_legacy_chat_attachment_size(size)?;
    cleanup_expired_chat_attachment_uploads()?;
    let upload_id = uuid::Uuid::new_v4().to_string();
    let path = pending_upload_path(&upload_id)?;
    let copied = copy_file_atomic_create_new(source_path, &path)?;
    if let Err(error) = usize::try_from(copied)
        .context("attachment size exceeds this platform")
        .and_then(ensure_legacy_chat_attachment_size)
    {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(AttachmentUploadLease {
        upload_id,
        name: file_name.to_string(),
        mime_type: mime_type.to_string(),
        size_bytes: copied,
    })
}

pub fn discard_chat_attachment_upload(upload_id: &str) -> Result<()> {
    let path = pending_upload_path(upload_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("discard attachment {}", path.display())),
    }
}

pub fn cleanup_expired_chat_attachment_uploads() -> Result<usize> {
    let dir = pending_upload_dir()?;
    std::fs::create_dir_all(&dir)?;
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("lease-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or_default();
        if age >= UPLOAD_LEASE_TTL && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Kind of media item — drives frontend rendering (image preview vs file card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    File,
}

/// Structured media attachment produced by a tool result.
/// Used by `send_attachment` and future tools that need to ship files with
/// filename + MIME metadata to the frontend. Emitted via the `__MEDIA_ITEMS__`
/// prefix in the tool result string (parallel to the simpler `__MEDIA_URLS__`).
///
/// URL semantics: `url` is the logical reference
/// `/api/attachments/{sessionId}/{filename}` — frontend consumes directly
/// (HTTP relies on its HttpOnly session; Tauri prefers `local_path` via
/// `convertFileSrc`). `local_path` is the absolute
/// path on the server, used by IM channel workers to read bytes and by the
/// Tauri frontend to open/reveal locally. HTTP sinks strip `local_path`
/// from events so it never leaks to web clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    /// Logical URL `/api/attachments/{sessionId}/{filename}`. Frontends resolve
    /// this through the transport layer (Tauri uses `local_path`, HTTP uses a
    /// same-origin session cookie).
    pub url: String,
    /// Absolute server-side path. Present for outbound delivery (IM workers,
    /// Tauri file ops). Stripped before forwarding events over HTTP.
    #[serde(rename = "localPath", default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// Display filename (already sanitized).
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    pub kind: MediaKind,
    /// Optional caption / description shown with the attachment. Used as the
    /// IM caption when a channel API supports one (Telegram/WhatsApp/etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

impl MediaItem {
    /// Build a MediaItem for a file that was just persisted by
    /// `save_attachment_bytes`. Handles basename extraction, URL encoding,
    /// and the `_temp` session fallback so every callsite stays consistent.
    pub fn from_saved_path(
        session_id: Option<&str>,
        saved_path: &str,
        display_name: &str,
        mime_type: String,
        size_bytes: u64,
        kind: MediaKind,
        caption: Option<String>,
    ) -> Self {
        let basename = Path::new(saved_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(display_name);
        let sid = session_id
            .filter(|s| !s.is_empty())
            .unwrap_or(TEMP_SESSION_ID);
        let url = format!("/api/attachments/{}/{}", sid, urlencoding::encode(basename));
        Self {
            url,
            local_path: Some(saved_path.to_string()),
            name: display_name.to_string(),
            mime_type,
            size_bytes,
            kind,
            caption,
        }
    }
}

/// Save an attachment's raw bytes to disk.
///
/// When `session_id` is `Some(non-empty)`, writes to
/// `~/.hope-agent/attachments/{session_id}/`. Otherwise falls back to a
/// shared temp bucket (`~/.hope-agent/attachments/_temp/`) so the caller
/// can stage files before a session exists.
///
/// The filename is prefixed with a timestamp and UUID to avoid collisions.
/// Returns the absolute path of the written file.
pub fn save_attachment_bytes(
    session_id: Option<&str>,
    file_name: &str,
    data: &[u8],
) -> Result<String> {
    let file_path = attachment_destination(session_id, file_name)?;
    crate::platform::write_atomic_create_new(&file_path, data)
        .with_context(|| format!("write attachment {}", file_path.display()))?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Persist a streamed attachment from disk without buffering it in memory.
pub fn save_attachment_file(
    session_id: Option<&str>,
    file_name: &str,
    source_path: &Path,
) -> Result<String> {
    let size_bytes = std::fs::metadata(source_path)
        .with_context(|| format!("stat attachment upload {}", source_path.display()))?
        .len();
    let size = usize::try_from(size_bytes).context("attachment size exceeds this platform")?;
    ensure_chat_attachment_size(size)?;
    let file_path = attachment_destination(session_id, file_name)?;
    let copied = copy_file_atomic_create_new(source_path, &file_path)?;
    if let Err(error) = usize::try_from(copied)
        .context("attachment size exceeds this platform")
        .and_then(ensure_chat_attachment_size)
    {
        let _ = std::fs::remove_file(&file_path);
        return Err(error);
    }
    Ok(file_path.to_string_lossy().to_string())
}

/// Resolve and freeze every typed file mention before the first provider
/// attempt. `target_ids` are canonical composer bindings relative to the
/// session working directory; arbitrary attachment paths supplied by a client
/// are never accepted as authority.
///
/// Normal sessions copy the exact bytes into the session-owned attachment
/// store and point the model/tool layer at that immutable turn snapshot.
/// Incognito sessions retain the bytes only in the in-memory `data` field, so
/// provider retries see the same image/file payload without creating a durable
/// prompt-context artifact.
#[cfg(test)]
pub fn freeze_typed_file_mentions(
    session_id: &str,
    working_dir: &str,
    target_ids: &[String],
    incognito: bool,
    attachments: &mut [Attachment],
) -> Result<Vec<FrozenMentionAttachment>> {
    let sources = resolve_typed_file_sources(working_dir, target_ids)?;
    freeze_resolved_mention_sources(session_id, &sources, incognito, attachments)
}

struct ResolvedMentionSource {
    target_id: String,
    containment_root: PathBuf,
    relative_source: PathBuf,
    source: PathBuf,
    attachment_source: &'static str,
}

fn resolve_typed_file_sources(
    working_dir: &str,
    target_ids: &[String],
) -> Result<Vec<ResolvedMentionSource>> {
    use std::collections::HashSet;
    use std::path::Component;

    let root = Path::new(working_dir)
        .canonicalize()
        .with_context(|| format!("canonicalize session working directory {working_dir}"))?;
    if !root.is_dir() {
        anyhow::bail!("session working directory is not a directory");
    }

    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for target_id in target_ids {
        if !seen.insert(target_id.clone()) {
            continue;
        }
        let relative = Path::new(target_id);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            anyhow::bail!("typed file mention is not a safe relative path: {target_id}");
        }

        let source = root
            .join(relative)
            .canonicalize()
            .with_context(|| format!("resolve typed file mention {target_id}"))?;
        if !source.starts_with(&root) {
            anyhow::bail!("typed file mention escapes the session working directory");
        }
        let relative_source = source
            .strip_prefix(&root)
            .context("typed file mention is not relative to its authorized root")?
            .to_path_buf();
        sources.push(ResolvedMentionSource {
            target_id: target_id.clone(),
            containment_root: root.clone(),
            relative_source,
            source,
            attachment_source: "mention",
        });
    }
    Ok(sources)
}

/// Resolve registry-owned `@plan` bindings and freeze them through the same
/// open-once snapshot path as workspace files. The client-supplied absolute
/// attachment path is only a correlation hint: the backend independently
/// resolves the short id/version and requires an exact canonical match.
#[cfg(test)]
pub fn freeze_typed_plan_mentions(
    session_id: &str,
    target_ids: &[String],
    incognito: bool,
    attachments: &mut [Attachment],
) -> Result<Vec<FrozenMentionAttachment>> {
    let sources = resolve_typed_plan_sources(target_ids)?;
    freeze_resolved_mention_sources(session_id, &sources, incognito, attachments)
}

fn resolve_typed_plan_sources(target_ids: &[String]) -> Result<Vec<ResolvedMentionSource>> {
    use std::collections::HashSet;

    if target_ids.is_empty() {
        return Ok(Vec::new());
    }
    let containment_root = crate::paths::plans_dir()?
        .canonicalize()
        .context("canonicalize typed plan registry root")?;
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for target_id in target_ids {
        if !seen.insert(target_id.clone()) {
            continue;
        }
        let (short_id, version_text) = target_id
            .split_once(":v")
            .with_context(|| format!("typed plan mention has invalid target: {target_id}"))?;
        if !(4..=16).contains(&short_id.len())
            || !short_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || version_text.is_empty()
            || !version_text.bytes().all(|byte| byte.is_ascii_digit())
        {
            anyhow::bail!("typed plan mention has invalid target: {target_id}");
        }
        let version = version_text
            .parse::<u32>()
            .with_context(|| format!("typed plan version is out of range: {target_id}"))?;
        let (_, _, source, resolved_version) =
            crate::plan::resolve_plan_mention_path(short_id, version)?;
        if resolved_version != version {
            anyhow::bail!("typed plan mention resolved to an unexpected version");
        }
        let source = source
            .canonicalize()
            .with_context(|| format!("resolve typed plan mention {target_id}"))?;
        if !source.starts_with(&containment_root) {
            anyhow::bail!("typed plan mention escapes the plan registry root");
        }
        let relative_source = source
            .strip_prefix(&containment_root)
            .context("typed plan mention is not relative to its registry root")?
            .to_path_buf();
        sources.push(ResolvedMentionSource {
            target_id: target_id.clone(),
            containment_root: containment_root.clone(),
            relative_source,
            source,
            attachment_source: "plan_mention",
        });
    }
    Ok(sources)
}

/// Freeze all explicit local resource bindings as one atomic batch. This is
/// the chat-turn entrypoint: mixed `@file` + `@plan` turns either publish every
/// immutable snapshot or leave the original attachment array untouched.
#[cfg(test)]
pub fn freeze_typed_resource_mentions(
    session_id: &str,
    working_dir: Option<&str>,
    file_target_ids: &[String],
    plan_target_ids: &[String],
    incognito: bool,
    attachments: &mut [Attachment],
) -> Result<Vec<FrozenMentionAttachment>> {
    let mut prepared = prepare_typed_resource_mentions(
        working_dir,
        file_target_ids,
        plan_target_ids,
        incognito,
        attachments,
    )?;
    if !incognito {
        prepared.bind_persistence_run(&uuid::Uuid::new_v4().to_string())?;
    }
    publish_typed_resource_mentions(session_id, prepared, incognito, attachments)
}

/// Resolve, authorize, open, and read typed resources without publishing any
/// durable artifact. This phase deliberately runs before the persistent chat
/// stream is registered so deterministic local validation failures remain
/// ledger-free, while successful bytes stay memory-only until the ledger is
/// ready to record their materialization refs.
#[doc(hidden)]
pub fn prepare_typed_resource_mentions(
    working_dir: Option<&str>,
    file_target_ids: &[String],
    plan_target_ids: &[String],
    incognito: bool,
    attachments: &[Attachment],
) -> Result<PreparedTypedResourceMentions> {
    let mut sources = if file_target_ids.is_empty() {
        Vec::new()
    } else {
        resolve_typed_file_sources(
            working_dir.context("typed file mentions require a session working directory")?,
            file_target_ids,
        )?
    };
    sources.extend(resolve_typed_plan_sources(plan_target_ids)?);
    prepare_resolved_mention_sources(&sources, incognito, attachments)
}

fn prepare_resolved_mention_sources(
    sources: &[ResolvedMentionSource],
    _incognito: bool,
    attachments: &[Attachment],
) -> Result<PreparedTypedResourceMentions> {
    prepare_resolved_mention_sources_with_budget(
        sources,
        attachments,
        typed_resource_acquisition_budget_bytes()?,
    )
}

fn base64_encoded_len(raw_bytes: usize) -> Option<usize> {
    raw_bytes.checked_add(2)?.checked_div(3)?.checked_mul(4)
}

pub(crate) fn typed_provider_image_payload_bytes(
    encoded_bytes: usize,
    mime_len: usize,
) -> Option<usize> {
    encoded_bytes
        .checked_add(mime_len.checked_mul(6)?)?
        .checked_add(TYPED_RESOURCE_PROVIDER_IMAGE_ENVELOPE_BYTES)
}

pub(crate) fn typed_provider_text_resident_bytes(materialized_utf8_bytes: usize) -> Option<usize> {
    // JSON control-character escaping can expand one input byte to six. Ten
    // copies cover extraction/envelope, canonical/provider histories, request
    // values, diagnostic serialization, and the HTTP body.
    materialized_utf8_bytes
        .checked_mul(6)?
        .checked_add(2_048)?
        .checked_mul(10)
}

fn ensure_typed_resource_metadata(file_name: &str, mime_type: &str) -> Result<()> {
    if file_name.is_empty()
        || file_name.len() > MAX_TYPED_RESOURCE_FILE_NAME_BYTES
        || file_name.chars().any(char::is_control)
    {
        anyhow::bail!("typed resource filename is empty, too long, or contains control characters");
    }
    if mime_type.is_empty()
        || mime_type.len() > MAX_TYPED_RESOURCE_MIME_BYTES
        || mime_type.chars().any(char::is_control)
    {
        anyhow::bail!(
            "typed resource MIME type is empty, too long, or contains control characters"
        );
    }
    Ok(())
}

fn ensure_typed_resource_acquisition_shape(attachment: &Attachment) -> Result<()> {
    ensure_typed_resource_metadata(&attachment.name, &attachment.mime_type)?;
    if attachment.data.is_some() {
        anyhow::bail!(
            "typed resource mention must not carry client-supplied inline data before freezing"
        );
    }
    if attachment.upload_id.is_some()
        || attachment.quote_lines.is_some()
        || attachment.quote_revealable.is_some()
        || attachment.quote_project_root.is_some()
        || attachment.quote_worktree_root.is_some()
        || attachment.quote_role.is_some()
    {
        anyhow::bail!("typed resource mention contains incompatible attachment metadata");
    }
    let path = attachment
        .file_path
        .as_deref()
        .context("typed resource mention has no client path binding")?;
    if path.is_empty()
        || path.len() > MAX_TYPED_RESOURCE_CLIENT_PATH_BYTES
        || path.chars().any(char::is_control)
    {
        anyhow::bail!("typed resource path is empty, too long, or contains control characters");
    }
    Ok(())
}

/// Validate the transport boundary between a canonical typed-mention sidecar
/// and the attachment array that accompanies it. File and Plan attachments do
/// not carry a client-controlled target id, so the shell first proves that the
/// message-bound sidecar has exactly one attachment for every unique target;
/// canonical path matching remains the freeze phase's independent authority.
///
/// Call this before persisting attachment data or queue rows. Repeated mentions
/// of the same target intentionally share one frozen attachment.
pub fn validate_typed_resource_attachment_bindings(
    message: &str,
    incoming_turn: Option<&crate::prompt_context::IncomingTurnWire>,
    attachments: &[Attachment],
) -> Result<()> {
    crate::prompt_context::validate_incoming_turn(message, incoming_turn)?;

    let mut file_targets = HashSet::new();
    let mut plan_targets = HashSet::new();
    if let Some(wire) = incoming_turn {
        for mention in &wire.mentions {
            match mention.kind {
                crate::prompt_context::MentionKind::File => {
                    file_targets.insert(mention.target_id.as_str());
                }
                crate::prompt_context::MentionKind::Plan => {
                    plan_targets.insert(mention.target_id.as_str());
                }
                _ => {}
            }
        }
    }

    let mut file_attachments = 0usize;
    let mut plan_attachments = 0usize;
    for attachment in attachments {
        match attachment.source.as_deref() {
            Some("mention") => {
                ensure_typed_resource_acquisition_shape(attachment)?;
                file_attachments = file_attachments.saturating_add(1);
            }
            Some("plan_mention") => {
                ensure_typed_resource_acquisition_shape(attachment)?;
                plan_attachments = plan_attachments.saturating_add(1);
            }
            _ => {}
        }
    }

    if file_attachments != file_targets.len() || plan_attachments != plan_targets.len() {
        anyhow::bail!(
            "typed resource attachments do not exactly match the unique File/Plan sidecar targets"
        );
    }
    Ok(())
}

fn typed_resource_acquisition_budget_bytes() -> Result<usize> {
    MAX_TYPED_RESOURCE_TURN_MEMORY_BYTES
        .checked_sub(TYPED_RESOURCE_CONTINUATION_FLOOR_BYTES)
        .context("typed resource continuation floor exceeds its hard memory ceiling")
}

fn standard_base64_decoded_len(encoded: &str) -> Option<usize> {
    if encoded.len() % 4 != 0 {
        return None;
    }
    let padding = encoded
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    if padding > 2 {
        return None;
    }
    encoded
        .len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TypedResourceMemoryProjection {
    raw_batch_bytes: usize,
    base64_batch_bytes: usize,
    provider_image_payload_bytes: usize,
    reference_text_bytes: usize,
    max_transient_bytes: usize,
}

impl TypedResourceMemoryProjection {
    fn with_candidate(
        self,
        raw_bytes: usize,
        provider_image_mime_len: Option<usize>,
    ) -> Option<Self> {
        let encoded_bytes = base64_encoded_len(raw_bytes)?;
        let provider_image_payload = match provider_image_mime_len {
            Some(mime_len) => typed_provider_image_payload_bytes(encoded_bytes, mime_len)?,
            None => 0,
        };
        let reference_text =
            typed_provider_text_resident_bytes(TYPED_RESOURCE_REFERENCE_MATERIALIZED_BYTES)?;
        Some(Self {
            raw_batch_bytes: self.raw_batch_bytes.checked_add(raw_bytes)?,
            base64_batch_bytes: self.base64_batch_bytes.checked_add(encoded_bytes)?,
            provider_image_payload_bytes: self
                .provider_image_payload_bytes
                .checked_add(provider_image_payload)?,
            reference_text_bytes: self.reference_text_bytes.checked_add(reference_text)?,
            // Acquisition compacts one Vec into Arc storage at a time and
            // non-image materialization decodes one Base64 item at a time.
            max_transient_bytes: self.max_transient_bytes.max(raw_bytes),
        })
    }

    fn resident_bytes(self) -> Option<usize> {
        self.raw_batch_bytes
            .checked_add(self.base64_batch_bytes)?
            .checked_add(
                self.provider_image_payload_bytes
                    .checked_mul(TYPED_RESOURCE_PROVIDER_PAYLOAD_COPIES)?,
            )?
            .checked_add(self.reference_text_bytes)?
            .checked_add(self.max_transient_bytes)
    }
}

/// Remaining bulk-allocation allowance available to bounded extraction after
/// the frozen typed-resource baseline is retained. `None` means this turn has
/// no typed resources. The provider materializer recomputes the projection
/// from the exact standard-Base64 strings so extraction cannot silently use a
/// second, disconnected budget.
pub(crate) fn typed_resource_extraction_budget_bytes(
    attachments: &[Attachment],
) -> Result<Option<usize>> {
    let mut found = false;
    let mut projection = TypedResourceMemoryProjection::default();
    for attachment in attachments.iter().filter(|attachment| {
        matches!(
            attachment.source.as_deref(),
            Some("mention" | "plan_mention")
        )
    }) {
        found = true;
        ensure_typed_resource_metadata(&attachment.name, &attachment.mime_type)?;
        let encoded = attachment
            .data
            .as_deref()
            .context("frozen typed resource has no retained Base64 bytes")?;
        let raw_bytes = standard_base64_decoded_len(encoded)
            .context("frozen typed resource has malformed standard Base64 length")?;
        let image_mime_len = attachment
            .mime_type
            .starts_with("image/")
            .then_some(attachment.mime_type.len());
        projection = projection
            .with_candidate(raw_bytes, image_mime_len)
            .context("typed resource extraction budget overflow")?;
    }
    if !found {
        return Ok(None);
    }
    let baseline = projection
        .resident_bytes()
        .context("typed resource extraction budget overflow")?;
    let remaining = MAX_TYPED_RESOURCE_TURN_MEMORY_BYTES
        .checked_sub(baseline)
        .context("typed resource baseline exceeds its hard memory ceiling")?;
    Ok(Some(remaining))
}

/// Rebuild the same turn-wide frozen-resource baseline from scoped raw refs
/// when `read_context_resource` executes in a later tool round. Accounting all
/// refs (rather than only the selected handle) prevents repeated continuation
/// reads from treating the 256 MiB ceiling as a fresh per-resource allowance.
pub(crate) fn context_resource_extraction_budget_bytes(
    resources: &[crate::prompt_context::ContextResourceRef],
) -> Result<usize> {
    let mut projection = TypedResourceMemoryProjection::default();
    for resource in resources {
        ensure_typed_resource_metadata(&resource.file_name, &resource.mime_type)?;
        let image_mime_len = resource
            .mime_type
            .starts_with("image/")
            .then_some(resource.mime_type.len());
        projection = projection
            .with_candidate(resource.bytes.len(), image_mime_len)
            .context("context resource extraction budget overflow")?;
    }
    let baseline = projection
        .resident_bytes()
        .context("context resource extraction budget overflow")?;
    MAX_TYPED_RESOURCE_TURN_MEMORY_BYTES
        .checked_sub(baseline)
        .context("context resource baseline exceeds its hard memory ceiling")
}

fn max_raw_bytes_for_projected_budget(
    projection: TypedResourceMemoryProjection,
    memory_budget: usize,
    provider_image_mime_len: Option<usize>,
) -> usize {
    let mut low = 0usize;
    let mut high = memory_budget;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if projection
            .with_candidate(mid, provider_image_mime_len)
            .and_then(TypedResourceMemoryProjection::resident_bytes)
            .is_some_and(|value| value <= memory_budget)
        {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

fn compact_frozen_bytes(bytes: Vec<u8>, declared_size: usize) -> Result<std::sync::Arc<[u8]>> {
    if bytes.len() != declared_size {
        anyhow::bail!("typed resource mention changed size while it was being frozen");
    }
    // `into_boxed_slice` drops any spare Vec capacity before the bytes become
    // retained turn state. Projection can therefore charge the exact slice
    // length instead of trusting a stale pre-read stat as allocation size.
    Ok(std::sync::Arc::from(bytes.into_boxed_slice()))
}

fn read_exact_declared_bytes(reader: &mut impl Read, declared_size: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; declared_size];
    reader
        .read_exact(&mut bytes)
        .context("typed resource mention changed size while it was being frozen")?;
    let mut probe = [0u8; 1];
    loop {
        match reader.read(&mut probe) {
            Ok(0) => break,
            Ok(_) => anyhow::bail!("typed resource mention changed size while it was being frozen"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("probe typed resource mention EOF"),
        }
    }
    Ok(bytes)
}

fn prepare_resolved_mention_sources_with_budget(
    sources: &[ResolvedMentionSource],
    attachments: &[Attachment],
    memory_budget: usize,
) -> Result<PreparedTypedResourceMentions> {
    use std::collections::HashSet;

    for attachment in attachments.iter().filter(|attachment| {
        matches!(
            attachment.source.as_deref(),
            Some("mention" | "plan_mention")
        )
    }) {
        ensure_typed_resource_acquisition_shape(attachment)?;
    }

    // Phase A is read-only: validate and acquire the complete set before
    // mutating an attachment or publishing a durable snapshot. A failure in
    // one mention therefore cannot leave a partially usable turn context.
    let mut seen_sources = HashSet::new();
    let mut candidates = Vec::new();
    let mut memory_projection = TypedResourceMemoryProjection::default();
    for resolved in sources {
        let target_id = &resolved.target_id;
        let source = &resolved.source;
        if !seen_sources.insert(source.clone()) {
            anyhow::bail!(
                "multiple typed resource targets resolve to the same source object: {target_id}"
            );
        }
        let attachment_index = attachments
            .iter()
            .position(|attachment| {
                if attachment.source.as_deref() != Some(resolved.attachment_source) {
                    return false;
                }
                attachment
                    .file_path
                    .as_deref()
                    .and_then(|path| Path::new(path).canonicalize().ok())
                    .is_some_and(|path| path == *source)
            })
            .with_context(|| {
                format!("typed resource mention has no matching attachment: {target_id}")
            })?;
        let provider_image_mime_len = attachments[attachment_index]
            .mime_type
            .starts_with("image/")
            .then_some(attachments[attachment_index].mime_type.len());
        ensure_typed_resource_acquisition_shape(&attachments[attachment_index])?;

        // Open exactly once through the authorized root namespace. Unix walks
        // every component with openat + O_NOFOLLOW from a root descriptor;
        // Windows holds a no-delete-share direct-directory handle chain from
        // the drive/UNC root, validates every final path, and rejects reparse
        // points. Pathname swaps after the resolver can therefore only fail
        // closed, never redirect this handle.
        let mut source_file = crate::platform::open_file_beneath(
            &resolved.containment_root,
            &resolved.relative_source,
        )
        .with_context(|| format!("open typed resource mention {target_id}"))?;
        let metadata = source_file
            .metadata()
            .with_context(|| format!("stat opened typed resource mention {target_id}"))?;
        if !metadata.is_file() {
            anyhow::bail!("typed resource mention source is not a regular file");
        }
        let declared_size = usize::try_from(metadata.len())
            .context("typed resource mention size exceeds this platform")?;
        ensure_chat_attachment_size(declared_size)?;
        let max_raw_bytes = max_raw_bytes_for_projected_budget(
            memory_projection,
            memory_budget,
            provider_image_mime_len,
        )
        .min(max_chat_attachment_bytes());
        if declared_size > max_raw_bytes {
            anyhow::bail!(
                "typed resource batch exceeds the {} MiB turn memory budget",
                memory_budget / 1024 / 1024
            );
        }
        let bytes = read_exact_declared_bytes(&mut source_file, declared_size)
            .with_context(|| format!("read typed resource mention {target_id}"))?;
        ensure_chat_attachment_size(bytes.len())?;
        let bytes = compact_frozen_bytes(bytes, declared_size)?;
        memory_projection = memory_projection
            .with_candidate(bytes.len(), provider_image_mime_len)
            .context("typed resource batch memory accounting overflow")?;
        let projected_memory_bytes = memory_projection
            .resident_bytes()
            .context("typed resource batch memory accounting overflow")?;
        if bytes.len() > max_raw_bytes || projected_memory_bytes > memory_budget {
            anyhow::bail!(
                "typed resource batch exceeds the {} MiB turn memory budget",
                memory_budget / 1024 / 1024
            );
        }
        let content_fingerprint =
            crate::cache_routing::audit_fingerprint("typed-file-snapshot", &bytes);
        let resource_ref = format!("resource_ref_{}", uuid::Uuid::new_v4().simple());
        candidates.push(PreparedMentionAttachment {
            target_id: target_id.clone(),
            resource_ref,
            snapshot_name: None,
            attachment_index,
            bytes,
            object_identity_fingerprint: opened_file_identity_fingerprint(&source, &metadata),
            content_fingerprint,
        });
    }
    Ok(PreparedTypedResourceMentions { candidates })
}

/// File-publication half of a prepared typed-resource batch. For durable
/// sessions the caller must run this synchronously inside the SessionDB
/// publication gate; attachment mutation and Base64 encoding happen only
/// after that gate commits so they do not extend the SQLite writer lock.
#[doc(hidden)]
pub struct PublishedTypedResourceMentions {
    candidates: Vec<PreparedMentionAttachment>,
    snapshot_paths: Vec<PathBuf>,
    incognito: bool,
}

#[doc(hidden)]
pub fn publish_typed_resource_snapshot_files(
    session_id: &str,
    prepared: PreparedTypedResourceMentions,
    incognito: bool,
) -> Result<PublishedTypedResourceMentions> {
    let PreparedTypedResourceMentions { candidates } = prepared;

    // Phase B prepares all durable copies before switching the attachment
    // array to them. Best-effort cleanup makes a failed batch invisible to the
    // execution path and avoids accumulating ordinary half-snapshots.
    let mut snapshot_paths = Vec::new();
    if !incognito {
        let attachment_dir = paths::attachments_dir(session_id)?;
        std::fs::create_dir_all(&attachment_dir).with_context(|| {
            format!(
                "create typed-resource snapshot directory {}",
                attachment_dir.display()
            )
        })?;
        for candidate in &candidates {
            let snapshot_name = candidate
                .snapshot_name
                .as_deref()
                .context("durable typed-resource snapshot has no basename")?;
            let path = attachment_dir.join(snapshot_name);
            match crate::platform::write_atomic_create_new(&path, &candidate.bytes) {
                Ok(()) => snapshot_paths.push(path),
                Err(error) => {
                    for published_path in &snapshot_paths {
                        let Some(published_name) =
                            published_path.file_name().and_then(|v| v.to_str())
                        else {
                            continue;
                        };
                        if let Ok((root, relative)) =
                            typed_snapshot_cleanup_path(session_id, published_name)
                        {
                            let _ = crate::platform::remove_file_beneath(&root, &relative);
                        }
                    }
                    return Err(error)
                        .with_context(|| format!("publish typed resource {}", path.display()));
                }
            }
        }
    }

    Ok(PublishedTypedResourceMentions {
        candidates,
        snapshot_paths,
        incognito,
    })
}

/// Finish a successfully published batch after the DB publication gate has
/// committed. This phase is deterministic/infallible for a prepared batch and
/// may perform the large Base64 allocations without blocking unrelated DB
/// writers.
#[doc(hidden)]
pub fn finalize_typed_resource_mentions(
    published: PublishedTypedResourceMentions,
    attachments: &mut [Attachment],
) -> Vec<FrozenMentionAttachment> {
    let PublishedTypedResourceMentions {
        candidates,
        snapshot_paths,
        incognito,
    } = published;

    let mut frozen = Vec::with_capacity(candidates.len());
    for (ordinal, candidate) in candidates.into_iter().enumerate() {
        let attachment = &mut attachments[candidate.attachment_index];
        let snapshot_name = candidate.snapshot_name;
        if incognito {
            attachment.data =
                Some(base64::engine::general_purpose::STANDARD.encode(&candidate.bytes));
        } else {
            attachment.file_path = Some(snapshot_paths[ordinal].to_string_lossy().into_owned());
            // Every provider/profile attempt consumes the same in-memory
            // bytes. The durable snapshot is recovery evidence, not a mutable
            // path that the hot path reopens between retries.
            attachment.data =
                Some(base64::engine::general_purpose::STANDARD.encode(&candidate.bytes));
        }
        frozen.push(FrozenMentionAttachment {
            target_id: candidate.target_id,
            resource_ref: candidate.resource_ref,
            snapshot_name,
            file_name: attachment.name.clone(),
            mime_type: attachment.mime_type.clone(),
            source_bytes: candidate.bytes.len() as u64,
            durable: !incognito,
            object_identity_fingerprint: candidate.object_identity_fingerprint,
            content_fingerprint: candidate.content_fingerprint,
            bytes: candidate.bytes,
        });
    }
    frozen
}

/// Publish a fully prepared batch. Production durable turns wrap the
/// file-publication half in SessionDB's writer gate; this convenience wrapper
/// remains useful for focused attachment tests, including incognito flow.
#[cfg(test)]
pub(crate) fn publish_typed_resource_mentions(
    session_id: &str,
    prepared: PreparedTypedResourceMentions,
    incognito: bool,
    attachments: &mut [Attachment],
) -> Result<Vec<FrozenMentionAttachment>> {
    let published = publish_typed_resource_snapshot_files(session_id, prepared, incognito)?;
    Ok(finalize_typed_resource_mentions(published, attachments))
}

#[cfg(test)]
fn freeze_resolved_mention_sources(
    session_id: &str,
    sources: &[ResolvedMentionSource],
    incognito: bool,
    attachments: &mut [Attachment],
) -> Result<Vec<FrozenMentionAttachment>> {
    let mut prepared = prepare_resolved_mention_sources(sources, incognito, attachments)?;
    if !incognito {
        prepared.bind_persistence_run(&uuid::Uuid::new_v4().to_string())?;
    }
    publish_typed_resource_mentions(session_id, prepared, incognito, attachments)
}

/// Best-effort rollback for a batch whose Initial Context reference never
/// became durable. Basename validation keeps cleanup scoped to artifacts that
/// this typed-resource publisher owns.
#[doc(hidden)]
pub fn remove_uncommitted_typed_resource_snapshots(session_id: &str, snapshot_names: &[String]) {
    for snapshot_name in snapshot_names {
        let path = Path::new(snapshot_name);
        let owned =
            path.components().count() == 1 && snapshot_name.starts_with("context-snapshot-run_");
        if owned {
            if let Ok((root, relative)) = typed_snapshot_cleanup_path(session_id, snapshot_name) {
                let _ = crate::platform::remove_file_beneath(&root, &relative);
            }
        }
    }
}

fn typed_snapshot_cleanup_path(
    session_id: &str,
    snapshot_name: &str,
) -> Result<(PathBuf, PathBuf)> {
    use std::path::Component;

    let session = Path::new(session_id);
    if session.components().count() != 1
        || !matches!(session.components().next(), Some(Component::Normal(_)))
    {
        anyhow::bail!("typed-resource cleanup session id is not one path component");
    }
    let data_root = paths::root_dir()?
        .canonicalize()
        .context("canonicalize typed-resource cleanup data root")?;
    Ok((
        data_root,
        PathBuf::from("attachments")
            .join(session)
            .join(snapshot_name),
    ))
}

/// Remove one exact ledger-owned snapshot after its stream run was deleted.
/// Missing files acknowledge cleanly so a crash between unlink and DB ack is
/// idempotent. Any malformed or cross-owner row fails closed and remains in the
/// ledger for inspection instead of influencing an arbitrary path.
pub(crate) fn remove_pending_typed_resource_snapshot(
    cleanup: &crate::session::TypedResourceSnapshotCleanup,
) -> Result<()> {
    let prefix = typed_resource_run_prefix(&cleanup.run_id)?;
    let path = Path::new(&cleanup.snapshot_name);
    if path.components().count() != 1
        || !cleanup.snapshot_name.starts_with(&prefix)
        || typed_resource_snapshot_owner(&cleanup.snapshot_name).as_deref()
            != Some(cleanup.run_id.as_str())
    {
        anyhow::bail!("typed-resource cleanup row is not owned by its run");
    }
    let (root, relative) =
        typed_snapshot_cleanup_path(&cleanup.session_id, &cleanup.snapshot_name)?;
    match crate::platform::remove_file_beneath(&root, &relative) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "remove expired typed-resource snapshot {}",
                cleanup.snapshot_name
            )
        }),
    }
}

/// Reconcile every snapshot owned by one backend-generated persistence run.
/// Only exact, single-component names under that run's UUID prefix are ever
/// considered; journal paths outside this namespace cannot influence cleanup.
pub(crate) fn reconcile_run_typed_resource_snapshots(
    session_id: &str,
    run_id: &str,
    referenced_snapshot_names: &HashSet<String>,
) -> Result<usize> {
    let prefix = typed_resource_run_prefix(run_id)?;
    for snapshot_name in referenced_snapshot_names {
        if !snapshot_name.starts_with(&prefix) || Path::new(snapshot_name).components().count() != 1
        {
            anyhow::bail!("typed-resource journal snapshot is not owned by this run");
        }
    }

    let root = paths::attachments_dir(session_id)?;
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("enumerate typed-resource snapshots {}", root.display()))
        }
    };
    let mut removed = 0usize;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("read typed-resource snapshot entry in {}", root.display()))?;
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if !name.starts_with(&prefix) || referenced_snapshot_names.contains(&name) {
            continue;
        }
        let (cleanup_root, cleanup_relative) = typed_snapshot_cleanup_path(session_id, &name)?;
        crate::platform::remove_file_beneath(&cleanup_root, &cleanup_relative)
            .with_context(|| format!("remove uncommitted typed-resource snapshot {name}"))?;
        removed += 1;
    }
    Ok(removed)
}

/// Enumerate exact `(session_id, persistence_run_id)` owners represented by
/// run-scoped snapshot basenames. Startup uses this namespace to reconcile a
/// terminal run if a hard crash prevented its normal Drop cleanup.
pub(crate) fn run_owned_typed_resource_snapshot_owners() -> Result<Vec<(String, String)>> {
    let root = paths::root_dir()?.join("attachments");
    let session_entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("enumerate attachment sessions {}", root.display()))
        }
    };
    let mut owners = HashSet::new();
    for session_entry in session_entries {
        let session_entry = session_entry
            .with_context(|| format!("read attachment session entry in {}", root.display()))?;
        if !session_entry
            .file_type()
            .with_context(|| format!("stat attachment session {}", session_entry.path().display()))?
            .is_dir()
        {
            continue;
        }
        let Some(session_id) = session_entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        for snapshot_entry in std::fs::read_dir(session_entry.path()).with_context(|| {
            format!(
                "enumerate session attachment snapshots {}",
                session_entry.path().display()
            )
        })? {
            let snapshot_entry = snapshot_entry.with_context(|| {
                format!(
                    "read session attachment snapshot in {}",
                    session_entry.path().display()
                )
            })?;
            let Some(name) = snapshot_entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if let Some(run_id) = typed_resource_snapshot_owner(&name) {
                owners.insert((session_id.clone(), run_id));
            }
        }
    }
    let mut owners = owners.into_iter().collect::<Vec<_>>();
    owners.sort();
    Ok(owners)
}

fn opened_file_identity_fingerprint(_path: &Path, metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let identity = format!(
            "unix:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mode()
        );
        crate::cache_routing::audit_fingerprint("typed-file-object", identity.as_bytes())
    }
    #[cfg(not(unix))]
    {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let identity = format!(
            "portable:{}:{}:{}",
            _path.to_string_lossy(),
            metadata.len(),
            modified
        );
        crate::cache_routing::audit_fingerprint("typed-file-object", identity.as_bytes())
    }
}

fn attachment_destination(session_id: Option<&str>, file_name: &str) -> Result<PathBuf> {
    let att_dir: PathBuf = match session_id {
        Some(sid) if !sid.is_empty() => paths::attachments_dir(sid)?,
        _ => paths::root_dir()?.join("attachments").join(TEMP_SESSION_ID),
    };
    std::fs::create_dir_all(&att_dir)
        .with_context(|| format!("create attachments dir {}", att_dir.display()))?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name = file_name.replace(['/', '\\', ':'], "_");
    Ok(att_dir.join(format!("{}_{}_{}", ts, uuid::Uuid::new_v4(), safe_name)))
}

fn copy_file_atomic_create_new(source_path: &Path, destination: &Path) -> Result<u64> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("attachment destination has no parent"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create attachment directory {}", parent.display()))?;
    let mut source = std::fs::File::open(source_path)
        .with_context(|| format!("open staged upload {}", source_path.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create attachment temp in {}", parent.display()))?;
    let copied = std::io::copy(&mut source, &mut temp)
        .with_context(|| format!("copy staged upload {}", source_path.display()))?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(destination).map_err(|error| {
        anyhow::Error::new(error.error)
            .context(format!("publish attachment {}", destination.display()))
    })?;
    Ok(copied)
}

/// Persist chat input attachments into the session attachment directory and
/// return the JSON payload stored in `messages.attachments_meta`.
///
/// Images may arrive as base64 `data`; file attachments usually arrive as
/// `file_path` pointing either at the session directory or the shared `_temp`
/// bucket. The function updates each `Attachment.file_path` to the final path
/// so the chat engine reads the same persisted bytes that the UI can recover
/// from history.
pub fn persist_chat_user_attachments_meta(
    session_id: &str,
    attachments: &mut [Attachment],
) -> Result<Option<String>> {
    let max_bytes = max_chat_attachment_bytes();
    let max_mb = max_chat_attachment_mb();
    let legacy_max_bytes = legacy_chat_attachment_bytes();
    if attachments.len() > MAX_CHAT_ATTACHMENTS {
        anyhow::bail!("a message can contain at most {MAX_CHAT_ATTACHMENTS} attachments");
    }
    if attachments.is_empty() {
        return Ok(None);
    }
    for attachment in attachments.iter() {
        if attachment.upload_id.is_some() {
            if attachment.data.is_some() || attachment.file_path.is_some() {
                anyhow::bail!("upload_id is mutually exclusive with data and file_path");
            }
            if !matches!(
                attachment.source.as_deref(),
                Some("upload") | Some(PASTED_TEXT_SOURCE)
            ) {
                anyhow::bail!("upload_id is only valid for uploaded attachments");
            }
        }
        if matches!(
            attachment.source.as_deref(),
            Some("upload") | Some(PASTED_TEXT_SOURCE)
        ) {
            if let Some(data) = attachment.data.as_deref() {
                let encoded_limit = legacy_max_bytes.saturating_mul(4) / 3 + 8;
                let decoded_too_large = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map(|decoded| decoded.len() > legacy_max_bytes)
                    .unwrap_or(false);
                if data.len() > encoded_limit || decoded_too_large {
                    anyhow::bail!("attachment exceeds the legacy chat upload limit");
                }
            }
        }
    }

    let att_dir = paths::attachments_dir(session_id)?;
    std::fs::create_dir_all(&att_dir)
        .with_context(|| format!("create attachments dir {}", att_dir.display()))?;
    let temp_dir = paths::root_dir()?.join("attachments").join(TEMP_SESSION_ID);
    std::fs::create_dir_all(&temp_dir)
        .with_context(|| format!("create temp attachments dir {}", temp_dir.display()))?;
    let canonical_att_dir = att_dir
        .canonicalize()
        .with_context(|| format!("canonicalize attachments dir {}", att_dir.display()))?;
    let canonical_temp_dir = temp_dir
        .canonicalize()
        .with_context(|| format!("canonicalize temp attachments dir {}", temp_dir.display()))?;

    // Prepare every lease before deleting any source. Copying into the session
    // directory is rollback-safe: a failure removes all prepared destinations,
    // leaving every original lease available for retry/discard.
    let mut prepared_leases: Vec<(usize, String, Option<PathBuf>, PathBuf, bool)> = Vec::new();
    let prepare_result = (|| -> Result<()> {
        for (index, att) in attachments.iter().enumerate() {
            let Some(upload_id) = att.upload_id.as_deref() else {
                continue;
            };
            if att.data.is_some() || att.file_path.is_some() {
                anyhow::bail!("upload_id is mutually exclusive with data and file_path");
            }
            let safe_name = att.name.replace(['/', '\\', ':'], "_");
            let destination = att_dir.join(format!("{upload_id}_{safe_name}"));
            match crate::file_upload::copy_completed_upload_create_new(
                upload_id,
                crate::file_upload::FileUploadPurpose::ChatAttachment,
                &destination,
            ) {
                Ok(lease) => {
                    if lease.size_bytes > max_bytes as u64 {
                        let _ = std::fs::remove_file(&destination);
                        anyhow::bail!("attachment exceeds the configured {max_mb} MB limit");
                    }
                    prepared_leases.push((index, upload_id.to_string(), None, destination, true));
                }
                Err(generic_error) => {
                    // Compatibility with clients using the pre-chunked staging endpoint.
                    let source = pending_upload_path(upload_id)?;
                    let canonical_source = source.canonicalize().with_context(|| {
                        format!("attachment upload lease not found: {upload_id} ({generic_error})")
                    })?;
                    let metadata = std::fs::metadata(&canonical_source)?;
                    if !canonical_source.starts_with(&canonical_temp_dir)
                        || !metadata.is_file()
                        || metadata.len() > legacy_max_bytes as u64
                    {
                        anyhow::bail!("invalid attachment upload lease: {upload_id}");
                    }
                    copy_file_atomic_create_new(&canonical_source, &destination).with_context(
                        || {
                            format!(
                                "claim attachment upload {} to {}",
                                canonical_source.display(),
                                destination.display()
                            )
                        },
                    )?;
                    prepared_leases.push((
                        index,
                        upload_id.to_string(),
                        Some(canonical_source),
                        destination,
                        false,
                    ));
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = prepare_result {
        for (_, _, _, destination, _) in &prepared_leases {
            let _ = std::fs::remove_file(destination);
        }
        return Err(error);
    }
    for (index, upload_id, source, destination, generic) in prepared_leases {
        attachments[index].file_path = Some(destination.to_string_lossy().to_string());
        attachments[index].upload_id = None;
        if generic {
            let _ = crate::file_upload::discard_upload(&upload_id);
        } else if let Some(source) = source {
            let _ = std::fs::remove_file(source);
        }
    }

    let mut meta_list = Vec::new();
    for att in attachments.iter_mut() {
        let source = att.source.clone();
        let source_ref = source.as_deref();
        // File-browser quotes carry no bytes — persist them as structured quote
        // objects so history can render a friendly reference card (the model
        // already saw a `<file_reference>` via content.rs).
        if source_ref == Some("quote") {
            let history_path = quote_history_path(att);
            meta_list.push(json!({
                "kind": "quote",
                "name": att.name,
                "path": history_path,
                "lines": att.quote_lines,
                "content": att.data,
                "revealable": att.quote_revealable,
                "project_root": att.quote_project_root,
                "worktree_root": att.quote_worktree_root,
            }));
            continue;
        }
        // Conversation excerpts are inline context, not files. Persist their
        // role + exact selected text so history can restore the quote card.
        if source_ref == Some(MESSAGE_QUOTE_SOURCE) {
            meta_list.push(json!({
                "kind": MESSAGE_QUOTE_SOURCE,
                "role": att.quote_role,
                "content": att.data,
            }));
            continue;
        }
        if !is_user_upload_source(source_ref) {
            continue;
        }
        if let Some(ref b64_data) = att.data {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64_data)
                .unwrap_or_default();
            if let Some(ref fp) = att.file_path {
                let src_path = Path::new(fp);
                match resolve_persisted_user_attachment_path(
                    src_path,
                    &canonical_temp_dir,
                    &canonical_att_dir,
                    &att_dir,
                ) {
                    Ok(final_path) => {
                        let canonical_final_path =
                            final_path.canonicalize().with_context(|| {
                                format!("canonicalize attachment {}", final_path.display())
                            })?;
                        if canonical_final_path.starts_with(&canonical_att_dir) {
                            att.file_path =
                                Some(canonical_final_path.to_string_lossy().to_string());
                            let size = std::fs::metadata(&canonical_final_path)
                                .map(|m| m.len())
                                .unwrap_or(decoded.len() as u64);
                            meta_list.push(user_attachment_meta(
                                att,
                                size,
                                &canonical_final_path,
                                source_ref,
                            ));
                            continue;
                        }
                    }
                    Err(err) => {
                        let failure = crate::cache_routing::audit_fingerprint(
                            "attachment-persist",
                            err.to_string().as_bytes(),
                        );
                        app_warn!(
                            "app",
                            "chat",
                            "Falling back to attachment bytes after persistence failure ({})",
                            &failure[..16]
                        );
                    }
                }
            }
            let path = match save_bytes_in_dir(&att_dir, &att.name, &decoded)
                .with_context(|| format!("save image attachment {}", att.name))
            {
                Ok(path) => path,
                Err(err) => {
                    let failure = crate::cache_routing::audit_fingerprint(
                        "attachment-persist",
                        err.to_string().as_bytes(),
                    );
                    app_warn!(
                        "app",
                        "chat",
                        "Skipping one attachment after persistence failure ({})",
                        &failure[..16]
                    );
                    continue;
                }
            };
            att.file_path = Some(path.to_string_lossy().to_string());
            meta_list.push(user_attachment_meta(
                att,
                decoded.len() as u64,
                &path,
                source_ref,
            ));
            continue;
        }

        let Some(ref fp) = att.file_path else {
            continue;
        };
        let src_path = Path::new(fp);
        let final_path = match resolve_persisted_user_attachment_path(
            src_path,
            &canonical_temp_dir,
            &canonical_att_dir,
            &att_dir,
        ) {
            Ok(path) => path,
            Err(err) => {
                let failure = crate::cache_routing::audit_fingerprint(
                    "attachment-persist",
                    err.to_string().as_bytes(),
                );
                app_warn!(
                    "app",
                    "chat",
                    "Skipping one attachment after path validation failure ({})",
                    &failure[..16]
                );
                continue;
            }
        };
        let canonical_final_path = match final_path
            .canonicalize()
            .with_context(|| format!("canonicalize attachment {}", final_path.display()))
        {
            Ok(path) => path,
            Err(err) => {
                let failure = crate::cache_routing::audit_fingerprint(
                    "attachment-persist",
                    err.to_string().as_bytes(),
                );
                app_warn!(
                    "app",
                    "chat",
                    "Skipping one attachment after canonicalization failure ({})",
                    &failure[..16]
                );
                continue;
            }
        };
        if !canonical_final_path.starts_with(&canonical_att_dir) {
            app_warn!(
                "app",
                "chat",
                "attachment path outside allowed attachment directories"
            );
            continue;
        }

        att.file_path = Some(canonical_final_path.to_string_lossy().to_string());
        att.upload_id = None;
        let size = std::fs::metadata(&canonical_final_path)
            .map(|m| m.len())
            .unwrap_or(0);
        meta_list.push(user_attachment_meta(
            att,
            size,
            &canonical_final_path,
            source_ref,
        ));
    }

    if meta_list.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::to_string(&meta_list)?))
    }
}

fn quote_history_path(attachment: &Attachment) -> Option<String> {
    let path = attachment.file_path.as_deref()?;
    let root = attachment.quote_worktree_root.as_deref().or_else(|| {
        attachment
            .quote_project_root
            .as_ref()
            .map(|root| root.path.as_str())
    });
    let Some(root) = root else {
        return Some(path.to_string());
    };
    let Ok(relative) = Path::new(path).strip_prefix(Path::new(root)) else {
        return Some(path.to_string());
    };
    if relative.as_os_str().is_empty() {
        return Some(path.to_string());
    }
    Some(relative.to_string_lossy().to_string())
}

/// Move queue attachments into the session-owned attachment directory before
/// serializing the queue row. Uploaded image bytes are cleared after a durable
/// `file_path` is established so the queue DB never balloons with base64 data;
/// quotes retain their inline excerpt and mention attachments remain references.
pub fn persist_queued_chat_attachments(
    session_id: &str,
    request_id: &str,
    attachments: &mut [Attachment],
) -> Result<()> {
    // This helper can also be reached by non-chat transports. Even if a shell
    // accidentally omits the sidecar association check, never persist an
    // unbounded/preloaded payload that claims typed-resource provenance.
    for attachment in attachments.iter().filter(|attachment| {
        matches!(
            attachment.source.as_deref(),
            Some("mention" | "plan_mention")
        )
    }) {
        ensure_typed_resource_acquisition_shape(attachment)?;
    }

    // A text-only queued message has no attachment directory to prepare. The
    // generic persistence helper intentionally returns before creating one for
    // an empty slice, so avoid canonicalizing a path that does not exist yet.
    if attachments.is_empty() {
        return Ok(());
    }
    let _ = persist_chat_user_attachments_meta(session_id, attachments)?;
    let attachment_root = paths::attachments_dir(session_id)?;
    let canonical_root = attachment_root.canonicalize()?;
    let safe_request_id: String = request_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let queue_prefix = format!("queue_{safe_request_id}_");
    for attachment in attachments {
        if attachment.file_path.is_some()
            && matches!(
                attachment.source.as_deref(),
                Some("upload") | Some(PASTED_TEXT_SOURCE) | Some(CHANNEL_QUEUE_SOURCE)
            )
        {
            if let Some(path) = attachment.file_path.as_deref().map(PathBuf::from) {
                let canonical_path = path.canonicalize()?;
                if canonical_path.starts_with(&canonical_root) {
                    let basename = canonical_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("attachment");
                    if !basename.starts_with(&queue_prefix) {
                        let queued_path = attachment_root
                            .join(format!("{queue_prefix}{}_{basename}", uuid::Uuid::new_v4()));
                        match std::fs::rename(&canonical_path, &queued_path) {
                            Ok(()) => {}
                            Err(_) => {
                                std::fs::copy(&canonical_path, &queued_path)?;
                                std::fs::remove_file(&canonical_path)?;
                            }
                        }
                        attachment.file_path = Some(queued_path.to_string_lossy().to_string());
                    }
                }
            }
            attachment.data = None;
        }
    }
    Ok(())
}

/// Remove files owned exclusively by a discarded durable queue row. The
/// request-id filename prefix makes this fail closed: mention/quote paths and
/// files belonging to another row are never touched.
pub fn remove_discarded_queued_attachments(
    session_id: &str,
    request_id: &str,
    attachments: &[Attachment],
) {
    let Ok(root) = paths::attachments_dir(session_id) else {
        return;
    };
    let Ok(canonical_root) = root.canonicalize() else {
        return;
    };
    let safe_request_id: String = request_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let queue_prefix = format!("queue_{safe_request_id}_");
    for attachment in attachments {
        if !matches!(
            attachment.source.as_deref(),
            Some("upload") | Some(PASTED_TEXT_SOURCE) | Some(CHANNEL_QUEUE_SOURCE)
        ) {
            continue;
        }
        let Some(path) = attachment.file_path.as_deref().map(PathBuf::from) else {
            continue;
        };
        let Ok(canonical_path) = path.canonicalize() else {
            continue;
        };
        let owned = canonical_path.starts_with(&canonical_root)
            && canonical_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&queue_prefix));
        if owned {
            let _ = std::fs::remove_file(canonical_path);
        }
    }
}

/// Copy durable attachment files referenced by a message into a forked
/// session and rewrite the known attachment metadata shapes to point at the
/// new session. Workspace quote references and unknown metadata are left
/// untouched because they are references, not session-owned bytes.
pub(crate) fn fork_attachments_meta(
    source_session_id: &str,
    forked_session_id: &str,
    raw_meta: &str,
) -> Result<String> {
    let Ok(mut meta) = serde_json::from_str::<Value>(raw_meta) else {
        return Ok(raw_meta.to_string());
    };
    let mut changed = false;

    match &mut meta {
        Value::Array(items) => {
            changed |= rewrite_user_attachment_items(items, source_session_id, forked_session_id)?;
        }
        Value::Object(object) => {
            if let Some(items) = object
                .get_mut("user_attachments")
                .and_then(Value::as_array_mut)
            {
                changed |=
                    rewrite_user_attachment_items(items, source_session_id, forked_session_id)?;
            }
            if let Some(items) = object
                .get_mut(crate::session::ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS)
                .and_then(Value::as_array_mut)
            {
                changed |= rewrite_tool_media_items(items, source_session_id, forked_session_id)?;
            }
        }
        _ => {}
    }

    if changed {
        Ok(serde_json::to_string(&meta)?)
    } else {
        Ok(raw_meta.to_string())
    }
}

fn rewrite_user_attachment_items(
    items: &mut [Value],
    source_session_id: &str,
    forked_session_id: &str,
) -> Result<bool> {
    let mut changed = false;
    for item in items {
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        if matches!(
            object.get("kind").and_then(Value::as_str),
            Some("quote") | Some(MESSAGE_QUOTE_SOURCE)
        ) {
            continue;
        }
        let Some(source_path) = object.get("path").and_then(Value::as_str) else {
            continue;
        };
        if let Some(copied_path) =
            copy_session_attachment(source_path, source_session_id, forked_session_id)?
        {
            object.insert(
                "path".to_string(),
                Value::String(copied_path.to_string_lossy().to_string()),
            );
            changed = true;
        }
    }
    Ok(changed)
}

fn rewrite_tool_media_items(
    items: &mut [Value],
    source_session_id: &str,
    forked_session_id: &str,
) -> Result<bool> {
    let source_url_prefix = format!("/api/attachments/{source_session_id}/");
    let mut changed = false;

    for item in items {
        let Some(object) = item.as_object_mut() else {
            continue;
        };

        let mut copied_path = None;
        if let Some(local_path) = object.get("localPath").and_then(Value::as_str) {
            copied_path =
                copy_session_attachment(local_path, source_session_id, forked_session_id)?;
        }

        if copied_path.is_none() {
            if let Some(encoded_name) = object
                .get("url")
                .and_then(Value::as_str)
                .and_then(|url| url.strip_prefix(&source_url_prefix))
            {
                let decoded_name = urlencoding::decode(encoded_name)
                    .with_context(|| format!("decode attachment URL {encoded_name}"))?;
                if decoded_name.contains(['/', '\\']) {
                    anyhow::bail!("invalid attachment URL filename: {encoded_name}");
                }
                let source_path = paths::attachments_dir(source_session_id)?.join(&*decoded_name);
                copied_path = copy_session_attachment(
                    &source_path.to_string_lossy(),
                    source_session_id,
                    forked_session_id,
                )?;
            }
        }

        let Some(copied_path) = copied_path else {
            continue;
        };
        let file_name = copied_path
            .file_name()
            .and_then(|value| value.to_str())
            .context("forked attachment filename is not valid UTF-8")?;
        object.insert(
            "localPath".to_string(),
            Value::String(copied_path.to_string_lossy().to_string()),
        );
        object.insert(
            "url".to_string(),
            Value::String(format!(
                "/api/attachments/{}/{}",
                forked_session_id,
                urlencoding::encode(file_name)
            )),
        );
        changed = true;
    }

    Ok(changed)
}

fn copy_session_attachment(
    raw_path: &str,
    source_session_id: &str,
    forked_session_id: &str,
) -> Result<Option<PathBuf>> {
    let source_dir = paths::attachments_dir(source_session_id)?;
    let source_path = PathBuf::from(raw_path);
    let lexically_owned = source_path.starts_with(&source_dir);
    let canonical_source_dir = match source_dir.canonicalize() {
        Ok(path) => path,
        Err(_) if !lexically_owned => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("canonicalize attachments dir {}", source_dir.display()));
        }
    };
    let canonical_source_path = match source_path.canonicalize() {
        Ok(path) => path,
        Err(_) if !lexically_owned => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("canonicalize attachment {}", source_path.display()));
        }
    };
    if !canonical_source_path.starts_with(&canonical_source_dir) {
        if lexically_owned {
            anyhow::bail!(
                "attachment escapes source session directory: {}",
                source_path.display()
            );
        }
        return Ok(None);
    }
    if !canonical_source_path.is_file() {
        anyhow::bail!("attachment path is not a file: {}", source_path.display());
    }

    let file_name = canonical_source_path
        .file_name()
        .context("source attachment has no filename")?;
    let forked_dir = paths::attachments_dir(forked_session_id)?;
    std::fs::create_dir_all(&forked_dir)
        .with_context(|| format!("create attachments dir {}", forked_dir.display()))?;
    let forked_path = forked_dir.join(file_name);
    std::fs::copy(&canonical_source_path, &forked_path).with_context(|| {
        format!(
            "copy attachment {} to {}",
            canonical_source_path.display(),
            forked_path.display()
        )
    })?;
    Ok(Some(forked_path))
}

fn is_user_upload_source(source: Option<&str>) -> bool {
    matches!(
        source,
        None | Some("upload") | Some(PASTED_TEXT_SOURCE) | Some(CHANNEL_QUEUE_SOURCE)
    )
}

fn user_attachment_meta(att: &Attachment, size: u64, path: &Path, source: Option<&str>) -> Value {
    let mut meta = json!({
        "name": &att.name,
        "mime_type": &att.mime_type,
        "size": size,
        "path": path.to_string_lossy(),
    });
    if let (Some(source), Some(obj)) = (source, meta.as_object_mut()) {
        obj.insert("source".to_string(), json!(source));
    }
    meta
}

fn save_bytes_in_dir(att_dir: &Path, file_name: &str, data: &[u8]) -> Result<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name = file_name.replace(['/', '\\', ':'], "_");
    let file_path = att_dir.join(format!("{}_{}", ts, safe_name));
    std::fs::write(&file_path, data)
        .with_context(|| format!("write attachment {}", file_path.display()))?;
    Ok(file_path)
}

fn move_temp_attachment(src_path: &Path, att_dir: &Path) -> Result<PathBuf> {
    let Some(fname) = src_path.file_name() else {
        return Ok(src_path.to_path_buf());
    };
    let dest = att_dir.join(fname);
    match std::fs::rename(src_path, &dest) {
        Ok(()) => Ok(dest),
        Err(rename_err) => {
            std::fs::copy(src_path, &dest).with_context(|| {
                format!(
                    "move attachment {} to {} after rename failed: {}",
                    src_path.display(),
                    dest.display(),
                    rename_err
                )
            })?;
            let _ = std::fs::remove_file(src_path);
            Ok(dest)
        }
    }
}

fn resolve_persisted_user_attachment_path(
    src_path: &Path,
    canonical_temp_dir: &Path,
    canonical_att_dir: &Path,
    att_dir: &Path,
) -> Result<PathBuf> {
    let canonical_src = src_path
        .canonicalize()
        .with_context(|| format!("canonicalize attachment {}", src_path.display()))?;
    let metadata = std::fs::metadata(&canonical_src)
        .with_context(|| format!("stat attachment {}", canonical_src.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("attachment path is not a file: {}", src_path.display());
    }

    if canonical_src.starts_with(canonical_temp_dir) {
        return move_temp_attachment(&canonical_src, att_dir);
    }
    if canonical_src.starts_with(canonical_att_dir) {
        return Ok(canonical_src);
    }

    anyhow::bail!(
        "attachment path outside allowed attachment directories: {}",
        src_path.display()
    );
}

// ── MIME Sniffing ───────────────────────────────────────────────

/// Sniff a MIME type: try magic bytes first, then extension, then fall back
/// to `application/octet-stream`. Shared between `send_attachment` and the
/// HTTP `/api/attachments/...` download route.
pub fn sniff_mime(data: &[u8], path: &Path) -> String {
    if let Some(m) = sniff_mime_magic(data) {
        return m.to_string();
    }
    if let Some(ext) = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
    {
        if let Some(m) = mime_from_extension(&ext) {
            return m.to_string();
        }
    }
    "application/octet-stream".to_string()
}

/// Match a prefix of the file against well-known magic bytes. Returns `None`
/// when no known signature matches.
pub fn sniff_mime_magic(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if data.len() >= 3 && &data[..3] == b"\xFF\xD8\xFF" {
        return Some("image/jpeg");
    }
    if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if data.len() >= 2 && &data[..2] == b"BM" {
        return Some("image/bmp");
    }
    if data.len() >= 4 && &data[..4] == b"%PDF" {
        return Some("application/pdf");
    }
    // ZIP family (also docx / xlsx / pptx / odt). Callers can drill down if
    // they need to distinguish Office from plain zip; `application/zip` is a
    // reasonable default for generic display.
    if data.len() >= 4 && &data[..4] == b"PK\x03\x04" {
        return Some("application/zip");
    }
    if data.len() >= 2 && &data[..2] == b"\x1F\x8B" {
        return Some("application/gzip");
    }
    if data.len() >= 6 && &data[..6] == b"7z\xBC\xAF\x27\x1C" {
        return Some("application/x-7z-compressed");
    }
    if data.len() >= 7 && &data[..7] == b"Rar!\x1A\x07\x01" {
        return Some("application/vnd.rar");
    }
    // MP4 / QuickTime (ftyp box at offset 4).
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        return Some("video/mp4");
    }
    None
}

/// Map a lowercase file extension to a best-guess MIME type.
pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "js" | "mjs" => "application/javascript",
        "ts" | "tsx" => "text/typescript",
        "py" => "text/x-python",
        "rs" => "text/rust",
        "go" => "text/x-go",
        "sh" | "bash" | "zsh" => "application/x-sh",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Attachment;

    #[test]
    fn typed_resource_debug_omits_bytes_paths_names_and_fingerprints() {
        const SENTINEL: &str = "PRIVATE-ATTACHMENT-SENTINEL";
        let frozen = FrozenMentionAttachment {
            target_id: format!("private/{SENTINEL}/target"),
            resource_ref: "resource_ref_safe".into(),
            snapshot_name: Some(format!("snapshot-{SENTINEL}")),
            file_name: format!("{SENTINEL}.txt"),
            mime_type: "text/plain".into(),
            source_bytes: SENTINEL.len() as u64,
            durable: true,
            object_identity_fingerprint: format!("object-{SENTINEL}"),
            content_fingerprint: format!("content-{SENTINEL}"),
            bytes: std::sync::Arc::from(SENTINEL.as_bytes()),
        };
        let prepared = PreparedMentionAttachment {
            target_id: format!("private/{SENTINEL}/prepared"),
            resource_ref: "resource_ref_prepared".into(),
            snapshot_name: Some(format!("prepared-{SENTINEL}")),
            attachment_index: 3,
            bytes: std::sync::Arc::from(SENTINEL.as_bytes()),
            object_identity_fingerprint: format!("object-{SENTINEL}"),
            content_fingerprint: format!("content-{SENTINEL}"),
        };
        let prepared_batch = PreparedTypedResourceMentions {
            candidates: vec![prepared],
        };

        for debug in [format!("{frozen:?}"), format!("{prepared_batch:?}")] {
            assert!(!debug.contains(SENTINEL));
            assert!(!debug.contains("private/"));
        }
    }

    fn assert_session_attachment_path(path: &str, root: &Path, session_id: &str) {
        let path = Path::new(path);
        let expected_dir = root.join("attachments").join(session_id);
        let expected_dir = expected_dir
            .canonicalize()
            .expect("session attachments dir should exist");
        assert!(
            path.starts_with(&expected_dir),
            "expected {} to be inside {}",
            path.display(),
            expected_dir.display()
        );
    }

    fn mention_attachment(path: &Path) -> Attachment {
        Attachment {
            name: "selected.txt".to_string(),
            mime_type: "text/plain".to_string(),
            source: Some("mention".to_string()),
            data: None,
            file_path: Some(path.to_string_lossy().into_owned()),
            upload_id: None,
            quote_lines: None,
            quote_revealable: None,
            quote_role: None,
            quote_project_root: None,
            quote_worktree_root: None,
        }
    }

    fn file_binding_wire(
        message: &str,
        spans: &[(usize, usize)],
    ) -> crate::prompt_context::IncomingTurnWire {
        let digest = crate::prompt_context::canonical_text_digest(message);
        crate::prompt_context::IncomingTurnWire {
            prompt_contract_version: crate::prompt_context::PROMPT_CONTRACT_VERSION,
            mention_wire_version: crate::prompt_context::MENTION_WIRE_VERSION,
            user_input: crate::prompt_context::CanonicalUserInput {
                input_item_id: "input-1".into(),
                canonicalization_version: 1,
                text: message.into(),
                digest: digest.clone(),
            },
            mentions: spans
                .iter()
                .enumerate()
                .map(|(index, (start, end))| crate::prompt_context::MentionBindingWire {
                    id: format!("file-{index}"),
                    kind: crate::prompt_context::MentionKind::File,
                    target_id: "selected.txt".into(),
                    display_label: "selected.txt".into(),
                    origin: crate::prompt_context::StructuredMentionOrigin::FirstPartyComposerGesture,
                    source_anchor: crate::prompt_context::SourceAnchor::Inline {
                        input_item_id: "input-1".into(),
                        canonical_text_digest: digest.clone(),
                        start_utf8: *start as u64,
                        end_utf8: *end as u64,
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn typed_resource_transport_binding_is_exact_before_persistence() {
        let path = Path::new("/tmp/selected.txt");
        let attachment = mention_attachment(path);
        let message = "@selected.txt";
        let wire = file_binding_wire(message, &[(0, message.len())]);
        validate_typed_resource_attachment_bindings(
            message,
            Some(&wire),
            std::slice::from_ref(&attachment),
        )
        .expect("one unique target has one typed attachment");

        assert!(validate_typed_resource_attachment_bindings(message, Some(&wire), &[]).is_err());
        assert!(validate_typed_resource_attachment_bindings(
            "plain",
            None,
            std::slice::from_ref(&attachment),
        )
        .is_err());

        let repeated = "@selected.txt and @selected.txt";
        let first = repeated.find("@selected.txt").unwrap();
        let second = repeated.rfind("@selected.txt").unwrap();
        let repeated_wire = file_binding_wire(
            repeated,
            &[
                (first, first + "@selected.txt".len()),
                (second, second + "@selected.txt".len()),
            ],
        );
        validate_typed_resource_attachment_bindings(repeated, Some(&repeated_wire), &[attachment])
            .expect("repeated mentions of one target share one attachment");
    }

    #[test]
    fn queued_typed_resource_rejects_client_inline_data_before_disk_io() {
        let mut attachment = mention_attachment(Path::new("/tmp/selected.txt"));
        attachment.data = Some("forged-inline-payload".into());
        let error = persist_queued_chat_attachments(
            "does-not-need-to-exist",
            "request",
            std::slice::from_mut(&mut attachment),
        )
        .expect_err("queue persistence must reject typed inline data first");
        assert!(error.to_string().contains("client-supplied inline data"));
    }

    #[test]
    fn typed_resource_memory_projection_charges_batches_and_one_decode_peak() {
        let reference =
            typed_provider_text_resident_bytes(TYPED_RESOURCE_REFERENCE_MATERIALIZED_BYTES)
                .expect("reference charge");
        let projection = TypedResourceMemoryProjection::default()
            .with_candidate(3, None)
            .expect("first candidate")
            .with_candidate(4, None)
            .expect("second candidate");
        assert_eq!(projection.raw_batch_bytes, 7);
        assert_eq!(projection.base64_batch_bytes, 12);
        assert_eq!(projection.provider_image_payload_bytes, 0);
        assert_eq!(projection.reference_text_bytes, reference * 2);
        assert_eq!(projection.max_transient_bytes, 4);
        assert_eq!(projection.resident_bytes(), Some(23 + reference * 2));
        assert_eq!(
            max_raw_bytes_for_projected_budget(
                TypedResourceMemoryProjection::default(),
                reference + 9,
                None,
            ),
            2,
            "three raw bytes exceed nine non-reference resident bytes"
        );
    }

    #[test]
    fn typed_resource_projection_charges_provider_image_copies() {
        let mime_len = "image/png".len();
        let projection = TypedResourceMemoryProjection::default()
            .with_candidate(3, Some(mime_len))
            .expect("image candidate");
        let one_payload = base64_encoded_len(3).unwrap()
            + mime_len * 6
            + TYPED_RESOURCE_PROVIDER_IMAGE_ENVELOPE_BYTES;
        let reference =
            typed_provider_text_resident_bytes(TYPED_RESOURCE_REFERENCE_MATERIALIZED_BYTES)
                .expect("reference charge");
        assert_eq!(
            TYPED_RESOURCE_PROVIDER_PAYLOAD_COPIES, 7,
            "Codex retains seven provider payload copies beyond Attachment.data"
        );
        assert_eq!(projection.provider_image_payload_bytes, one_payload);
        assert_eq!(
            projection.resident_bytes(),
            Some(3 + 4 + one_payload * 7 + reference + 3)
        );

        let max_source = LEGACY_MAX_CHAT_ATTACHMENT_BYTES;
        let max_image = TypedResourceMemoryProjection::default()
            .with_candidate(max_source, Some(mime_len))
            .expect("maximum image projection");
        assert!(
            max_image.resident_bytes().unwrap()
                <= typed_resource_acquisition_budget_bytes().unwrap(),
            "the aggregate guard must preserve the existing single-file source ceiling"
        );
        assert!(
            max_raw_bytes_for_projected_budget(
                max_image,
                MAX_TYPED_RESOURCE_TURN_MEMORY_BYTES,
                Some(mime_len),
            ) < max_source,
            "provider payload copies must constrain a second large image"
        );
    }

    #[test]
    fn thirty_two_resource_admission_preserves_one_global_continuation_floor() {
        let acquisition_budget = typed_resource_acquisition_budget_bytes().unwrap();
        let mut projection = TypedResourceMemoryProjection::default();
        for _ in 0..31 {
            projection = projection
                .with_candidate(1, None)
                .expect("small reference projection");
        }
        let final_raw = max_raw_bytes_for_projected_budget(projection, acquisition_budget, None);
        let admitted = projection
            .with_candidate(final_raw, None)
            .expect("32-resource admitted projection");
        let resident = admitted.resident_bytes().expect("resident projection");
        assert!(resident <= acquisition_budget);
        assert!(
            MAX_TYPED_RESOURCE_TURN_MEMORY_BYTES - resident
                >= TYPED_RESOURCE_CONTINUATION_FLOOR_BYTES
        );
        assert!(
            projection
                .with_candidate(final_raw + 1, None)
                .and_then(TypedResourceMemoryProjection::resident_bytes)
                .is_some_and(|value| value > acquisition_budget),
            "one more source byte must cross the admission threshold"
        );
    }

    #[test]
    fn compact_frozen_bytes_rejects_stat_read_size_change() {
        let mut oversized = Vec::with_capacity(1024);
        oversized.extend_from_slice(b"x");
        assert!(compact_frozen_bytes(oversized, 1024)
            .expect_err("a post-stat truncate must fail closed")
            .to_string()
            .contains("changed size"));

        let mut spare_capacity = Vec::with_capacity(1024);
        spare_capacity.extend_from_slice(b"x");
        let compact = compact_frozen_bytes(spare_capacity, 1).expect("compact exact bytes");
        assert_eq!(&*compact, b"x");

        assert!(read_exact_declared_bytes(&mut std::io::Cursor::new(b"x"), 2).is_err());
        assert!(read_exact_declared_bytes(&mut std::io::Cursor::new(b"xy"), 1).is_err());
        assert_eq!(
            read_exact_declared_bytes(&mut std::io::Cursor::new(b"xy"), 2).unwrap(),
            b"xy"
        );
    }

    #[test]
    fn typed_resource_prepare_enforces_aggregate_memory_before_publication() {
        let root = tempfile::tempdir().expect("tempdir");
        let first = root.path().join("first.txt");
        let second = root.path().join("second.txt");
        std::fs::write(&first, b"four").expect("first");
        std::fs::write(&second, b"more").expect("second");
        let containment_root = root.path().canonicalize().expect("root canonical");
        let sources = vec![
            ResolvedMentionSource {
                target_id: "first.txt".into(),
                containment_root: containment_root.clone(),
                relative_source: PathBuf::from("first.txt"),
                source: first.canonicalize().expect("first canonical"),
                attachment_source: "mention",
            },
            ResolvedMentionSource {
                target_id: "second.txt".into(),
                containment_root,
                relative_source: PathBuf::from("second.txt"),
                source: second.canonicalize().expect("second canonical"),
                attachment_source: "mention",
            },
        ];
        let attachments = vec![mention_attachment(&first), mention_attachment(&second)];

        // raw batch 8 + Base64 batch 16 + one 4-byte decode peak = 28.
        let error = prepare_resolved_mention_sources_with_budget(&sources, &attachments, 27)
            .expect_err("aggregate projection must fail visibly");
        assert!(error.to_string().contains("turn memory budget"));
        assert!(attachments
            .iter()
            .all(|attachment| attachment.data.is_none()));
    }

    #[test]
    fn typed_resource_prepare_rejects_unbounded_or_preloaded_client_metadata() {
        let root = tempfile::tempdir().expect("tempdir");
        let source = root.path().join("selected.txt");
        std::fs::write(&source, b"safe").expect("source");
        let targets = vec!["selected.txt".to_string()];

        let mut too_long = mention_attachment(&source);
        too_long.name = "x".repeat(MAX_TYPED_RESOURCE_FILE_NAME_BYTES + 1);
        let error = prepare_typed_resource_mentions(
            root.path().to_str(),
            &targets,
            &[],
            false,
            &[too_long],
        )
        .expect_err("oversized display name");
        assert!(error.to_string().contains("filename"));

        let mut preloaded = mention_attachment(&source);
        preloaded.data = Some("client-owned-inline-data".into());
        let error = prepare_typed_resource_mentions(
            root.path().to_str(),
            &targets,
            &[],
            false,
            &[preloaded],
        )
        .expect_err("preloaded typed data");
        assert!(error.to_string().contains("client-supplied inline data"));

        let mut long_path = mention_attachment(&source);
        long_path.file_path = Some("p".repeat(MAX_TYPED_RESOURCE_CLIENT_PATH_BYTES + 1));
        let error = prepare_typed_resource_mentions(
            root.path().to_str(),
            &targets,
            &[],
            false,
            &[long_path],
        )
        .expect_err("oversized client path");
        assert!(error.to_string().contains("path"));

        let mut escape_heavy = mention_attachment(&source);
        escape_heavy.name = "&".repeat(MAX_TYPED_RESOURCE_FILE_NAME_BYTES);
        prepare_typed_resource_mentions(
            root.path().to_str(),
            &targets,
            &[],
            false,
            &[escape_heavy],
        )
        .expect("bounded escape-heavy name");
        assert!(
            TYPED_RESOURCE_REFERENCE_MATERIALIZED_BYTES * 6
                >= MAX_TYPED_RESOURCE_FILE_NAME_BYTES * 2 * "&amp;".len(),
            "reference reserve must cover two maximally escaped filename attributes"
        );
    }

    #[test]
    fn typed_file_freeze_is_open_once_scoped_and_recoverable() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let workspace = root.path().join("workspace");
            std::fs::create_dir_all(&workspace).expect("workspace");
            let source = workspace.join("selected.txt");
            std::fs::write(&source, b"frozen bytes").expect("source");

            let mut incognito_attachment = vec![mention_attachment(&source)];
            let incognito = freeze_typed_file_mentions(
                "incognito-session",
                workspace.to_str().unwrap(),
                &["selected.txt".to_string()],
                true,
                &mut incognito_attachment,
            )
            .expect("freeze incognito");
            assert_eq!(&*incognito[0].bytes, b"frozen bytes");
            assert!(!incognito[0].durable);
            assert!(incognito[0].snapshot_name.is_none());
            assert!(!incognito[0].object_identity_fingerprint.is_empty());

            let mut durable_attachment = vec![mention_attachment(&source)];
            let durable = freeze_typed_file_mentions(
                "durable-session",
                workspace.to_str().unwrap(),
                &["selected.txt".to_string()],
                false,
                &mut durable_attachment,
            )
            .expect("freeze durable");
            let snapshot_name = durable[0].snapshot_name.as_deref().expect("snapshot ref");
            assert_eq!(Path::new(snapshot_name).components().count(), 1);
            assert_eq!(
                std::fs::read(
                    root.path()
                        .join("attachments")
                        .join("durable-session")
                        .join(snapshot_name),
                )
                .expect("durable snapshot"),
                b"frozen bytes"
            );
            assert_eq!(&*durable[0].bytes, b"frozen bytes");
            assert!(durable[0].durable);
        });
    }

    #[test]
    fn typed_resource_prepare_is_read_only_and_publish_has_journal_owned_name() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let workspace = root.path().join("workspace");
            std::fs::create_dir_all(&workspace).expect("workspace");
            let source = workspace.join("selected.txt");
            std::fs::write(&source, b"frozen bytes").expect("source");
            let mut attachments = vec![mention_attachment(&source)];
            attachments[0].name = format!("{}.txt", "x".repeat(220));
            assert!(
                !root.path().join("plans").exists(),
                "file-only typed turns must not require a plan registry"
            );

            let mut prepared = prepare_typed_resource_mentions(
                Some(workspace.to_str().unwrap()),
                &["selected.txt".to_string()],
                &[],
                false,
                &attachments,
            )
            .expect("prepare");
            let run_id = "4b9c76fd-95e7-4cee-9670-9dd0d9b67263";
            prepared
                .bind_persistence_run(run_id)
                .expect("bind persistence run");
            let snapshot_name = prepared.candidates[0]
                .snapshot_name
                .clone()
                .expect("planned snapshot basename");
            assert_eq!(Path::new(&snapshot_name).components().count(), 1);
            assert!(snapshot_name.starts_with(
                "context-snapshot-run_4b9c76fd95e74cee96709dd0d9b67263-resource_ref_"
            ));
            assert!(snapshot_name.len() < 128);
            assert!(!snapshot_name.contains(&"x".repeat(32)));
            assert!(
                !root
                    .path()
                    .join("attachments")
                    .join("durable-session")
                    .join(&snapshot_name)
                    .exists(),
                "read-only preparation must not publish bytes"
            );

            let frozen = publish_typed_resource_mentions(
                "durable-session",
                prepared,
                false,
                &mut attachments,
            )
            .expect("publish");
            let published = root
                .path()
                .join("attachments")
                .join("durable-session")
                .join(&snapshot_name);
            assert_eq!(
                std::fs::read(&published).expect("snapshot"),
                b"frozen bytes"
            );
            assert_eq!(
                frozen[0].snapshot_name.as_deref(),
                Some(snapshot_name.as_str())
            );
            assert!(run_owned_typed_resource_snapshot_owners()
                .expect("enumerate owners")
                .contains(&("durable-session".to_string(), run_id.to_string())));

            let referenced = HashSet::from([snapshot_name.clone()]);
            assert_eq!(
                reconcile_run_typed_resource_snapshots("durable-session", run_id, &referenced)
                    .expect("keep referenced snapshot"),
                0
            );
            assert!(published.exists());
            let foreign = HashSet::from([
                "context-snapshot-run_00000000000000000000000000000000-resource_ref_other.txt"
                    .to_string(),
            ]);
            assert!(
                reconcile_run_typed_resource_snapshots("durable-session", run_id, &foreign)
                    .is_err()
            );
            assert!(published.exists(), "foreign refs must fail closed");
            assert_eq!(
                reconcile_run_typed_resource_snapshots("durable-session", run_id, &HashSet::new())
                    .expect("remove unreferenced snapshot"),
                1
            );
            assert!(!published.exists());
        });
    }

    #[test]
    fn typed_resource_publish_failure_rolls_back_new_files() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let first = root.path().join("first.txt");
            let second = root.path().join("second.txt");
            std::fs::write(&first, b"first").expect("first");
            std::fs::write(&second, b"second").expect("second");
            let containment_root = root.path().canonicalize().expect("root canonical");
            let sources = vec![
                ResolvedMentionSource {
                    target_id: "first.txt".into(),
                    containment_root: containment_root.clone(),
                    relative_source: PathBuf::from("first.txt"),
                    source: first.canonicalize().expect("first canonical"),
                    attachment_source: "mention",
                },
                ResolvedMentionSource {
                    target_id: "second.txt".into(),
                    containment_root,
                    relative_source: PathBuf::from("second.txt"),
                    source: second.canonicalize().expect("second canonical"),
                    attachment_source: "mention",
                },
            ];
            let mut attachments = vec![mention_attachment(&first), mention_attachment(&second)];
            let mut prepared =
                prepare_resolved_mention_sources(&sources, false, &attachments).expect("prepare");
            prepared
                .bind_persistence_run("a2b41080-9186-49ff-a417-2dc8167c25f4")
                .expect("bind");
            let first_name = prepared.candidates[0]
                .snapshot_name
                .clone()
                .expect("first name");
            let second_name = prepared.candidates[1]
                .snapshot_name
                .clone()
                .expect("second name");
            let attachment_dir = paths::attachments_dir("publish-failure").expect("dir");
            std::fs::create_dir_all(&attachment_dir).expect("create dir");
            std::fs::write(attachment_dir.join(&second_name), b"preexisting").expect("collision");

            publish_typed_resource_mentions("publish-failure", prepared, false, &mut attachments)
                .expect_err("create-new collision must abort the batch");

            assert!(!attachment_dir.join(first_name).exists());
            assert_eq!(
                std::fs::read(attachment_dir.join(second_name)).expect("preexisting survives"),
                b"preexisting"
            );
            assert!(attachments
                .iter()
                .all(|attachment| attachment.data.is_none()));
            assert_eq!(attachments[0].file_path.as_deref(), first.to_str());
            assert_eq!(attachments[1].file_path.as_deref(), second.to_str());
        });
    }

    #[test]
    fn typed_resource_cleanup_rejects_unknown_or_cross_owner_names() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let run_id = "dff53eeb-5c97-4f13-8f45-4bdacbe2ed92";
            let foreign_run = "d6f693f0-4efe-4764-a9a2-ed415bbfef79";
            let session_id = "cleanup-guard";
            let attachment_dir = paths::attachments_dir(session_id).expect("dir");
            std::fs::create_dir_all(&attachment_dir).expect("create dir");
            let foreign_name = format!(
                "{}resource_ref_{}",
                typed_resource_run_prefix(foreign_run).expect("foreign prefix"),
                uuid::Uuid::new_v4().simple()
            );
            let foreign_path = attachment_dir.join(&foreign_name);
            std::fs::write(&foreign_path, b"keep").expect("foreign snapshot");
            let cleanup = crate::session::TypedResourceSnapshotCleanup {
                ledger_row_id: 1,
                run_id: run_id.to_string(),
                session_id: session_id.to_string(),
                snapshot_name: foreign_name,
            };

            remove_pending_typed_resource_snapshot(&cleanup)
                .expect_err("a ledger row cannot target another run prefix");
            assert_eq!(std::fs::read(foreign_path).expect("preserved"), b"keep");
        });
    }

    #[test]
    fn mixed_typed_resources_freeze_as_one_snapshot_batch() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let workspace_file = root.path().join("workspace.txt");
            let plan_file = root.path().join("plan.md");
            std::fs::write(&workspace_file, b"workspace").expect("workspace source");
            std::fs::write(&plan_file, b"plan").expect("plan source");
            let mut attachments = vec![
                mention_attachment(&workspace_file),
                Attachment {
                    name: "plan.md".to_string(),
                    mime_type: "text/markdown".to_string(),
                    source: Some("plan_mention".to_string()),
                    data: None,
                    file_path: Some(plan_file.to_string_lossy().into_owned()),
                    upload_id: None,
                    quote_lines: None,
                    quote_revealable: None,
                    quote_role: None,
                    quote_project_root: None,
                    quote_worktree_root: None,
                },
            ];
            let containment_root = root.path().canonicalize().expect("root canonical");
            let sources = vec![
                ResolvedMentionSource {
                    target_id: "workspace.txt".into(),
                    containment_root: containment_root.clone(),
                    relative_source: PathBuf::from("workspace.txt"),
                    source: workspace_file.canonicalize().expect("workspace canonical"),
                    attachment_source: "mention",
                },
                ResolvedMentionSource {
                    target_id: "abcdef12:v0".into(),
                    containment_root,
                    relative_source: PathBuf::from("plan.md"),
                    source: plan_file.canonicalize().expect("plan canonical"),
                    attachment_source: "plan_mention",
                },
            ];
            let frozen = freeze_resolved_mention_sources(
                "incognito-session",
                &sources,
                true,
                &mut attachments,
            )
            .expect("freeze mixed resources");
            assert_eq!(frozen.len(), 2);
            assert_eq!(&*frozen[0].bytes, b"workspace");
            assert_eq!(&*frozen[1].bytes, b"plan");
            assert!(frozen.iter().all(|entry| !entry.durable));
        });
    }

    #[test]
    fn sniff_png_magic() {
        assert_eq!(
            sniff_mime(b"\x89PNG\r\n\x1a\nrest", Path::new("x")),
            "image/png"
        );
    }

    #[test]
    fn sniff_pdf_magic() {
        assert_eq!(
            sniff_mime(b"%PDF-1.4\n...", Path::new("x.bin")),
            "application/pdf"
        );
    }

    #[test]
    fn sniff_fallback_ext() {
        assert_eq!(
            sniff_mime(b"plain text body", Path::new("/tmp/foo.txt")),
            "text/plain"
        );
    }

    #[test]
    fn sniff_fallback_octet_stream() {
        assert_eq!(
            sniff_mime(b"\x00\x01\x02unknown", Path::new("/tmp/x")),
            "application/octet-stream"
        );
    }

    #[test]
    fn quote_revealable_wire_is_optional_and_preserves_false() {
        let legacy: Attachment = serde_json::from_value(json!({
            "name": "legacy quote",
            "mime_type": "text/plain",
            "source": "quote"
        }))
        .expect("deserialize legacy quote");
        assert_eq!(legacy.quote_revealable, None);

        let visual: Attachment = serde_json::from_value(json!({
            "name": "visual quote",
            "mime_type": "text/plain",
            "source": "quote",
            "quote_revealable": false
        }))
        .expect("deserialize visual quote");
        assert_eq!(visual.quote_revealable, Some(false));
        assert_eq!(
            serde_json::to_value(&visual).expect("serialize visual quote")["quote_revealable"],
            false
        );
    }

    #[test]
    fn persist_chat_user_attachments_meta_keeps_message_quote_inline() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let mut attachments = vec![Attachment {
                name: "message-quote".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some(MESSAGE_QUOTE_SOURCE.to_string()),
                data: Some("Selected answer".to_string()),
                file_path: None,
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: Some("assistant".to_string()),
                quote_project_root: None,
                quote_worktree_root: None,
            }];

            let raw = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("persist message quote")
                .expect("message quote metadata");
            let value: Value = serde_json::from_str(&raw).expect("valid metadata json");

            assert_eq!(value[0]["kind"], MESSAGE_QUOTE_SOURCE);
            assert_eq!(value[0]["role"], "assistant");
            assert_eq!(value[0]["content"], "Selected answer");
            assert!(value[0].get("path").is_none());
        });
    }

    #[test]
    fn persist_chat_user_attachments_meta_keeps_quote_browser_provenance() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let mut attachments = vec![Attachment {
                name: "brief.md".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some("quote".to_string()),
                data: Some("quoted lines".to_string()),
                file_path: Some("/repos/shared-feature/brief.md".to_string()),
                upload_id: None,
                quote_lines: Some("3-5".to_string()),
                quote_revealable: Some(false),
                quote_project_root: Some(crate::agent::QuoteProjectRoot {
                    index: 1,
                    path: "/repos/shared".to_string(),
                }),
                quote_worktree_root: Some("/repos/shared-feature".to_string()),
                quote_role: None,
            }];

            let raw = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("persist file quote")
                .expect("file quote metadata");
            let value: Value = serde_json::from_str(&raw).expect("valid metadata json");

            assert_eq!(value[0]["kind"], "quote");
            assert_eq!(value[0]["path"], "brief.md");
            assert_eq!(value[0]["revealable"], false);
            assert_eq!(value[0]["project_root"]["index"], 1);
            assert_eq!(value[0]["project_root"]["path"], "/repos/shared");
            assert_eq!(value[0]["worktree_root"], "/repos/shared-feature");
        });
    }

    #[test]
    fn persist_chat_user_attachments_meta_skips_temp_path_traversal() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let temp_dir = root.path().join("attachments").join(TEMP_SESSION_ID);
            std::fs::create_dir_all(&temp_dir).expect("create temp dir");
            let outside = root.path().join("attachments").join("secret.txt");
            std::fs::write(&outside, b"secret").expect("write outside file");

            let traversal = temp_dir.join("..").join("secret.txt");
            let mut attachments = vec![Attachment {
                name: "secret.txt".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some("upload".to_string()),
                data: None,
                file_path: Some(traversal.to_string_lossy().to_string()),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];

            let meta = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("path traversal should be skipped without failing the chat request");
            assert!(meta.is_none());
            assert!(
                !root
                    .path()
                    .join("attachments")
                    .join("session-a")
                    .join("secret.txt")
                    .exists(),
                "outside file must not be copied into the session attachments directory"
            );
        });
    }

    #[test]
    fn persist_chat_user_attachments_meta_skips_missing_file_and_keeps_valid_attachment() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let saved = save_attachment_bytes(None, "note.txt", b"hello").expect("save temp");
            let missing = root
                .path()
                .join("attachments")
                .join(TEMP_SESSION_ID)
                .join("missing.txt");
            let mut attachments = vec![
                Attachment {
                    name: "missing.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    source: Some("upload".to_string()),
                    data: None,
                    file_path: Some(missing.to_string_lossy().to_string()),
                    upload_id: None,
                    quote_lines: None,
                    quote_revealable: None,
                    quote_role: None,
                    quote_project_root: None,
                    quote_worktree_root: None,
                },
                Attachment {
                    name: "note.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    source: Some("upload".to_string()),
                    data: None,
                    file_path: Some(saved.clone()),
                    upload_id: None,
                    quote_lines: None,
                    quote_revealable: None,
                    quote_role: None,
                    quote_project_root: None,
                    quote_worktree_root: None,
                },
            ];

            let meta = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("missing file should not fail the whole request")
                .expect("valid attachment should still produce metadata");

            let missing_after = attachments[0].file_path.as_deref().expect("missing path");
            assert_eq!(missing_after, missing.to_string_lossy());
            let final_path = attachments[1].file_path.as_deref().expect("final path");
            assert_session_attachment_path(final_path, root.path(), "session-a");
            assert!(!Path::new(&saved).exists(), "temp file should be moved");
            assert_eq!(std::fs::read(final_path).expect("read final"), b"hello");
            assert!(meta.contains("\"name\":\"note.txt\""));
            assert!(!meta.contains("missing.txt"));
        });
    }

    #[test]
    fn persist_chat_user_attachments_meta_moves_temp_file_into_session_dir() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let saved = save_attachment_bytes(None, "note.txt", b"hello").expect("save temp");
            let mut attachments = vec![Attachment {
                name: "note.txt".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some("upload".to_string()),
                data: None,
                file_path: Some(saved.clone()),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];

            let meta = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("persist")
                .expect("meta");

            let final_path = attachments[0].file_path.as_deref().expect("final path");
            assert_session_attachment_path(final_path, root.path(), "session-a");
            assert!(!Path::new(&saved).exists(), "temp file should be moved");
            assert_eq!(std::fs::read(final_path).expect("read final"), b"hello");
            assert!(meta.contains("\"name\":\"note.txt\""));
            assert!(meta.contains("\"mime_type\":\"text/plain\""));
        });
    }

    #[test]
    fn persist_chat_user_attachments_meta_skips_mention_paths() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let mentioned = root.path().join("project-note.md");
            std::fs::write(&mentioned, b"project").expect("write mention file");
            let original = mentioned.to_string_lossy().to_string();
            let mut attachments = vec![Attachment {
                name: "project-note.md".to_string(),
                mime_type: "text/markdown".to_string(),
                source: Some("mention".to_string()),
                data: None,
                file_path: Some(original.clone()),
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];

            let meta = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("mention path should not fail persistence");

            assert!(meta.is_none());
            assert_eq!(attachments[0].file_path.as_deref(), Some(original.as_str()));
        });
    }

    #[test]
    fn persist_queued_chat_attachments_accepts_text_only_message_without_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let mut attachments = Vec::new();
            persist_queued_chat_attachments("session-text-only", "request", &mut attachments)
                .expect("text-only queue persistence");
            assert!(!root
                .path()
                .join("attachments")
                .join("session-text-only")
                .exists());
        });
    }

    #[test]
    fn channel_queue_attachment_is_durable_without_base64_and_removed_on_discard() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let mut attachments = vec![Attachment {
                name: "channel-image.png".to_string(),
                mime_type: "image/png".to_string(),
                source: Some(CHANNEL_QUEUE_SOURCE.to_string()),
                data: Some("aGVsbG8=".to_string()),
                file_path: None,
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];

            persist_queued_chat_attachments(
                "session-channel-queue",
                "request/with/slashes",
                &mut attachments,
            )
            .expect("persist channel queue attachment");

            assert!(attachments[0].data.is_none());
            let queued_path = PathBuf::from(
                attachments[0]
                    .file_path
                    .as_deref()
                    .expect("queue attachment path"),
            );
            assert!(queued_path.exists());
            assert!(queued_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("queue_request_with_slashes_")));

            remove_discarded_queued_attachments(
                "session-channel-queue",
                "request/with/slashes",
                &attachments,
            );
            assert!(!queued_path.exists());
        });
    }

    #[test]
    fn staged_upload_is_claimed_into_session_and_source_is_removed() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let lease = stage_chat_attachment("note.txt", "text/plain", b"hello lease")
                .expect("stage attachment");
            let source = pending_upload_path(&lease.upload_id).expect("lease path");
            let mut attachments = vec![Attachment {
                name: lease.name,
                mime_type: lease.mime_type,
                source: Some("upload".to_string()),
                data: None,
                file_path: None,
                upload_id: Some(lease.upload_id),
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];

            let meta = persist_chat_user_attachments_meta("session-lease", &mut attachments)
                .expect("claim attachment")
                .expect("attachment metadata");

            assert!(!source.exists(), "claimed lease must be removed");
            assert!(attachments[0].upload_id.is_none());
            let final_path = attachments[0].file_path.as_deref().expect("claimed path");
            assert_session_attachment_path(final_path, root.path(), "session-lease");
            assert_eq!(
                std::fs::read(final_path).expect("read claimed file"),
                b"hello lease"
            );
            assert!(meta.contains("note.txt"));
        });
    }

    #[test]
    fn generic_chunked_upload_is_claimed_into_chat_session() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let lease =
                crate::file_upload::start_upload(crate::file_upload::FileUploadStartInput {
                    purpose: crate::file_upload::FileUploadPurpose::ChatAttachment,
                    file_name: "chunked.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 7,
                })
                .expect("start generic upload");
            crate::file_upload::upload_chunk(&lease.upload_id, 0, b"chunked")
                .expect("upload chunk");
            crate::file_upload::complete_upload(&lease.upload_id).expect("complete upload");

            let mut attachments = vec![Attachment {
                name: "chunked.txt".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some("upload".to_string()),
                data: None,
                file_path: None,
                upload_id: Some(lease.upload_id.clone()),
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];
            let metadata = persist_chat_user_attachments_meta("session-a", &mut attachments)
                .expect("claim")
                .expect("metadata");
            let final_path = attachments[0].file_path.as_deref().expect("final path");
            assert_session_attachment_path(final_path, root.path(), "session-a");
            assert_eq!(std::fs::read(final_path).unwrap(), b"chunked");
            assert!(attachments[0].upload_id.is_none());
            assert!(crate::file_upload::upload_status(&lease.upload_id).is_err());
            assert!(metadata.contains("chunked.txt"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn generic_chat_claim_does_not_follow_existing_destination_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let lease =
                crate::file_upload::start_upload(crate::file_upload::FileUploadStartInput {
                    purpose: crate::file_upload::FileUploadPurpose::ChatAttachment,
                    file_name: "chunked.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 7,
                })
                .expect("start generic upload");
            crate::file_upload::upload_chunk(&lease.upload_id, 0, b"chunked")
                .expect("upload chunk");
            crate::file_upload::complete_upload(&lease.upload_id).expect("complete upload");

            let att_dir = crate::paths::attachments_dir("session-symlink").expect("attachment dir");
            std::fs::create_dir_all(&att_dir).expect("create attachment dir");
            let outside = root.path().join("outside.txt");
            std::fs::write(&outside, b"original").expect("outside file");
            let destination = att_dir.join(format!("{}_chunked.txt", lease.upload_id));
            symlink(&outside, &destination).expect("destination symlink");

            let mut attachments = vec![Attachment {
                name: "chunked.txt".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some("upload".to_string()),
                data: None,
                file_path: None,
                upload_id: Some(lease.upload_id.clone()),
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            }];
            persist_chat_user_attachments_meta("session-symlink", &mut attachments)
                .expect_err("pre-existing destination symlink must fail closed");

            assert_eq!(std::fs::read(&outside).unwrap(), b"original");
            assert!(std::fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                crate::file_upload::upload_status(&lease.upload_id)
                    .expect("lease remains retryable")
                    .state,
                crate::file_upload::FileUploadState::Complete
            );
            assert_eq!(
                attachments[0].upload_id.as_deref(),
                Some(lease.upload_id.as_str())
            );
            assert!(attachments[0].file_path.is_none());
        });
    }

    #[test]
    fn missing_upload_keeps_all_other_leases_retryable() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let lease = stage_chat_attachment("kept.txt", "text/plain", b"retry me")
                .expect("stage attachment");
            let source = pending_upload_path(&lease.upload_id).expect("lease path");
            let mut attachments = vec![
                Attachment {
                    name: lease.name,
                    mime_type: lease.mime_type,
                    source: Some("upload".to_string()),
                    data: None,
                    file_path: None,
                    upload_id: Some(lease.upload_id),
                    quote_lines: None,
                    quote_revealable: None,
                    quote_role: None,
                    quote_project_root: None,
                    quote_worktree_root: None,
                },
                Attachment {
                    name: "missing.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    source: Some("upload".to_string()),
                    data: None,
                    file_path: None,
                    upload_id: Some(uuid::Uuid::new_v4().to_string()),
                    quote_lines: None,
                    quote_revealable: None,
                    quote_role: None,
                    quote_project_root: None,
                    quote_worktree_root: None,
                },
            ];

            assert!(
                persist_chat_user_attachments_meta("session-rollback", &mut attachments).is_err()
            );
            assert!(
                source.exists(),
                "successful lease must remain available for retry"
            );
            assert!(attachments
                .iter()
                .all(|attachment| attachment.file_path.is_none()));
            let session_dir = root.path().join("attachments").join("session-rollback");
            assert_eq!(
                std::fs::read_dir(session_dir).expect("session dir").count(),
                0,
                "prepared destinations must be rolled back"
            );
        });
    }

    #[test]
    fn attachment_count_limit_is_enforced_before_claiming() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let template = Attachment {
                name: "note.txt".to_string(),
                mime_type: "text/plain".to_string(),
                source: Some("upload".to_string()),
                data: Some(base64::engine::general_purpose::STANDARD.encode(b"x")),
                file_path: None,
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            };
            let mut attachments = vec![template; MAX_CHAT_ATTACHMENTS + 1];
            assert!(
                persist_chat_user_attachments_meta("session-too-many", &mut attachments).is_err()
            );
        });
    }
}
