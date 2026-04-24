//! Server-side filesystem browsing for the chat "working directory" picker.
//!
//! Desktop clients use the native directory dialog for this; HTTP/WS clients
//! have no such affordance (browsers sandbox filesystem access), so the
//! server exposes a minimal read-only listing API. Auth is handled by the
//! existing `Authorization: Bearer` middleware — anyone who can hit this
//! endpoint already has full agent-level access to the host.
//!
//! The endpoint is non-recursive and skips entries it can't stat, so a
//! permission-denied subtree doesn't fail the whole listing.
use axum::extract::Query;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::AppError;

/// Cap so huge directories (`/nix/store`, populated `node_modules`, …) don't
/// balloon memory or serialize into a multi-MB JSON response the picker can't
/// render anyway.
const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Deserialize)]
pub struct ListDirQuery {
    /// Absolute path to list. When omitted, the handler returns a platform
    /// default root (Unix: `/`, Windows: drive letters) so the first call
    /// can start from nothing.
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryDto {
    pub name: String,
    /// Absolute path of this entry — lets the client navigate without having
    /// to re-join name onto the parent (avoids separator-guessing bugs).
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    /// mtime in unix millis. `None` when the platform can't report it.
    pub modified_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListDirResponse {
    path: String,
    parent: Option<String>,
    entries: Vec<DirEntryDto>,
    /// `true` when the directory held more than `MAX_ENTRIES` children; the
    /// UI can surface a "results truncated" hint.
    truncated: bool,
}

/// `GET /api/filesystem/list-dir?path=<abs>` — list one level of a directory
/// on the server machine.
///
/// - `path` MUST be an absolute path; relative paths are rejected so a buggy
///   client can't accidentally walk the server's current-working-directory.
/// - `canonicalize` is applied so the returned `path` is symlink-free and the
///   UI sees a stable identity across subsequent navigations.
/// - Entries are sorted directories-first, then name ascending (case-insensitive).
/// - Results are capped at `MAX_ENTRIES`; `truncated=true` signals overflow.
pub async fn list_dir(Query(q): Query<ListDirQuery>) -> Result<Json<Value>, AppError> {
    let requested = q.path.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let requested = requested.map(|s| s.to_string());

    let result = tokio::task::spawn_blocking(move || list_dir_blocking(requested.as_deref()))
        .await
        .map_err(|e| AppError::internal(format!("list-dir task failed: {}", e)))??;
    Ok(Json(serde_json::to_value(result)?))
}

fn list_dir_blocking(requested: Option<&str>) -> Result<ListDirResponse, AppError> {
    let target: PathBuf = match requested {
        Some(p) => {
            let path = Path::new(p);
            if !path.is_absolute() {
                return Err(AppError::bad_request(format!(
                    "path must be absolute: {}",
                    p
                )));
            }
            path.canonicalize().map_err(|e| {
                AppError::bad_request(format!("cannot resolve path '{}': {}", path.display(), e))
            })?
        }
        None => default_root(),
    };

    if !target.is_dir() {
        return Err(AppError::bad_request(format!(
            "path is not a directory: {}",
            target.display()
        )));
    }

    let read_dir = std::fs::read_dir(&target).map_err(|e| {
        AppError::bad_request(format!(
            "cannot read directory '{}': {}",
            target.display(),
            e
        ))
    })?;

    let target_str = target.to_string_lossy().to_string();
    let parent = target
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty() && *s != target_str);

    let mut entries: Vec<DirEntryDto> = Vec::new();
    let mut truncated = false;
    for entry in read_dir {
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            ha_core::app_warn!(
                "filesystem",
                "list_dir",
                "skipping unreadable entry under {}",
                target.display()
            );
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let file_type = meta.file_type();
        // Resolve `is_dir` through the symlink so a symlink to a directory
        // shows up as browsable.
        let is_dir = if file_type.is_symlink() {
            std::fs::metadata(entry.path())
                .map(|m| m.is_dir())
                .unwrap_or(false)
        } else {
            file_type.is_dir()
        };
        let size = if !is_dir { Some(meta.len()) } else { None };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        entries.push(DirEntryDto {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().to_string_lossy().to_string(),
            is_dir,
            is_symlink: file_type.is_symlink(),
            size,
            modified_ms,
        });
    }

    entries.sort_by(|a, b| match b.is_dir.cmp(&a.is_dir) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    ha_core::app_info!(
        "filesystem",
        "list_dir",
        "path={} entries={} truncated={}",
        target.display(),
        entries.len(),
        truncated
    );

    Ok(ListDirResponse {
        path: target_str,
        parent,
        entries,
        truncated,
    })
}

#[cfg(unix)]
fn default_root() -> PathBuf {
    // `/` is always readable as a path; the caller will see at minimum the
    // top-level directories they have permission to stat.
    PathBuf::from("/")
}

#[cfg(windows)]
fn default_root() -> PathBuf {
    // Without a drive letter there's no useful "root" on Windows; fall back
    // to the user profile home, which is what the GUI picker would open.
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_dir_returns_tmp_entries() {
        let tmp = std::env::temp_dir();
        let dir_name = format!(
            "ha-server-list-dir-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let sub = tmp.join(&dir_name);
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("hello.txt");
        std::fs::write(&file, b"hi").unwrap();

        let res = list_dir(Query(ListDirQuery {
            path: Some(sub.to_string_lossy().to_string()),
        }))
        .await
        .expect("list_dir ok");
        let body = res.0;
        let entries = body.get("entries").and_then(|v| v.as_array()).unwrap();
        assert!(entries.iter().any(|e| e
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s == "hello.txt")
            .unwrap_or(false)));

        let _ = std::fs::remove_dir_all(&sub);
    }

    #[tokio::test]
    async fn list_dir_rejects_non_directory() {
        let tmp = std::env::temp_dir();
        let file = tmp.join(format!(
            "ha-server-list-dir-not-a-dir-{}.tmp",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::write(&file, b"x").unwrap();
        let res = list_dir(Query(ListDirQuery {
            path: Some(file.to_string_lossy().to_string()),
        }))
        .await;
        assert!(res.is_err(), "expected error when path is a file");
        let _ = std::fs::remove_file(&file);
    }

    #[tokio::test]
    async fn list_dir_rejects_relative_path() {
        let res = list_dir(Query(ListDirQuery {
            path: Some("relative/path".to_string()),
        }))
        .await;
        assert!(res.is_err(), "relative paths must be rejected");
    }
}
