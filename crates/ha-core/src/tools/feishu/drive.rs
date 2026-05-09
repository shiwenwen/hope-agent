//! Feishu drive (云盘 / Lark Drive) — 3 LLM tools.
//!
//! - [`feishu_drive_list_files`]    — list a folder's contents (paginated)
//! - [`feishu_drive_upload_media`]  — upload a local file (≤ 20 MB) to drive
//! - [`feishu_drive_download_media`]— download a drive file by token to a local path
//!
//! Local-path tools (upload / download) declare a `path` argument that the
//! permission engine recognizes via [`permission::rules::extract_path_arg`]
//! — protected-path patterns (`~/.ssh/...`, `*.pem`, etc.) trigger the
//! same approval gate as `read` / `write`. Required Feishu app scope:
//! `drive:drive` (or `drive:drive.read` for list / download).

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::channel::feishu::api_drive::DRIVE_UPLOAD_MAX_BYTES;
use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::resolve_feishu_api;

pub const TOOL_DRIVE_LIST_FILES: &str = "feishu_drive_list_files";
pub const TOOL_DRIVE_UPLOAD_MEDIA: &str = "feishu_drive_upload_media";
pub const TOOL_DRIVE_DOWNLOAD_MEDIA: &str = "feishu_drive_download_media";

const CONFIG_HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable drive tools.";

fn account_param() -> Value {
    json!({
        "type": "string",
        "description": "Feishu channel account ID. Required only when more than one Feishu account is configured; otherwise the only configured account is used."
    })
}

fn configured_tier() -> ToolTier {
    ToolTier::Configured {
        default_for_main: false,
        default_for_others: false,
        default_deferred: true,
        config_hint: CONFIG_HINT,
    }
}

// ── Tool definitions ────────────────────────────────────────────

pub fn list_files_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DRIVE_LIST_FILES.into(),
        description:
            "List the contents of a Feishu (Lark) drive folder. Returns files (regular + docx + \
             sheet + bitable + folder + shortcut). Omit `folder_token` to list the user's drive \
             root. Required Feishu app scope: `drive:drive.read` or `drive:drive`."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "folder_token": {
                    "type": "string",
                    "description": "Folder token to list. Omit for the user's drive root."
                },
                "page_token": {"type": "string"},
                "page_size":  {"type": "integer", "description": "Items per page, default 20."},
                "account": account_param(),
            },
            "additionalProperties": false
        }),
    }
}

pub fn upload_media_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DRIVE_UPLOAD_MEDIA.into(),
        description:
            "Upload a local file (≤ 20 MB) to a Feishu (Lark) drive folder. Returns the new \
             `file_token` which you can later pass to `feishu_drive_download_media`. Files larger \
             than 20 MB need the segmented upload v2 protocol (deferred to a future release). \
             Required Feishu app scope: `drive:drive`. \
             Note: `path` MUST be an absolute path on this machine; protected-path policies \
             (`~/.ssh/`, `*.pem`, etc.) apply just like the `read` tool."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path of the local file to upload."
                },
                "folder_token": {
                    "type": "string",
                    "description": "Destination folder token. Use the user's drive root token if omitting; many tenants require an explicit folder."
                },
                "file_name": {
                    "type": "string",
                    "description": "Filename to register on Feishu's side. Defaults to the basename of `path`."
                },
                "mime": {
                    "type": "string",
                    "description": "Optional MIME type override (e.g. `image/png`). Defaults to `application/octet-stream`."
                },
                "account": account_param(),
            },
            "required": ["path", "folder_token"],
            "additionalProperties": false
        }),
    }
}

