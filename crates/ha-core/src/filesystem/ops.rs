//! Working-directory file operations backing the project file browser.
//!
//! Every function takes a [`WorkspaceScope`] and operates on paths relative to
//! its root; the scope enforces containment, so these functions never see an
//! escaping path. DTOs serialize as camelCase to match the transport layer.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::workspace::WorkspaceScope;
use super::{FilesystemError, Result};

/// Files larger than this are not inlined as text — the browser falls back to
/// the raw/preview endpoint or a binary placeholder.
const MAX_TEXT_PREVIEW_BYTES: u64 = 5 * 1024 * 1024;

/// Documents are fully buffered + parsed (and PDFs rasterized) by
/// `file_extract`, so cap the input to keep a single preview from blowing up
/// memory.
const MAX_EXTRACT_BYTES: u64 = 50 * 1024 * 1024;

/// Per-directory entry cap (same intent as `MAX_LIST_ENTRIES` in `mod.rs`).
const MAX_LIST_ENTRIES: usize = 5000;

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

    if size_bytes > MAX_TEXT_PREVIEW_BYTES || looks_binary(abs) {
        return Ok(FileTextContent {
            rel_path,
            content: String::new(),
            is_binary: true,
            mime,
            total_lines: 0,
            size_bytes,
            truncated: size_bytes > MAX_TEXT_PREVIEW_BYTES,
        });
    }

    let bytes =
        std::fs::read(abs).map_err(|e| FilesystemError::internal(format!("read: {}", e)))?;
    let content = String::from_utf8_lossy(&bytes).to_string();
    let total_lines = content.lines().count();
    Ok(FileTextContent {
        rel_path,
        content,
        is_binary: false,
        mime,
        total_lines,
        size_bytes,
        truncated: false,
    })
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
    if meta.len() > MAX_EXTRACT_BYTES {
        return Err(FilesystemError::bad_input(format!(
            "file too large to preview: {} bytes (max {} bytes)",
            meta.len(),
            MAX_EXTRACT_BYTES
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
/// `create_only` makes the call fail if the target already exists.
pub fn project_write_text(
    scope: &WorkspaceScope,
    rel: &str,
    content: &str,
    create_only: bool,
) -> Result<WriteResult> {
    let abs = scope.resolve_new(rel)?;
    if create_only && abs.exists() {
        return Err(FilesystemError::bad_input("file already exists"));
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| FilesystemError::internal(format!("create parent: {}", e)))?;
    }
    std::fs::write(&abs, content.as_bytes())
        .map_err(|e| FilesystemError::internal(format!("write: {}", e)))?;
    Ok(WriteResult {
        rel_path: scope.rel_of(&abs),
        size_bytes: content.len() as u64,
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
    if data.len() > crate::project::MAX_PROJECT_FILE_BYTES {
        return Err(FilesystemError::bad_input(format!(
            "file too large: {} bytes (max {} bytes)",
            data.len(),
            crate::project::MAX_PROJECT_FILE_BYTES
        )));
    }
    let safe = sanitize_name(file_name);
    if safe.is_empty() {
        return Err(FilesystemError::bad_input("invalid file name"));
    }

    let dir_abs = scope.resolve_new(dir_rel)?;
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
    std::fs::write(&abs, data).map_err(|e| FilesystemError::internal(format!("write: {}", e)))?;
    Ok(UploadResult {
        rel_path: scope.rel_of(&abs),
        size_bytes: data.len() as u64,
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
