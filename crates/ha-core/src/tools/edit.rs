use anyhow::Result;
use serde_json::{json, Value};

use super::diff_util::{compute_line_delta, detect_language, truncate_for_metadata};
use super::extract_string_param;

pub(crate) async fn tool_edit(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    // Accept path aliases: path, file_path
    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let path = ctx.resolve_path(raw_path);

    // Accept old_text aliases: old_text, oldText, old_string
    let old_text = args
        .get("old_text")
        .or_else(|| args.get("oldText"))
        .or_else(|| args.get("old_string"))
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'old_text' parameter"))?;

    // Accept new_text aliases: new_text, newText, new_string (empty string allowed for deletion)
    let new_text = args
        .get("new_text")
        .or_else(|| args.get("newText"))
        .or_else(|| args.get("new_string"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(""); // empty = deletion

    app_info!("tool", "edit", "Editing file: {}", path);

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    let count = content.matches(old_text).count();
    if count == 0 {
        // Post-write recovery: the file may already contain new_text from a previous
        // edit that threw after writing (e.g. interrupted tool call). If new_text is
        // present and old_text is absent, treat as success rather than false failure.
        if !new_text.is_empty() && content.contains(new_text) {
            app_info!(
                "tool",
                "edit",
                "Post-write recovery: old_text absent but new_text already present in '{}'",
                path
            );
            return Ok(format!(
                "Successfully edited {} (recovered — replacement already applied)",
                path
            ));
        }
        return Err(anyhow::anyhow!(
            "old_text not found in '{}'. Make sure the text matches exactly (including whitespace and indentation).",
            path
        ));
    }
    if count > 1 {
        return Err(anyhow::anyhow!(
            "old_text found {} times in '{}'. Please provide more context to make the match unique.",
            count,
            path
        ));
    }

    let new_content = content.replacen(old_text, new_text, 1);

    let write_result = tokio::fs::write(&path, &new_content).await;
    if write_result.is_ok() {
        emit_file_change_metadata(ctx, &path, &content, &new_content).await;
    }

    if let Err(ref e) = write_result {
        // Post-write recovery: if write returned an error but the file on disk actually
        // contains the correct content, treat as success. This handles edge cases where
        // data was flushed but the OS reported an error (e.g. network mounts, interrupted fsync).
        if let Ok(on_disk) = tokio::fs::read_to_string(&path).await {
            let has_new = new_text.is_empty() || on_disk.contains(new_text);
            let still_has_old = !old_text.is_empty() && on_disk.contains(old_text);
            if has_new && !still_has_old {
                app_warn!(
                    "tool",
                    "edit",
                    "Post-write recovery: write error but file correct in '{}': {}",
                    path,
                    e
                );
                return Ok(format!(
                    "Successfully edited {} (recovered after write error)",
                    path
                ));
            }
        }
        return Err(anyhow::anyhow!("Failed to write file '{}': {}", path, e));
    }

    Ok(format!(
        "Successfully edited {} (replaced 1 occurrence)",
        path
    ))
}

async fn emit_file_change_metadata(
    ctx: &super::ToolExecContext,
    path: &str,
    before: &str,
    after: &str,
) {
    if ctx.metadata_sink.is_none() {
        return;
    }
    let (added, removed) = compute_line_delta(before, after);
    let (before_t, before_trunc) = truncate_for_metadata(before);
    let (after_t, after_trunc) = truncate_for_metadata(after);
    ctx.emit_metadata(json!({
        "kind": "file_change",
        "path": path,
        "action": "edit",
        "linesAdded": added,
        "linesRemoved": removed,
        "before": before_t,
        "after": after_t,
        "language": detect_language(path),
        "truncated": before_trunc || after_trunc,
    }))
    .await;
}