pub fn download_media_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DRIVE_DOWNLOAD_MEDIA.into(),
        description:
            "Download a Feishu (Lark) drive media file by its `file_token` and save it to a \
             local absolute path. Returns the path written. The destination must be an absolute \
             path; the parent directory must already exist. Required Feishu app scope: \
             `drive:drive.read` or `drive:drive`. Protected-path policies apply just like the \
             `write` tool."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "file_token": {
                    "type": "string",
                    "description": "The drive media `file_token` to download."
                },
                "path": {
                    "type": "string",
                    "description": "Absolute local path to write the downloaded bytes to. Parent directory must exist."
                },
                "account": account_param(),
            },
            "required": ["file_token", "path"],
            "additionalProperties": false
        }),
    }
}

// ── Argument helpers ────────────────────────────────────────────

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn get_required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    get_str(args, key).ok_or_else(|| anyhow!("`{}` is required and must be a string", key))
}

fn get_u32(args: &Value, key: &str) -> Result<Option<u32>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .map(Some)
            .ok_or_else(|| anyhow!("`{}` must be a non-negative integer fitting in u32", key)),
        _ => Err(anyhow!("`{}` must be an integer", key)),
    }
}

/// Require a tilde-expanded absolute path. Mirrors what `tools/read.rs`,
/// `tools/write.rs` etc. expect — keeps the protected-path engine working
/// (it normalizes lexically) and avoids ambiguous "where did the file go"
/// behavior when the LLM passes something relative.
fn require_absolute_path(raw: &str) -> Result<PathBuf> {
    let expanded = crate::tools::expand_tilde(raw);
    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        return Err(anyhow!(
            "`path` must be an absolute path (got: {:?}). Use a path starting with `/` (Unix) or `C:\\` (Windows), or a `~`-prefixed path that resolves to an absolute path.",
            raw
        ));
    }
    Ok(path)
}

// ── Execute fns ─────────────────────────────────────────────────

pub(crate) async fn execute_list_files(args: &Value) -> Result<String> {
    let folder_token = get_str(args, "folder_token");
    let page_token = get_str(args, "page_token");
    let page_size = get_u32(args, "page_size")?;
    let account = get_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let page = api
        .drive_list_files(folder_token, page_token, page_size)
        .await?;
    Ok(serde_json::to_string(&page)?)
}

pub(crate) async fn execute_upload_media(args: &Value) -> Result<String> {
    let path_str = get_required_str(args, "path")?;
    let folder_token = get_required_str(args, "folder_token")?;
    let file_name_arg = get_str(args, "file_name");
    let mime = get_str(args, "mime");
    let account = get_str(args, "account");

    let local_path = require_absolute_path(path_str)?;
    let metadata = tokio::fs::metadata(&local_path)
        .await
        .with_context(|| format!("failed to stat {:?}", local_path))?;
    if !metadata.is_file() {
        return Err(anyhow!(
            "`path` must point to a regular file ({:?} is not a file)",
            local_path
        ));
    }
    let size = metadata.len();
    if size == 0 {
        return Err(anyhow!("file at {:?} is empty", local_path));
    }
    if size > DRIVE_UPLOAD_MAX_BYTES {
        return Err(anyhow!(
            "file size {} bytes (path={:?}) exceeds 20 MB limit. Files >20 MB require segmented upload v2, deferred.",
            size,
            local_path
        ));
    }
    let bytes = tokio::fs::read(&local_path)
        .await
        .with_context(|| format!("failed to read {:?}", local_path))?;

    let file_name = file_name_arg.map(str::to_string).unwrap_or_else(|| {
        local_path
            .file_name()
            .and_then(|os| os.to_str())
            .unwrap_or("upload.bin")
            .to_string()
    });

    let api = resolve_feishu_api(account).await?;
    let result = api
        .drive_upload_media(&file_name, "explorer", folder_token, bytes, mime)
        .await?;
    Ok(serde_json::to_string(&result)?)
}

