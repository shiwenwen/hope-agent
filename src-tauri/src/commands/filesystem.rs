//! Tauri commands for filesystem listing & search.
//!
//! Thin wrappers over `ha_core::filesystem`. Used by:
//! - `WorkingDirectoryButton` directory picker (via `listServerDirectory`,
//!   though desktop normally uses the native dialog)
//! - chat-input `@` mention popper (browse dir + fuzzy search)
//!
//! Errors are flattened into `String` at the Tauri boundary; the front-end
//! shows the message directly.

use ha_core::filesystem::{self, DirListing, FileSearchResponse};

#[tauri::command]
pub async fn fs_list_dir(path: Option<String>) -> Result<DirListing, String> {
    tokio::task::spawn_blocking(move || filesystem::list_dir(path.as_deref()))
        .await
        .map_err(|e| format!("fs_list_dir task failed: {}", e))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fs_search_files(
    root: String,
    q: String,
    limit: Option<usize>,
) -> Result<FileSearchResponse, String> {
    tokio::task::spawn_blocking(move || filesystem::search_files(&root, &q, limit))
        .await
        .map_err(|e| format!("fs_search_files task failed: {}", e))?
        .map_err(|e| e.to_string())
}
