use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use super::{expand_tilde, extract_string_param};

pub(crate) async fn tool_write_file(args: &Value) -> Result<String> {
    // Accept both "path" and "file_path", with structured content support
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let path = expand_tilde(raw_path);

    // Validate path: disallow writing outside user home directory
    let resolved = std::path::Path::new(&path);
    if let Some(home) = dirs::home_dir() {
        // Attempt canonicalization of parent to detect traversal
        if let Some(parent) = resolved.parent() {
            if parent.exists() {
                if let Ok(canonical_parent) = parent.canonicalize() {
                    if !canonical_parent.starts_with(&home) {
                        return Err(anyhow::anyhow!(
                            "Refusing to write outside home directory: {}",
                            path
                        ));
                    }
                }
            }
        }
    }

    // Accept structured content: plain string, {type:"text", text:"..."}, or array thereof
    let content = args
        .get("content")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    app_info!("tool", "write", "Writing file: {}", path);

    if let Some(parent) = Path::new(&path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create directories: {}", e))?;
    }

    tokio::fs::write(&path, content)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {}", path, e))?;

    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        path
    ))
}
