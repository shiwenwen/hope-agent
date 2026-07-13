use anyhow::Result;
use serde_json::{json, Value};

use super::ToolExecContext;

pub(crate) async fn tool_project_memory(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if ctx.incognito {
        anyhow::bail!("project_memory is unavailable in an incognito session");
    }
    if !crate::memory::load_extract_config().enabled {
        anyhow::bail!("project_memory is unavailable because long-term memory is turned off");
    }
    let project_id = ctx
        .project_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("project_memory requires a project-bound session"))?;
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list")
        .to_string();
    let args = args.clone();
    let operation_project_id = project_id.clone();
    let operation_action = action.clone();
    let bound_session_db = ctx.session_db.as_ref().map(|handle| handle.0.clone());

    let result = crate::blocking::run_blocking(move || -> Result<Value> {
        let project_exists = if let Some(session_db) = bound_session_db {
            crate::project::ProjectDB::new(session_db)
                .get(&operation_project_id)?
                .is_some()
        } else {
            crate::require_project_db()?
                .get(&operation_project_id)?
                .is_some()
        };
        if !project_exists {
            anyhow::bail!("project not found: {}", operation_project_id);
        }
        let value = match operation_action.as_str() {
            "list" => {
                let entries = crate::project::memory::list(&operation_project_id)?;
                let total = entries.len();
                let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 50) as usize;
                json!({
                    "entries": entries.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),
                    "total": total,
                    "offset": offset.min(total),
                })
            }
            "read" => {
                let file_name = required_string(&args, "fileName")?;
                let file = crate::project::memory::read(&operation_project_id, file_name)?;
                let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let max_chars = args
                    .get("maxChars")
                    .and_then(Value::as_u64)
                    .unwrap_or(12_000)
                    .clamp(1_000, 20_000) as usize;
                let total_chars = file.content.chars().count();
                let content = file
                    .content
                    .chars()
                    .skip(offset)
                    .take(max_chars)
                    .collect::<String>();
                let returned_chars = content.chars().count();
                json!({
                    "fileName": file.entry.file_name,
                    "name": file.entry.name,
                    "description": file.entry.description,
                    "memoryType": file.entry.memory_type,
                    "sizeBytes": file.entry.size_bytes,
                    "fileHash": file.file_hash,
                    "content": content,
                    "offset": offset.min(total_chars),
                    "returnedChars": returned_chars,
                    "totalChars": total_chars,
                    "truncated": offset.saturating_add(returned_chars) < total_chars,
                })
            }
            "search" => {
                let query = required_string(&args, "query")?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
                json!({ "hits": crate::project::memory::search(&operation_project_id, query, limit)? })
            }
            "write" => {
                let input = crate::project::memory::ProjectMemoryWriteInput {
                    file_name: args
                        .get("fileName")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    expected_file_hash: args
                        .get("expectedFileHash")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    name: required_string(&args, "name")?.to_string(),
                    description: required_string(&args, "description")?.to_string(),
                    memory_type: args
                        .get("memoryType")
                        .and_then(Value::as_str)
                        .unwrap_or("project")
                        .to_string(),
                    content: required_string(&args, "content")?.to_string(),
                };
                let file = crate::project::memory::write(&operation_project_id, input)?;
                json!({
                    "written": true,
                    "fileName": file.entry.file_name,
                    "name": file.entry.name,
                    "description": file.entry.description,
                    "memoryType": file.entry.memory_type,
                    "sizeBytes": file.entry.size_bytes,
                    "fileHash": file.file_hash,
                })
            }
            "delete" => {
                let file_name = required_string(&args, "fileName")?;
                let expected_file_hash = args.get("expectedFileHash").and_then(Value::as_str);
                json!({
                    "deleted": crate::project::memory::delete(
                        &operation_project_id,
                        file_name,
                        expected_file_hash,
                    )?
                })
            }
            "rebuild_index" => {
                crate::project::memory::rebuild_index(&operation_project_id)?;
                json!({
                    "rebuilt": true,
                    "topics": crate::project::memory::list(&operation_project_id)?.len(),
                })
            }
            other => anyhow::bail!("invalid project_memory action: {}", other),
        };
        Ok(value)
    })
    .await?;

    if matches!(action.as_str(), "write" | "delete" | "rebuild_index") {
        if let Some(bus) = crate::get_event_bus() {
            let _ = bus.emit(
                "project_memory:changed",
                json!({ "projectId": project_id, "action": action }),
            );
        }
    }
    serde_json::to_string_pretty(&result).map_err(Into::into)
}

fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: {}", key))
}
