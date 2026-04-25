use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use super::extract_string_param;

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate);
        }
        current = candidate.parent();
    }
    None
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    let Ok(canonical_root) = root.canonicalize() else {
        return false;
    };
    nearest_existing_ancestor(path)
        .and_then(|ancestor| ancestor.canonicalize().ok())
        .map(|ancestor| ancestor.starts_with(canonical_root))
        .unwrap_or(false)
}

pub(crate) async fn tool_write_file(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    // Accept both "path" and "file_path", with structured content support
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let path = ctx.resolve_path(raw_path);

    // Validate path: disallow writing outside the selected session working
    // directory or, when no session directory is set, outside user home.
    let resolved = std::path::Path::new(&path);
    if let Some(parent) = resolved.parent() {
        let session_root = ctx.session_working_dir.as_deref().map(Path::new);
        let home_root = dirs::home_dir();
        let allowed = session_root
            .map(|root| path_is_under_root(parent, root))
            .unwrap_or(false)
            || home_root
                .as_deref()
                .map(|root| path_is_under_root(parent, root))
                .unwrap_or(false);

        if !allowed {
            return Err(anyhow::anyhow!(
                "Refusing to write outside the session working directory or home directory: {}",
                path
            ));
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

#[cfg(test)]
mod tests {
    use super::tool_write_file;
    use crate::tools::ToolExecContext;
    use serde_json::json;

    #[cfg(unix)]
    #[tokio::test]
    async fn write_allows_relative_paths_under_session_working_dir_outside_home() {
        let dir = std::path::Path::new("/tmp").join(format!(
            "ha-session-working-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create tempdir outside user home");
        let ctx = ToolExecContext {
            session_working_dir: Some(dir.to_string_lossy().to_string()),
            ..ToolExecContext::default()
        };

        tool_write_file(&json!({"path": "note.txt", "content": "hello"}), &ctx)
            .await
            .expect("write relative path inside session working dir");

        let written = tokio::fs::read_to_string(dir.join("note.txt"))
            .await
            .expect("read written file");
        assert_eq!(written, "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
