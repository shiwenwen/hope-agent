use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use super::extract_string_param;

// ── Apply Patch Tool ──────────────────────────────────────────────

/// Parsed hunk kinds.
#[derive(Debug)]
enum PatchHunkKind {
    Add {
        path: String,
        contents: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        chunks: Vec<UpdateChunk>,
        move_to: Option<String>,
    },
}

/// A chunk within an Update hunk: context lines + old/new replacements.
#[derive(Debug)]
struct UpdateChunk {
    context: Vec<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

/// Parse a patch text into hunks.
fn parse_patch(input: &str) -> Result<Vec<PatchHunkKind>> {
    let lines: Vec<&str> = input.lines().collect();
    if lines.is_empty() {
        return Err(anyhow::anyhow!("Invalid patch: input is empty."));
    }

    // Find *** Begin Patch / *** End Patch boundaries (lenient: skip heredoc wrappers)
    let start = lines
        .iter()
        .position(|l| l.trim() == "*** Begin Patch")
        .ok_or_else(|| anyhow::anyhow!("The first line of the patch must be '*** Begin Patch'"))?;
    let end = lines
        .iter()
        .rposition(|l| l.trim() == "*** End Patch")
        .ok_or_else(|| anyhow::anyhow!("The last line of the patch must be '*** End Patch'"))?;

    if start >= end {
        return Err(anyhow::anyhow!(
            "Invalid patch: Begin Patch must come before End Patch"
        ));
    }

    let body = &lines[start + 1..end];
    let mut hunks = Vec::new();
    let mut i = 0;

    while i < body.len() {
        let line = body[i].trim();

        if line.is_empty() {
            i += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let path = path.trim().to_string();
            let mut contents = String::new();
            i += 1;
            while i < body.len() && !body[i].trim().starts_with("*** ") {
                let l = body[i];
                if let Some(stripped) = l.strip_prefix('+') {
                    contents.push_str(stripped);
                } else {
                    contents.push_str(l);
                }
                contents.push('\n');
                i += 1;
            }
            hunks.push(PatchHunkKind::Add { path, contents });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            hunks.push(PatchHunkKind::Delete {
                path: path.trim().to_string(),
            });
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = path.trim().to_string();
            let mut chunks = Vec::new();
            let mut move_to: Option<String> = None;
            i += 1;

            let mut current_context: Vec<String> = Vec::new();
            let mut current_old: Vec<String> = Vec::new();
            let mut current_new: Vec<String> = Vec::new();
            let mut in_change = false;

            while i < body.len() {
                let l = body[i];
                let trimmed = l.trim();

                // Check for next hunk boundary (but not End of File / Move to)
                if trimmed.starts_with("*** ")
                    && trimmed != "*** End of File"
                    && !trimmed.starts_with("*** Move to: ")
                {
                    break;
                }

                if trimmed == "*** End of File" {
                    if in_change || !current_context.is_empty() {
                        chunks.push(UpdateChunk {
                            context: std::mem::take(&mut current_context),
                            old_lines: std::mem::take(&mut current_old),
                            new_lines: std::mem::take(&mut current_new),
                        });
                        in_change = false;
                    }
                    i += 1;
                    continue;
                }

                if let Some(mp) = trimmed.strip_prefix("*** Move to: ") {
                    move_to = Some(mp.trim().to_string());
                    i += 1;
                    continue;
                }

                if trimmed.starts_with("@@") {
                    if in_change || !current_context.is_empty() {
                        chunks.push(UpdateChunk {
                            context: std::mem::take(&mut current_context),
                            old_lines: std::mem::take(&mut current_old),
                            new_lines: std::mem::take(&mut current_new),
                        });
                        in_change = false;
                    }
                    let ctx = trimmed.strip_prefix("@@").unwrap().trim();
                    if !ctx.is_empty() {
                        current_context.push(ctx.to_string());
                    }
                    i += 1;
                    continue;
                }

                if let Some(old) = l.strip_prefix('-') {
                    in_change = true;
                    current_old.push(old.to_string());
                    i += 1;
                } else if let Some(new_line) = l.strip_prefix('+') {
                    in_change = true;
                    current_new.push(new_line.to_string());
                    i += 1;
                } else {
                    if in_change {
                        chunks.push(UpdateChunk {
                            context: std::mem::take(&mut current_context),
                            old_lines: std::mem::take(&mut current_old),
                            new_lines: std::mem::take(&mut current_new),
                        });
                        in_change = false;
                    }
                    let ctx_line = l.strip_prefix(' ').unwrap_or(l);
                    current_context.push(ctx_line.to_string());
                    i += 1;
                }
            }

            // Flush remaining chunk
            if in_change || !current_old.is_empty() || !current_new.is_empty() {
                chunks.push(UpdateChunk {
                    context: std::mem::take(&mut current_context),
                    old_lines: std::mem::take(&mut current_old),
                    new_lines: std::mem::take(&mut current_new),
                });
            }

            hunks.push(PatchHunkKind::Update {
                path,
                chunks,
                move_to,
            });
        } else {
            i += 1;
        }
    }

    Ok(hunks)
}

/// Find a sequence of lines in file_lines using fuzzy matching (3-pass).
fn seek_sequence(file_lines: &[&str], needle: &[String], start_from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start_from);
    }
    let len = needle.len();
    if len > file_lines.len() {
        return None;
    }

    let max_i = file_lines.len() - len;

    // Helper: search range with a comparator
    let search = |cmp: &dyn Fn(&str, &str) -> bool| -> Option<usize> {
        // Search forward from start_from first
        for i in start_from..=max_i {
            if (0..len).all(|j| cmp(file_lines[i + j], &needle[j])) {
                return Some(i);
            }
        }
        // Then search before start_from
        for i in 0..start_from.min(max_i + 1) {
            if (0..len).all(|j| cmp(file_lines[i + j], &needle[j])) {
                return Some(i);
            }
        }
        None
    };

    // Pass 1: exact
    if let Some(pos) = search(&|a: &str, b: &str| a == b) {
        return Some(pos);
    }
    // Pass 2: trimmed end
    if let Some(pos) = search(&|a: &str, b: &str| a.trim_end() == b.trim_end()) {
        return Some(pos);
    }
    // Pass 3: fully trimmed
    search(&|a: &str, b: &str| a.trim() == b.trim())
}

