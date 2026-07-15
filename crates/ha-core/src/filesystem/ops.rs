//! Working-directory file operations backing the project file browser.
//!
//! Every function takes a [`WorkspaceScope`] and operates on paths relative to
//! its root; the scope enforces containment, so these functions never see an
//! escaping path. DTOs serialize as camelCase to match the transport layer.

use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use super::workspace::WorkspaceScope;
use super::{FilesystemError, Result};

/// Static compatibility ceiling for the legacy whole-body workspace upload.
/// New clients use resumable leases and may opt into a higher configured cap.
pub const LEGACY_MAX_WORKSPACE_UPLOAD_BYTES: u64 = 20 * 1024 * 1024;

/// Per-directory entry cap (same intent as `MAX_LIST_ENTRIES` in `mod.rs`).
const MAX_LIST_ENTRIES: usize = 5000;

/// Serializes owner-plane workspace mutations inside this process. In
/// particular, the editor's hash comparison and atomic publish must be one
/// critical section relative to HTTP/Tauri writes, uploads, renames and deletes.
static WORKSPACE_MUTATION_LOCK: Mutex<()> = Mutex::new(());

fn lock_workspace_mutations() -> std::sync::MutexGuard<'static, ()> {
    WORKSPACE_MUTATION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---- DTOs ------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    pub modified_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceListing {
    pub dir_rel: String,
    pub parent_rel: Option<String>,
    pub entries: Vec<WorkspaceEntry>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTextContent {
    pub rel_path: String,
    pub content: String,
    pub is_binary: bool,
    pub mime: Option<String>,
    pub total_lines: usize,
    pub size_bytes: u64,
    pub truncated: bool,
    pub content_hash: Option<String>,
    pub is_utf8: bool,
    pub line_ending: LineEnding,
    pub has_utf8_bom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    Lf,
    Crlf,
    Cr,
    Mixed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedImageDto {
    pub data: String,
    pub mime: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedContent {
    pub rel_path: String,
    /// `"pdf"` or `"office"`.
    pub kind: String,
    pub text: Option<String>,
    pub images: Vec<ExtractedImageDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    pub rel_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FileWriteOutcome {
    Saved {
        #[serde(rename = "relPath")]
        rel_path: String,
        #[serde(rename = "sizeBytes")]
        size_bytes: u64,
        #[serde(rename = "contentHash")]
        content_hash: String,
    },
    Conflict {
        reason: FileWriteConflictReason,
        #[serde(rename = "currentContentHash", skip_serializing_if = "Option::is_none")]
        current_content_hash: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileWriteConflictReason {
    Changed,
    Deleted,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameResult {
    pub rel_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResult {
    pub rel_path: String,
    pub size_bytes: u64,
}

// ---- Operations ------------------------------------------------------------

/// List a single directory level relative to the workspace root.
pub fn project_list_dir(scope: &WorkspaceScope, rel: &str) -> Result<WorkspaceListing> {
    let abs = scope.resolve_existing(rel)?;
    if !abs.is_dir() {
        return Err(FilesystemError::bad_input("path is not a directory"));
    }
    let read_dir = std::fs::read_dir(&abs)
        .map_err(|e| FilesystemError::internal(format!("read dir: {}", e)))?;

    let mut entries: Vec<WorkspaceEntry> = Vec::new();
    let mut truncated = false;
    for entry in read_dir {
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let ft = meta.file_type();
        let is_dir = if ft.is_symlink() {
            std::fs::metadata(entry.path())
                .map(|m| m.is_dir())
                .unwrap_or(false)
        } else {
            ft.is_dir()
        };
        let size = if !is_dir { Some(meta.len()) } else { None };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        entries.push(WorkspaceEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            rel_path: scope.rel_of(&entry.path()),
            is_dir,
            is_symlink: ft.is_symlink(),
            size,
            modified_ms,
        });
    }

    entries.sort_by(|a, b| match b.is_dir.cmp(&a.is_dir) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    let dir_rel = scope.rel_of(&abs);
    let parent_rel = if dir_rel.is_empty() {
        None
    } else {
        Path::new(&dir_rel)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    };

    Ok(WorkspaceListing {
        dir_rel,
        parent_rel,
        entries,
        truncated,
    })
}

/// Read a text file's full content (binary / oversized files return
/// `is_binary = true` with no content, so the caller falls back to raw/preview).
pub fn project_read_text(scope: &WorkspaceScope, rel: &str) -> Result<FileTextContent> {
    let abs = scope.resolve_existing(rel)?;
    let rel_path = scope.rel_of(&abs);
    read_text_at(&abs, rel_path)
}

/// Read a text file by **absolute path** (preview-by-path). Performs no scope
/// containment — the caller must authorize the path first (desktop trusts local
/// paths; the HTTP layer gates by session reference + working-dir containment,
/// see `is_session_path_authorized`). The DTO's `rel_path` is the absolute path.
pub fn read_text_abs(abs: &Path) -> Result<FileTextContent> {
    read_text_at(abs, abs.to_string_lossy().to_string())
}

/// Shared body for scope-relative and absolute-path text reads. `rel_path` is
/// the display/quote path embedded in the returned DTO.
fn read_text_at(abs: &Path, rel_path: String) -> Result<FileTextContent> {
    let meta =
        std::fs::metadata(abs).map_err(|e| FilesystemError::internal(format!("stat: {}", e)))?;
    if meta.is_dir() {
        return Err(FilesystemError::bad_input("path is a directory"));
    }
    let size_bytes = meta.len();
    let mime = mime_for_path(abs);

    let max_preview_bytes = crate::config::cached_config()
        .filesystem
        .max_text_preview_bytes();
    if size_bytes > max_preview_bytes || looks_binary(abs) {
        return Ok(FileTextContent {
            rel_path,
            content: String::new(),
            is_binary: true,
            mime,
            total_lines: 0,
            size_bytes,
            truncated: size_bytes > max_preview_bytes,
            content_hash: None,
            is_utf8: false,
            line_ending: LineEnding::Lf,
            has_utf8_bom: false,
        });
    }

    let mut file =
        std::fs::File::open(abs).map_err(|e| FilesystemError::internal(format!("open: {}", e)))?;
    let mut bytes = Vec::with_capacity(size_bytes.min(max_preview_bytes) as usize);
    Read::by_ref(&mut file)
        .take(max_preview_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| FilesystemError::internal(format!("read: {}", e)))?;
    if bytes.len() as u64 > max_preview_bytes {
        return Ok(FileTextContent {
            rel_path,
            content: String::new(),
            is_binary: true,
            mime,
            total_lines: 0,
            size_bytes: size_bytes.max(bytes.len() as u64),
            truncated: true,
            content_hash: None,
            is_utf8: false,
            line_ending: LineEnding::Lf,
            has_utf8_bom: false,
        });
    }
    let content_hash = blake3::hash(&bytes).to_hex().to_string();
    let has_utf8_bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let text_bytes = if has_utf8_bom { &bytes[3..] } else { &bytes };
    let is_utf8 = std::str::from_utf8(text_bytes).is_ok();
    let content = String::from_utf8_lossy(text_bytes).to_string();
    let total_lines = content.lines().count();
    let line_ending = detect_line_ending(&content);
    Ok(FileTextContent {
        rel_path,
        content,
        is_binary: false,
        mime,
        total_lines,
        size_bytes,
        truncated: false,
        content_hash: Some(content_hash),
        is_utf8,
        line_ending,
        has_utf8_bom,
    })
}

fn detect_line_ending(content: &str) -> LineEnding {
    let bytes = content.as_bytes();
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let mut cr = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 2;
            }
            b'\r' => {
                cr += 1;
                i += 1;
            }
            b'\n' => {
                lf += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    let kinds = usize::from(lf > 0) + usize::from(crlf > 0) + usize::from(cr > 0);
    if kinds > 1 {
        LineEnding::Mixed
    } else if crlf > 0 {
        LineEnding::Crlf
    } else if cr > 0 {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

/// Extract content from a PDF / Office document for preview, reusing
/// [`crate::file_extract`]. PDFs return per-page PNGs as base64 images; Office
/// formats return extracted text (and any embedded images).
pub fn project_fs_extract(scope: &WorkspaceScope, rel: &str) -> Result<ExtractedContent> {
    let abs = scope.resolve_existing(rel)?;
    let rel_path = scope.rel_of(&abs);
    extract_at(&abs, rel_path)
}

/// Extract a PDF / Office document by **absolute path** (preview-by-path).
/// Same authorization contract as [`read_text_abs`].
pub fn extract_abs(abs: &Path) -> Result<ExtractedContent> {
    extract_at(abs, abs.to_string_lossy().to_string())
}

/// Shared body for scope-relative and absolute-path document extraction.
fn extract_at(abs: &Path, rel_path: String) -> Result<ExtractedContent> {
    let meta =
        std::fs::metadata(abs).map_err(|e| FilesystemError::internal(format!("stat: {}", e)))?;
    let max_extract_bytes = crate::config::cached_config()
        .filesystem
        .max_document_preview_bytes();
    if meta.len() > max_extract_bytes {
        return Err(FilesystemError::bad_input(format!(
            "file too large to preview: {} bytes (max {} bytes)",
            meta.len(),
            max_extract_bytes
        )));
    }
    let path_str = abs.to_string_lossy().to_string();
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mime = mime_for_path(abs).unwrap_or_else(|| "application/octet-stream".to_string());
    let kind = if mime == "application/pdf" {
        "pdf"
    } else {
        "office"
    };
    let extracted = crate::file_extract::extract(&path_str, &name, &mime);
    let images = extracted
        .images
        .into_iter()
        .map(|im| ExtractedImageDto {
            data: im.data,
            mime: im.mime_type,
            label: im.label,
        })
        .collect();
    Ok(ExtractedContent {
        rel_path,
        kind: kind.to_string(),
        text: extracted.text,
        images,
    })
}

/// Write text content to a file (saving an edit or creating a new file).
/// `create_only` makes the call fail if the target already exists. The write is
/// **atomic** ([`crate::platform::write_atomic`]: temp file in the same dir →
/// fsync → rename), so a crash / power loss mid-write leaves either the old file
/// intact or the new one complete — never a truncated note. Matters most for
/// bound external vaults.
pub fn project_write_text(
    scope: &WorkspaceScope,
    rel: &str,
    content: &str,
    create_only: bool,
) -> Result<WriteResult> {
    let max_bytes = crate::config::cached_config()
        .filesystem
        .max_text_edit_bytes();
    if content.len() as u64 > max_bytes {
        return Err(FilesystemError::bad_input(format!(
            "text is too large to edit: {} bytes (max {} bytes)",
            content.len(),
            max_bytes
        )));
    }
    let abs = scope.resolve_new(rel)?;
    let _guard = lock_workspace_mutations();
    // Atomic temp+rename lives in `platform/` (cross-platform rename / permission
    // handling in one place); it creates parent dirs itself.
    let write = if create_only {
        crate::platform::write_atomic_create_new(&abs, content.as_bytes())
    } else {
        crate::platform::write_atomic(&abs, content.as_bytes())
    };
    write.map_err(|e| {
        if create_only && e.kind() == std::io::ErrorKind::AlreadyExists {
            FilesystemError::bad_input("file already exists")
        } else {
            FilesystemError::internal(format!("write: {e}"))
        }
    })?;
    Ok(WriteResult {
        rel_path: scope.rel_of(&abs),
        size_bytes: content.len() as u64,
    })
}

/// Compare-and-swap text write used by the interactive workspace editor.
/// Existing files require the raw-byte BLAKE3 returned by `project_read_text`;
/// new files use `create_only` and never overwrite an existing path.
pub fn project_write_text_checked(
    scope: &WorkspaceScope,
    rel: &str,
    content: &str,
    create_only: bool,
    expected_file_hash: Option<&str>,
) -> Result<FileWriteOutcome> {
    let max_bytes = crate::config::cached_config()
        .filesystem
        .max_text_edit_bytes();
    if content.len() as u64 > max_bytes {
        return Err(FilesystemError::bad_input(format!(
            "text is too large to edit: {} bytes (max {} bytes)",
            content.len(),
            max_bytes
        )));
    }
    let abs = scope.resolve_new(rel)?;
    let _guard = lock_workspace_mutations();
    let bytes = content.as_bytes();
    if create_only {
        match crate::platform::write_atomic_create_new(&abs, bytes) {
            Ok(()) => {
                return Ok(FileWriteOutcome::Saved {
                    rel_path: scope.rel_of(&abs),
                    size_bytes: bytes.len() as u64,
                    content_hash: blake3::hash(bytes).to_hex().to_string(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let current = std::fs::read(&abs)
                    .map_err(|e| FilesystemError::internal(format!("read existing file: {e}")))?;
                return Ok(FileWriteOutcome::Conflict {
                    reason: FileWriteConflictReason::Changed,
                    current_content_hash: Some(blake3::hash(&current).to_hex().to_string()),
                });
            }
            Err(error) => {
                return Err(FilesystemError::internal(format!("write: {error}")));
            }
        }
    }
    let expected = expected_file_hash
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| FilesystemError::bad_input("expectedFileHash is required"))?;
    if !abs.exists() {
        return Ok(FileWriteOutcome::Conflict {
            reason: FileWriteConflictReason::Deleted,
            current_content_hash: None,
        });
    }
    let current = std::fs::read(&abs)
        .map_err(|e| FilesystemError::internal(format!("read before write: {e}")))?;
    let current_hash = blake3::hash(&current).to_hex().to_string();
    if current_hash != expected {
        return Ok(FileWriteOutcome::Conflict {
            reason: FileWriteConflictReason::Changed,
            current_content_hash: Some(current_hash),
        });
    }

    // Fully prepare and fsync the replacement before the final comparison.
    // This keeps the unavoidable external-writer race window down to the
    // atomic publish itself instead of including temp creation + disk I/O.
    let parent = abs
        .parent()
        .ok_or_else(|| FilesystemError::internal("file has no parent directory"))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| FilesystemError::internal(format!("create staged write: {e}")))?;
    staged
        .write_all(bytes)
        .and_then(|()| staged.flush())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|e| FilesystemError::internal(format!("stage write: {e}")))?;
    if let Ok(metadata) = std::fs::metadata(&abs) {
        std::fs::set_permissions(staged.path(), metadata.permissions())
            .map_err(|e| FilesystemError::internal(format!("preserve permissions: {e}")))?;
    }

    let latest = match std::fs::read(&abs) {
        Ok(latest) => latest,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileWriteOutcome::Conflict {
                reason: FileWriteConflictReason::Deleted,
                current_content_hash: None,
            });
        }
        Err(error) => {
            return Err(FilesystemError::internal(format!(
                "re-read before publish: {error}"
            )));
        }
    };
    let latest_hash = blake3::hash(&latest).to_hex().to_string();
    if latest_hash != expected {
        return Ok(FileWriteOutcome::Conflict {
            reason: FileWriteConflictReason::Changed,
            current_content_hash: Some(latest_hash),
        });
    }
    let staged_path = staged.into_temp_path();
    crate::platform::publish_atomic_file(staged_path.as_ref(), &abs, true)
        .map_err(|e| FilesystemError::internal(format!("publish write: {e}")))?;
    Ok(FileWriteOutcome::Saved {
        rel_path: scope.rel_of(&abs),
        size_bytes: bytes.len() as u64,
        content_hash: blake3::hash(bytes).to_hex().to_string(),
    })
}

/// Delete a file or directory. A non-empty directory requires `recursive`.
pub fn project_delete(scope: &WorkspaceScope, rel: &str, recursive: bool) -> Result<()> {
    if rel.trim().trim_start_matches('/').is_empty() {
        return Err(FilesystemError::bad_input(
            "cannot delete the workspace root",
        ));
    }
    let abs = scope.resolve_existing(rel)?;
    let _guard = lock_workspace_mutations();
    let meta = std::fs::symlink_metadata(&abs)
        .map_err(|e| FilesystemError::internal(format!("stat: {}", e)))?;
    if meta.is_dir() {
        let empty = std::fs::read_dir(&abs)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true);
        if !empty && !recursive {
            return Err(FilesystemError::bad_input("directory is not empty"));
        }
        std::fs::remove_dir_all(&abs)
            .map_err(|e| FilesystemError::internal(format!("remove dir: {}", e)))?;
    } else {
        std::fs::remove_file(&abs)
            .map_err(|e| FilesystemError::internal(format!("remove file: {}", e)))?;
    }
    Ok(())
}

/// Rename / move an entry within the workspace. Refuses to clobber an existing
/// destination unless `overwrite` is set.
pub fn project_rename(
    scope: &WorkspaceScope,
    from_rel: &str,
    to_rel: &str,
    overwrite: bool,
) -> Result<RenameResult> {
    let from = scope.resolve_existing(from_rel)?;
    let to = scope.resolve_new(to_rel)?;
    let _guard = lock_workspace_mutations();
    if to.exists() && !overwrite {
        return Err(FilesystemError::bad_input("destination already exists"));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| FilesystemError::internal(format!("create parent: {}", e)))?;
    }
    std::fs::rename(&from, &to).map_err(|e| FilesystemError::internal(format!("rename: {}", e)))?;
    Ok(RenameResult {
        rel_path: scope.rel_of(&to),
    })
}

/// Create a directory (and any missing parents). Idempotent.
pub fn project_mkdir(scope: &WorkspaceScope, rel: &str) -> Result<WriteResult> {
    if rel.trim().trim_start_matches('/').is_empty() {
        return Err(FilesystemError::bad_input("directory name is empty"));
    }
    let abs = scope.resolve_new(rel)?;
    let _guard = lock_workspace_mutations();
    std::fs::create_dir_all(&abs)
        .map_err(|e| FilesystemError::internal(format!("mkdir: {}", e)))?;
    Ok(WriteResult {
        rel_path: scope.rel_of(&abs),
        size_bytes: 0,
    })
}

/// Upload a file into `dir_rel` (relative to the workspace root). Collisions
/// are de-duplicated with a ` (N)` suffix unless `overwrite` is set.
pub fn project_upload(
    scope: &WorkspaceScope,
    dir_rel: &str,
    file_name: &str,
    data: &[u8],
    overwrite: bool,
) -> Result<UploadResult> {
    let max_bytes = crate::config::cached_config()
        .filesystem
        .max_workspace_upload_bytes()
        .min(LEGACY_MAX_WORKSPACE_UPLOAD_BYTES);
    if data.len() as u64 > max_bytes {
        return Err(FilesystemError::bad_input(format!(
            "file too large: {} bytes (max {} bytes)",
            data.len(),
            max_bytes
        )));
    }
    let safe = sanitize_name(file_name);
    if safe.is_empty() {
        return Err(FilesystemError::bad_input("invalid file name"));
    }

    let dir_abs = scope.resolve_new(dir_rel)?;
    let _guard = lock_workspace_mutations();
    std::fs::create_dir_all(&dir_abs)
        .map_err(|e| FilesystemError::internal(format!("create dir: {}", e)))?;

    let dir_clean = dir_rel.trim().trim_start_matches('/').trim_end_matches('/');
    let target_rel = if dir_clean.is_empty() {
        safe.clone()
    } else {
        format!("{}/{}", dir_clean, safe)
    };
    let mut abs = scope.resolve_new(&target_rel)?;
    if abs.exists() && !overwrite {
        abs = dedupe_path(&abs);
    }
    crate::platform::write_atomic(&abs, data)
        .map_err(|e| FilesystemError::internal(format!("write: {e}")))?;
    Ok(UploadResult {
        rel_path: scope.rel_of(&abs),
        size_bytes: data.len() as u64,
    })
}

/// Upload an existing local file without buffering it in memory. The source is
/// copied into a workspace-local staging file and then published atomically.
pub fn project_upload_file(
    scope: &WorkspaceScope,
    dir_rel: &str,
    file_name: &str,
    source_path: &Path,
    overwrite: bool,
) -> Result<UploadResult> {
    let source_metadata = std::fs::symlink_metadata(source_path)
        .map_err(|e| FilesystemError::bad_input(format!("stat upload source: {e}")))?;
    if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
        return Err(FilesystemError::bad_input(
            "upload source must be a regular file",
        ));
    }
    let max_bytes = crate::config::cached_config()
        .filesystem
        .max_workspace_upload_bytes()
        .min(LEGACY_MAX_WORKSPACE_UPLOAD_BYTES);
    if source_metadata.len() > max_bytes {
        return Err(FilesystemError::bad_input(format!(
            "file too large: {} bytes (max {} bytes)",
            source_metadata.len(),
            max_bytes
        )));
    }
    let safe = sanitize_name(file_name);
    if safe.is_empty() {
        return Err(FilesystemError::bad_input("invalid file name"));
    }

    let dir_abs = scope.resolve_new(dir_rel)?;
    let _guard = lock_workspace_mutations();
    std::fs::create_dir_all(&dir_abs)
        .map_err(|e| FilesystemError::internal(format!("create dir: {e}")))?;
    let dir_clean = dir_rel.trim().trim_start_matches('/').trim_end_matches('/');
    let target_rel = if dir_clean.is_empty() {
        safe.clone()
    } else {
        format!("{dir_clean}/{safe}")
    };
    let mut target = scope.resolve_new(&target_rel)?;
    if target.exists() && !overwrite {
        target = dedupe_path(&target);
    }
    let staging = dir_abs.join(format!(".{safe}.upload-{}.tmp", uuid::Uuid::new_v4()));
    let publish = (|| -> Result<u64> {
        let source = std::fs::File::open(source_path)
            .map_err(|e| FilesystemError::bad_input(format!("open upload source: {e}")))?;
        let mut limited = source.take(max_bytes.saturating_add(1));
        let mut staged = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(|e| FilesystemError::internal(format!("create upload staging file: {e}")))?;
        let copied = std::io::copy(&mut limited, &mut staged)
            .map_err(|e| FilesystemError::internal(format!("copy upload: {e}")))?;
        if copied > max_bytes || copied != source_metadata.len() {
            return Err(FilesystemError::bad_input(
                "upload source changed or exceeded the configured limit",
            ));
        }
        staged
            .flush()
            .and_then(|_| staged.sync_all())
            .map_err(|e| FilesystemError::internal(format!("sync upload: {e}")))?;
        drop(staged);
        crate::platform::publish_atomic_file(&staging, &target, overwrite)
            .map_err(|e| FilesystemError::internal(format!("publish upload: {e}")))?;
        Ok(copied)
    })();
    if publish.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    let size_bytes = publish?;
    Ok(UploadResult {
        rel_path: scope.rel_of(&target),
        size_bytes,
    })
}

/// Claim a completed `workspace_upload` lease into this scope. The bytes are
/// copied into a sibling staging file, fsynced, then published atomically.
pub fn project_claim_upload(
    scope: &WorkspaceScope,
    dir_rel: &str,
    upload_id: &str,
    file_name: Option<&str>,
    overwrite: bool,
) -> Result<UploadResult> {
    let max_bytes = crate::config::cached_config()
        .filesystem
        .max_workspace_upload_bytes();
    let initial = crate::file_upload::upload_status(upload_id)
        .map_err(|error| FilesystemError::bad_input(error.to_string()))?;
    if initial.purpose != crate::file_upload::FileUploadPurpose::WorkspaceUpload {
        return Err(FilesystemError::bad_input("file upload purpose mismatch"));
    }
    if initial.size_bytes > max_bytes {
        return Err(FilesystemError::bad_input(format!(
            "file too large: {} bytes (max {} bytes)",
            initial.size_bytes, max_bytes
        )));
    }
    let safe = sanitize_name(file_name.unwrap_or(&initial.file_name));
    if safe.is_empty() {
        return Err(FilesystemError::bad_input("invalid file name"));
    }

    let dir_abs = scope.resolve_new(dir_rel)?;
    let _guard = lock_workspace_mutations();
    std::fs::create_dir_all(&dir_abs)
        .map_err(|error| FilesystemError::internal(format!("create dir: {error}")))?;
    let dir_clean = dir_rel.trim().trim_start_matches('/').trim_end_matches('/');
    let target_rel = if dir_clean.is_empty() {
        safe.clone()
    } else {
        format!("{dir_clean}/{safe}")
    };
    let mut target = scope.resolve_new(&target_rel)?;
    if target.exists() && !overwrite {
        target = dedupe_path(&target);
    }
    let staging = dir_abs.join(format!(".{safe}.upload-{}.tmp", uuid::Uuid::new_v4()));
    let lease = crate::file_upload::copy_completed_upload_create_new(
        upload_id,
        crate::file_upload::FileUploadPurpose::WorkspaceUpload,
        &staging,
    )
    .map_err(|error| FilesystemError::bad_input(error.to_string()))?;
    let publish = (|| -> Result<()> {
        let metadata = std::fs::symlink_metadata(&staging)
            .map_err(|error| FilesystemError::internal(format!("stat upload: {error}")))?;
        let current_max = crate::config::cached_config()
            .filesystem
            .max_workspace_upload_bytes();
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != lease.size_bytes
            || metadata.len() > current_max
        {
            return Err(FilesystemError::bad_input(
                "workspace upload no longer satisfies the configured limit",
            ));
        }
        std::fs::File::open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(|error| FilesystemError::internal(format!("sync upload: {error}")))?;
        crate::platform::publish_atomic_file(&staging, &target, overwrite)
            .map_err(|error| FilesystemError::internal(format!("publish upload: {error}")))?;
        Ok(())
    })();
    if let Err(error) = publish {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    // The workspace file is already atomically published. A cleanup failure
    // must not report the domain operation as failed; the expiry sweeper will
    // remove the retained lease later.
    let _ = crate::file_upload::discard_upload(upload_id);
    Ok(UploadResult {
        rel_path: scope.rel_of(&target),
        size_bytes: lease.size_bytes,
    })
}

// ---- helpers ---------------------------------------------------------------

/// Cheap binary sniff: a NUL byte in the first 8 KiB or invalid UTF-8 there.
fn looks_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 8192];
    let n = f.read(&mut head).unwrap_or(0);
    let slice = &head[..n];
    slice.contains(&0) || std::str::from_utf8(slice).is_err() && !is_valid_utf8_prefix(slice)
}

/// A truncated read can split a multi-byte UTF-8 sequence; treat a trailing
/// incomplete sequence as still-text rather than binary.
fn is_valid_utf8_prefix(slice: &[u8]) -> bool {
    match std::str::from_utf8(slice) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}

fn sanitize_name(name: &str) -> String {
    name.trim().replace(['/', '\\', ':', '\0'], "_")
}

/// Insert a ` (N)` suffix before the extension until the path is free.
fn dedupe_path(path: &Path) -> PathBuf {
    let Some(parent) = path.parent() else {
        return path.to_path_buf();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let np = Path::new(&name);
    let stem = np
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.clone());
    let ext = np.extension().map(|e| e.to_string_lossy().to_string());
    for n in 1..=9999 {
        let candidate = match &ext {
            Some(e) => format!("{} ({}).{}", stem, n, e),
            None => format!("{} ({})", stem, n),
        };
        let p = parent.join(&candidate);
        if !p.exists() {
            return p;
        }
    }
    parent.join(format!("{}-{}", name, uuid::Uuid::new_v4()))
}

/// Extension → MIME mapping covering the formats the browser previews
/// (documents, images, common text). Drives `file_extract` dispatch and the
/// `mime` field; the HTTP raw endpoint does its own full MIME resolution.
fn mime_for_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    let m = match ext.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ppt" => "application/vnd.ms-powerpoint",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        _ => return None,
    };
    Some(m.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_text_reports_raw_hash_bom_and_mixed_line_endings() {
        let root = tempfile::tempdir().expect("tempdir");
        let bytes = b"\xef\xbb\xbfalpha\r\nbeta\ngamma\rdelta";
        std::fs::write(root.path().join("mixed.txt"), bytes).expect("write fixture");
        let scope = WorkspaceScope::from_test_root(root.path());

        let content = project_read_text(&scope, "mixed.txt").expect("read text");

        assert_eq!(content.content, "alpha\r\nbeta\ngamma\rdelta");
        assert_eq!(
            content.content_hash,
            Some(blake3::hash(bytes).to_hex().to_string())
        );
        assert!(content.is_utf8);
        assert!(content.has_utf8_bom);
        assert_eq!(content.line_ending, LineEnding::Mixed);
    }

    #[test]
    fn checked_write_saves_matching_hash_and_rejects_stale_hash() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("note.md");
        std::fs::write(&path, b"first\r\n").expect("write fixture");
        let scope = WorkspaceScope::from_test_root(root.path());
        let initial = project_read_text(&scope, "note.md").expect("read initial");

        let saved = project_write_text_checked(
            &scope,
            "note.md",
            "second\r\n",
            false,
            initial.content_hash.as_deref(),
        )
        .expect("save matching version");
        let saved_hash = match saved {
            FileWriteOutcome::Saved { content_hash, .. } => content_hash,
            FileWriteOutcome::Conflict { .. } => panic!("unexpected conflict"),
        };
        assert_eq!(saved_hash, blake3::hash(b"second\r\n").to_hex().to_string());
        assert_eq!(std::fs::read(&path).expect("read saved"), b"second\r\n");

        std::fs::write(&path, b"external change").expect("external update");
        let conflict = project_write_text_checked(
            &scope,
            "note.md",
            "would overwrite",
            false,
            Some(&saved_hash),
        )
        .expect("structured conflict");
        match conflict {
            FileWriteOutcome::Conflict {
                reason: FileWriteConflictReason::Changed,
                current_content_hash: Some(current),
            } => assert_eq!(
                current,
                blake3::hash(b"external change").to_hex().to_string()
            ),
            _ => panic!("expected changed conflict"),
        }
        assert_eq!(
            std::fs::read(&path).expect("read current"),
            b"external change"
        );
    }

    #[test]
    fn create_only_never_overwrites_and_missing_existing_file_conflicts() {
        let root = tempfile::tempdir().expect("tempdir");
        let scope = WorkspaceScope::from_test_root(root.path());

        let created =
            project_write_text_checked(&scope, "new.txt", "new", true, None).expect("create file");
        assert!(matches!(created, FileWriteOutcome::Saved { .. }));
        let exists = project_write_text_checked(&scope, "new.txt", "replace", true, None)
            .expect("existing createOnly result");
        assert!(matches!(
            exists,
            FileWriteOutcome::Conflict {
                reason: FileWriteConflictReason::Changed,
                ..
            }
        ));
        assert_eq!(
            std::fs::read(root.path().join("new.txt")).expect("read"),
            b"new"
        );

        let deleted =
            project_write_text_checked(&scope, "missing.txt", "replace", false, Some("known-hash"))
                .expect("deleted conflict");
        assert!(matches!(
            deleted,
            FileWriteOutcome::Conflict {
                reason: FileWriteConflictReason::Deleted,
                current_content_hash: None,
            }
        ));
    }

    #[test]
    fn workspace_upload_lease_claims_atomically_and_is_consumed() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let workspace = root.path().join("workspace");
            std::fs::create_dir_all(&workspace).unwrap();
            let scope = WorkspaceScope::from_test_root(&workspace);
            let lease =
                crate::file_upload::start_upload(crate::file_upload::FileUploadStartInput {
                    purpose: crate::file_upload::FileUploadPurpose::WorkspaceUpload,
                    file_name: "report.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 6,
                })
                .unwrap();
            crate::file_upload::upload_chunk(&lease.upload_id, 0, b"report").unwrap();
            crate::file_upload::complete_upload(&lease.upload_id).unwrap();

            let result = project_claim_upload(&scope, "docs", &lease.upload_id, None, false)
                .expect("claim workspace upload");
            assert_eq!(result.rel_path, "docs/report.txt");
            assert_eq!(
                std::fs::read(workspace.join(&result.rel_path)).unwrap(),
                b"report"
            );
            assert!(crate::file_upload::upload_status(&lease.upload_id).is_err());
        });
    }
}