pub(crate) async fn execute_download_media(args: &Value) -> Result<String> {
    let file_token = get_required_str(args, "file_token")?;
    let path_str = get_required_str(args, "path")?;
    let account = get_str(args, "account");

    let local_path = require_absolute_path(path_str)?;
    if let Some(parent) = local_path.parent() {
        if !is_existing_dir(parent).await {
            return Err(anyhow!(
                "parent directory does not exist: {:?}. Create it first or pick a different `path`.",
                parent
            ));
        }
    }

    let api = resolve_feishu_api(account).await?;
    let bytes = api.drive_download_media(file_token).await?;
    tokio::fs::write(&local_path, &bytes)
        .await
        .with_context(|| format!("failed to write {} bytes to {:?}", bytes.len(), local_path))?;

    Ok(serde_json::to_string(&serde_json::json!({
        "path": local_path.to_string_lossy(),
        "bytes_written": bytes.len(),
    }))?)
}

async fn is_existing_dir(p: &Path) -> bool {
    tokio::fs::metadata(p)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_have_expected_names() {
        assert_eq!(list_files_tool().name, TOOL_DRIVE_LIST_FILES);
        assert_eq!(upload_media_tool().name, TOOL_DRIVE_UPLOAD_MEDIA);
        assert_eq!(download_media_tool().name, TOOL_DRIVE_DOWNLOAD_MEDIA);
    }

    #[test]
    fn definitions_are_tier_configured_off_by_default() {
        for def in [
            list_files_tool(),
            upload_media_tool(),
            download_media_tool(),
        ] {
            match def.tier {
                ToolTier::Configured {
                    default_for_main,
                    default_for_others,
                    config_hint,
                    ..
                } => {
                    assert!(!default_for_main, "{}", def.name);
                    assert!(!default_for_others, "{}", def.name);
                    assert!(config_hint.contains("Feishu"));
                }
                _ => panic!("{} must be Tier 3 Configured", def.name),
            }
        }
    }

    #[tokio::test]
    async fn upload_requires_path_and_folder_token() {
        let err = execute_upload_media(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("path"), "{}", err);

        let err = execute_upload_media(&json!({"path": "/tmp/x"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("folder_token"), "{}", err);
    }

    #[tokio::test]
    async fn upload_rejects_relative_path() {
        let err = execute_upload_media(&json!({
            "path": "relative/file.txt",
            "folder_token": "f1"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("absolute"), "{}", err);
    }

    #[tokio::test]
    async fn upload_rejects_missing_file() {
        let err = execute_upload_media(&json!({
            "path": "/nonexistent/path/should-not-exist.bin",
            "folder_token": "f1"
        }))
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("stat") || err.to_string().contains("No such"),
            "{}",
            err
        );
    }

    #[tokio::test]
    async fn upload_rejects_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.bin");
        tokio::fs::write(&path, b"").await.unwrap();
        let err = execute_upload_media(&json!({
            "path": path.to_string_lossy(),
            "folder_token": "f1"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("empty"), "{}", err);
    }

    #[tokio::test]
    async fn download_requires_file_token_and_path() {
        let err = execute_download_media(&json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("file_token"), "{}", err);

        let err = execute_download_media(&json!({"file_token": "boxcnA"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("path"), "{}", err);
    }

    #[tokio::test]
    async fn download_rejects_relative_path() {
        let err = execute_download_media(&json!({
            "file_token": "boxcnA",
            "path": "relative/dst.bin"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("absolute"), "{}", err);
    }

    #[tokio::test]
    async fn download_rejects_missing_parent_dir() {
        let err = execute_download_media(&json!({
            "file_token": "boxcnA",
            "path": "/definitely/missing/parent/dst.bin"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("parent"), "{}", err);
    }

    #[test]
    fn require_absolute_path_expands_tilde() {
        // `~/foo` should expand to an absolute path under HOME (when HOME set).
        let result = require_absolute_path("~/foo");
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            assert!(result.is_ok(), "{:?}", result);
            let p = result.unwrap();
            assert!(p.is_absolute(), "{:?} should be absolute", p);
        }
    }
}