/// Apply update chunks to file content.
fn apply_update_hunks(content: &str, path: &str, chunks: &[UpdateChunk]) -> Result<String> {
    let mut file_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut cursor: usize = 0;

    for chunk in chunks {
        let file_refs: Vec<&str> = file_lines.iter().map(|s| s.as_str()).collect();

        // Find position using context lines
        if !chunk.context.is_empty() {
            match seek_sequence(&file_refs, &chunk.context, cursor) {
                Some(pos) => cursor = pos + chunk.context.len(),
                None => {
                    return Err(anyhow::anyhow!(
                        "Failed to find context in {}: '{}'",
                        path,
                        chunk.context.first().unwrap_or(&String::new())
                    ));
                }
            }
        }

        // Apply old→new replacement
        if !chunk.old_lines.is_empty() {
            let file_refs: Vec<&str> = file_lines.iter().map(|s| s.as_str()).collect();
            match seek_sequence(&file_refs, &chunk.old_lines, cursor) {
                Some(pos) => {
                    file_lines.splice(
                        pos..pos + chunk.old_lines.len(),
                        chunk.new_lines.iter().cloned(),
                    );
                    cursor = pos + chunk.new_lines.len();
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "Failed to find expected lines in {}: '{}'",
                        path,
                        chunk.old_lines.first().unwrap_or(&String::new())
                    ));
                }
            }
        } else if !chunk.new_lines.is_empty() {
            // Insert-only (no old lines)
            for (j, new_line) in chunk.new_lines.iter().enumerate() {
                file_lines.insert(cursor + j, new_line.clone());
            }
            cursor += chunk.new_lines.len();
        }
    }

    let mut result = file_lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

pub(crate) async fn tool_apply_patch(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    let input = args
        .get("input")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'input' parameter"))?;

    if input.trim().is_empty() {
        return Err(anyhow::anyhow!("Provide a patch input."));
    }

    app_info!(
        "tool",
        "apply_patch",
        "Applying patch ({} chars)",
        input.len()
    );

    let hunks = parse_patch(input)?;
    if hunks.is_empty() {
        return Err(anyhow::anyhow!("No files were modified."));
    }

    let mut added: Vec<String> = Vec::new();
    let mut modified: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();

    for hunk in &hunks {
        match hunk {
            PatchHunkKind::Add { path, contents } => {
                let resolved_path = ctx.resolve_path(path);
                let p = Path::new(&resolved_path);
                if let Some(parent) = p.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        anyhow::anyhow!("Failed to create directories for '{}': {}", path, e)
                    })?;
                }
                tokio::fs::write(&resolved_path, contents)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to write new file '{}': {}", path, e))?;
                added.push(resolved_path);
            }
            PatchHunkKind::Delete { path } => {
                let resolved_path = ctx.resolve_path(path);
                tokio::fs::remove_file(&resolved_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to delete file '{}': {}", path, e))?;
                deleted.push(resolved_path);
            }
            PatchHunkKind::Update {
                path,
                chunks,
                move_to,
            } => {
                let resolved_path = ctx.resolve_path(path);
                let content = tokio::fs::read_to_string(&resolved_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

                let new_content = apply_update_hunks(&content, path, chunks)?;

                if let Some(new_path) = move_to {
                    let resolved_new_path = ctx.resolve_path(new_path);
                    let np = Path::new(&resolved_new_path);
                    if let Some(parent) = np.parent() {
                        tokio::fs::create_dir_all(parent).await.map_err(|e| {
                            anyhow::anyhow!("Failed to create dirs for '{}': {}", new_path, e)
                        })?;
                    }
                    tokio::fs::write(&resolved_new_path, &new_content)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to write '{}': {}", new_path, e))?;
                    tokio::fs::remove_file(&resolved_path).await.map_err(|e| {
                        anyhow::anyhow!("Failed to remove old file '{}': {}", path, e)
                    })?;
                    modified.push(format!("{} -> {}", resolved_path, resolved_new_path));
                } else {
                    tokio::fs::write(&resolved_path, &new_content)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to write '{}': {}", path, e))?;
                    modified.push(resolved_path);
                }
            }
        }
    }

    let mut summary_parts = Vec::new();
    if !added.is_empty() {
        summary_parts.push(format!("Added: {}", added.join(", ")));
    }
    if !modified.is_empty() {
        summary_parts.push(format!("Modified: {}", modified.join(", ")));
    }
    if !deleted.is_empty() {
        summary_parts.push(format!("Deleted: {}", deleted.join(", ")));
    }

    Ok(format!(
        "Patch applied successfully.\n{}",
        summary_parts.join("\n")
    ))
}
