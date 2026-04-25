use anyhow::Result;
use serde_json::Value;

use super::extract_string_param;

/// Default max entries for ls.
const LS_DEFAULT_LIMIT: usize = 500;
/// Max output bytes for ls (50KB).
const LS_MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub(crate) async fn tool_ls(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    // Accept path aliases: path, file_path; with structured content support
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(ctx.default_path());

    let path = ctx.resolve_path(raw_path);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(LS_DEFAULT_LIMIT);

    app_info!(
        "tool",
        "ls",
        "Listing directory: {} (limit={})",
        path,
        limit
    );

    // Validate path exists and is a directory
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| anyhow::anyhow!("Path not found: {}", path))?;

    if !meta.is_dir() {
        return Err(anyhow::anyhow!("Not a directory: {}", path));
    }

    let mut entries = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read directory '{}': {}", path, e))?;

    let mut items = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip entries that cannot be stat'd
        let indicator = match entry.file_type().await {
            Ok(ft) => {
                if ft.is_dir() {
                    "/"
                } else if ft.is_symlink() {
                    "@"
                } else {
                    ""
                }
            }
            Err(_) => "", // skip type indicator if stat fails
        };
        items.push(format!("{}{}", name, indicator));
    }

    // Case-insensitive sort
    items.sort_by_key(|a| a.to_lowercase());

    if items.is_empty() {
        return Ok("(empty directory)".to_string());
    }

    // Apply entry limit and byte limit
    let mut output = String::new();
    let mut count = 0;
    let mut byte_limited = false;
    let mut entry_limited = false;

    for item in &items {
        if count >= limit {
            entry_limited = true;
            break;
        }
        let line = format!("{}\n", item);
        if output.len() + line.len() > LS_MAX_OUTPUT_BYTES {
            byte_limited = true;
            break;
        }
        output.push_str(&line);
        count += 1;
    }

    // Append truncation notice
    if entry_limited || byte_limited {
        let mut notices = Vec::new();
        if entry_limited {
            notices.push(format!(
                "{} entries limit reached. Use limit={} for more.",
                limit,
                limit * 2
            ));
        }
        if byte_limited {
            notices.push(format!(
                "{}KB output limit reached.",
                LS_MAX_OUTPUT_BYTES / 1024
            ));
        }
        output.push_str(&format!("[{}]\n", notices.join(" ")));
    }

    Ok(output.trim_end().to_string())
}
