//! `project_read_file` — read a file shared inside the current session's
//! project. The tool only works when the active session is attached to a
//! project; it cannot escape that project's files directory.

use anyhow::Result;
use serde_json::Value;

use super::read::read_text_page;
use super::ToolExecContext;

const DEFAULT_LIMIT_LINES: usize = 2000;
const MAX_LIMIT_LINES: usize = 10_000;

pub(crate) async fn tool_project_read_file(
    args: &Value,
    ctx: &ToolExecContext,
) -> Result<String> {
    // 1. Resolve the current session → project.
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("project_read_file requires an active session"))?;

    let session_db = crate::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("Session database not initialized"))?;
    let session = session_db
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

    let project_id = session.project_id.ok_or_else(|| {
        anyhow::anyhow!(
            "The current session is not attached to any project. \
             Use the standard `read` tool for files outside a project."
        )
    })?;

    // 2. Resolve which project file was requested. Prefer `file_id`, then
    // fall back to `name` lookup inside the current project.
    let project_db = crate::get_project_db()
        .ok_or_else(|| anyhow::anyhow!("Project database not initialized"))?;

    let file = if let Some(file_id) = args.get("file_id").and_then(|v| v.as_str()) {
        project_db.get_file(&project_id, file_id)?
    } else if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        project_db.find_file_by_name(&project_id, name)?
    } else {
        return Err(anyhow::anyhow!(
            "project_read_file requires either 'file_id' or 'name'"
        ));
    };

    let file = file.ok_or_else(|| {
        anyhow::anyhow!(
            "No project file matched the requested identifier in project '{}'",
            project_id
        )
    })?;

    // 3. Locate extracted text content on disk. Binary files without an
    // extracted sibling are rejected with a clear message.
    let ext_rel = file.extracted_path.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "File '{}' has no extractable text (likely binary). \
             Use a different tool or view it via the UI.",
            file.name
        )
    })?;

    let base = crate::paths::projects_dir()?;
    let full_path = base.join(ext_rel);

    // Defense-in-depth: ensure the resolved path is still inside the project's
    // extracted directory. Rejects any `..` tricks hiding in the stored rel path.
    let allowed_root = crate::paths::project_extracted_dir(&project_id)?;
    let canonical_full = std::fs::canonicalize(&full_path).unwrap_or(full_path.clone());
    let canonical_root =
        std::fs::canonicalize(&allowed_root).unwrap_or(allowed_root.clone());
    if !canonical_full.starts_with(&canonical_root) {
        return Err(anyhow::anyhow!(
            "Refusing to read outside the project's extracted directory"
        ));
    }

    let content = tokio::fs::read_to_string(&full_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", file.name, e))?;

    // 4. Pagination — reuse `read.rs` helper so line format matches.
    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v.max(1) as usize)
        .unwrap_or(1); // 1-based

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).min(MAX_LIMIT_LINES))
        .unwrap_or(DEFAULT_LIMIT_LINES);

    let lines: Vec<&str> = content.lines().collect();
    let (body, lines_read, truncated, total_lines) =
        read_text_page(&lines, offset - 1, limit);

    let header = format!(
        "Project file: {} (project: {}, {} total lines, reading from line {})\n\
         ────────────────────────────────────────\n",
        file.name, project_id, total_lines, offset
    );

    let mut result = String::with_capacity(header.len() + body.len() + 96);
    result.push_str(&header);
    result.push_str(&body);
    if truncated {
        result.push_str(&format!(
            "\n[Read {} lines ({}-{} of {}). Use offset={} to continue reading.]\n",
            lines_read,
            offset,
            offset + lines_read - 1,
            total_lines,
            offset + lines_read
        ));
    }
    Ok(result)
}
