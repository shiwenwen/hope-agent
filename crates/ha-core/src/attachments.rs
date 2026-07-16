//! Attachment helpers shared by Tauri commands and HTTP routes.
//!
//! Writes uploaded bytes to the per-session attachments directory (or a
//! temporary bucket when the session hasn't been created yet) and returns
//! the absolute path so the caller can hand it to the agent/chat engine.

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::agent::Attachment;
use crate::paths;

/// Pseudo-session id for pre-session attachments (uploads that predate a
/// chat session). Maps to `~/.hope-agent/attachments/_temp/`.
pub const TEMP_SESSION_ID: &str = "_temp";
pub const PASTED_TEXT_SOURCE: &str = "pasted_text";
pub const MESSAGE_QUOTE_SOURCE: &str = "message_quote";
pub const MAX_CHAT_ATTACHMENTS: usize = 64;
pub const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;
/// Static compatibility ceiling for pre-chunked chat uploads and Base64 wire
/// payloads. Only the generic upload-lease protocol can use a configured
/// limit above 20 MiB.
pub const LEGACY_MAX_CHAT_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
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
/// (HTTP sink appends `?token=`; Tauri sink leaves as-is, and the frontend
/// prefers `local_path` via `convertFileSrc`). `local_path` is the absolute
/// path on the server, used by IM channel workers to read bytes and by the
/// Tauri frontend to open/reveal locally. HTTP sinks strip `local_path`
/// from events so it never leaks to web clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    /// Logical URL `/api/attachments/{sessionId}/{filename}`. Frontends resolve
    /// this through the transport layer (Tauri uses `local_path`, HTTP adds
    /// `?token=`).
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
            meta_list.push(json!({
                "kind": "quote",
                "name": att.name,
                "path": att.file_path,
                "lines": att.quote_lines,
                "content": att.data,
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
                        app_warn!(
                            "app",
                            "chat",
                            "Falling back to attachment bytes for '{}': {}",
                            att.name,
                            err
                        );
                    }
                }
            }
            let path = match save_bytes_in_dir(&att_dir, &att.name, &decoded)
                .with_context(|| format!("save image attachment {}", att.name))
            {
                Ok(path) => path,
                Err(err) => {
                    app_warn!("app", "chat", "Skipping attachment '{}': {}", att.name, err);
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
                app_warn!("app", "chat", "Skipping attachment '{}': {}", att.name, err);
                continue;
            }
        };
        let canonical_final_path = match final_path
            .canonicalize()
            .with_context(|| format!("canonicalize attachment {}", final_path.display()))
        {
            Ok(path) => path,
            Err(err) => {
                app_warn!("app", "chat", "Skipping attachment '{}': {}", att.name, err);
                continue;
            }
        };
        if !canonical_final_path.starts_with(&canonical_att_dir) {
            app_warn!(
                "app",
                "chat",
                "attachment path outside allowed attachment directories: {}",
                src_path.display()
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

/// Move queue attachments into the session-owned attachment directory before
/// serializing the queue row. Uploaded image bytes are cleared after a durable
/// `file_path` is established so the queue DB never balloons with base64 data;
/// quotes retain their inline excerpt and mention attachments remain references.
pub fn persist_queued_chat_attachments(
    session_id: &str,
    request_id: &str,
    attachments: &mut [Attachment],
) -> Result<()> {
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
                Some("upload") | Some(PASTED_TEXT_SOURCE)
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
            Some("upload") | Some(PASTED_TEXT_SOURCE)
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
    matches!(source, None | Some("upload") | Some(PASTED_TEXT_SOURCE))
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
                quote_role: Some("assistant".to_string()),
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
                quote_role: None,
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
                    quote_role: None,
                },
                Attachment {
                    name: "note.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    source: Some("upload".to_string()),
                    data: None,
                    file_path: Some(saved.clone()),
                    upload_id: None,
                    quote_lines: None,
                    quote_role: None,
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
                quote_role: None,
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
                quote_role: None,
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
                quote_role: None,
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
                quote_role: None,
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
                quote_role: None,
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
                    quote_role: None,
                },
                Attachment {
                    name: "missing.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    source: Some("upload".to_string()),
                    data: None,
                    file_path: None,
                    upload_id: Some(uuid::Uuid::new_v4().to_string()),
                    quote_lines: None,
                    quote_role: None,
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
                quote_role: None,
            };
            let mut attachments = vec![template; MAX_CHAT_ATTACHMENTS + 1];
            assert!(
                persist_chat_user_attachments_meta("session-too-many", &mut attachments).is_err()
            );
        });
    }
}
