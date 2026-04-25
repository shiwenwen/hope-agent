//! HTTP adapters for filesystem listing & search.
//!
//! Desktop clients use the native directory dialog for the working-dir
//! picker; HTTP/WS clients have no such affordance (browsers sandbox
//! filesystem access), so the server exposes a minimal read-only listing API.
//! Auth is handled by the existing `Authorization: Bearer` middleware —
//! anyone who can hit this endpoint already has full agent-level access to
//! the host.
//!
//! All real logic lives in `ha_core::filesystem`; this module only adapts
//! axum query params + error mapping.
use axum::extract::Query;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use ha_core::filesystem::{self, FilesystemError};

fn map_err(e: FilesystemError) -> AppError {
    if e.is_bad_input() {
        AppError::bad_request(e.message().to_string())
    } else {
        AppError::internal(e.message().to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct ListDirQuery {
    /// Absolute path to list. When omitted, the handler returns a platform
    /// default root.
    pub path: Option<String>,
}

/// `GET /api/filesystem/list-dir?path=<abs>` — list one level of a directory.
pub async fn list_dir(Query(q): Query<ListDirQuery>) -> Result<Json<Value>, AppError> {
    let requested = q.path.clone();
    let result = tokio::task::spawn_blocking(move || filesystem::list_dir(requested.as_deref()))
        .await
        .map_err(|e| AppError::internal(format!("list-dir task failed: {}", e)))?
        .map_err(map_err)?;
    Ok(Json(serde_json::to_value(result)?))
}

#[derive(Debug, Deserialize)]
pub struct SearchFilesQuery {
    pub root: String,
    pub q: String,
    pub limit: Option<usize>,
}

/// `GET /api/filesystem/search-files?root=<abs>&q=<query>&limit=50` — fuzzy
/// search files & directories under `root`.
pub async fn search_files(Query(params): Query<SearchFilesQuery>) -> Result<Json<Value>, AppError> {
    let SearchFilesQuery { root, q, limit } = params;
    let result = tokio::task::spawn_blocking(move || filesystem::search_files(&root, &q, limit))
        .await
        .map_err(|e| AppError::internal(format!("search-files task failed: {}", e)))?
        .map_err(map_err)?;
    Ok(Json(serde_json::to_value(result)?))
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
        .unwrap_or_else(|e| panic!("list_dir failed: {}", e.message));
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

    #[tokio::test]
    async fn search_files_finds_match() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join(format!(
            "ha-server-search-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("uniqueprefix_file.txt"), b"x").unwrap();

        let res = search_files(Query(SearchFilesQuery {
            root: dir.to_string_lossy().to_string(),
            q: "uniqueprefix".to_string(),
            limit: Some(10),
        }))
        .await
        .unwrap_or_else(|e| panic!("search_files failed: {}", e.message));
        let body = res.0;
        let matches = body.get("matches").and_then(|v| v.as_array()).unwrap();
        assert!(matches.iter().any(|m| m
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.contains("uniqueprefix"))
            .unwrap_or(false)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn search_files_rejects_empty_query() {
        let tmp = std::env::temp_dir().to_string_lossy().to_string();
        let res = search_files(Query(SearchFilesQuery {
            root: tmp,
            q: "  ".to_string(),
            limit: None,
        }))
        .await;
        assert!(res.is_err(), "empty query must be rejected");
    }
}
