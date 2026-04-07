use anyhow::Result;
use serde_json::Value;

use crate::memory::{self, AddResult, MemoryScope, MemorySearchQuery, MemoryType, NewMemory};

/// Tool: save_memory — persist information for future conversations.
pub(crate) async fn tool_save_memory(args: &Value) -> Result<String> {
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    let memory_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("user");

    let scope_str = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("global");

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let pinned = args
        .get("pinned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let scope = if scope_str == "agent" {
        MemoryScope::Agent {
            id: agent_id.to_string(),
        }
    } else {
        MemoryScope::Global
    };

    let entry = NewMemory {
        memory_type: MemoryType::from_str(memory_type),
        scope,
        content: content.to_string(),
        tags,
        source: "auto".to_string(),
        source_session_id: None,
        pinned,
        attachment_path: None,
        attachment_mime: None,
    };

    // Run blocking backend operations (embedding API + SQLite) on a blocking thread
    // to avoid blocking the tokio runtime.
    let memory_type = memory_type.to_string();
    let scope_str = scope_str.to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let dedup = memory::load_dedup_config();
        match backend.add_with_dedup(entry, dedup.threshold_high, dedup.threshold_merge)? {
            AddResult::Created { id } => Ok(format!(
                "Memory saved (id: {}, type: {}, scope: {})",
                id, memory_type, scope_str
            )),
            AddResult::Duplicate { existing_id, score } => Ok(format!(
                "Similar memory already exists (id: {}, similarity: {:.1}%). Not saved.",
                existing_id,
                score * 100.0
            )),
            AddResult::Updated { id } => Ok(format!(
                "Merged with existing memory (id: {}, type: {}, scope: {})",
                id, memory_type, scope_str
            )),
        }
    })
    .await??;

    Ok(result)
}

/// Tool: recall_memory — search persistent memories by keyword or semantic query.
/// Optionally also searches past conversation history (include_history=true).
pub(crate) async fn tool_recall_memory(args: &Value) -> Result<String> {
    let query_text = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?
        .to_string();

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let type_filter = args
        .get("type")
        .and_then(|v| v.as_str())
        .map(|t| vec![MemoryType::from_str(t)]);

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let include_history = args
        .get("include_history")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Run blocking backend operations (embedding API + SQLite) on a blocking thread
    // to avoid blocking the tokio runtime.
    let query_text_clone = query_text.clone();
    let agent_id_clone = agent_id.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let query = MemorySearchQuery {
            query: query_text,
            types: type_filter,
            scope: None,
            agent_id,
            limit: Some(limit),
        };

        let results = backend.search(&query)?;

        let mut output = String::new();

        if !results.is_empty() {
            output.push_str(&format!("Found {} memories:\n\n", results.len()));
            for (i, mem) in results.iter().enumerate() {
                let scope_label = match &mem.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                };
                let pin_marker = if mem.pinned { "★ " } else { "" };
                let tags_str = if mem.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", mem.tags.join(", "))
                };
                output.push_str(&format!(
                    "{}. {}(id: {}) [{}|{}]{}\n{}\n\n",
                    i + 1,
                    pin_marker,
                    mem.id,
                    mem.memory_type.as_str(),
                    scope_label,
                    tags_str,
                    mem.content,
                ));
            }
        }

        // Search conversation history if requested
        if include_history {
            if let Some(session_db) = crate::get_session_db() {
                let history_results = session_db
                    .search_messages(&query_text_clone, agent_id_clone.as_deref(), 5)
                    .unwrap_or_default();

                if !history_results.is_empty() {
                    output.push_str(&format!(
                        "\n--- Conversation History ({} matches) ---\n\n",
                        history_results.len()
                    ));
                    for (i, hit) in history_results.iter().enumerate() {
                        let session_label = hit.session_title.as_deref().unwrap_or("Untitled");
                        output.push_str(&format!(
                            "{}. [{}] {} (session: {}, {})\n{}\n\n",
                            i + 1,
                            hit.message_role,
                            hit.timestamp,
                            session_label,
                            hit.session_id,
                            hit.content_snippet,
                        ));
                    }
                }
            }
        }

        if output.is_empty() {
            return Ok("No memories or history found matching the query.".to_string());
        }

        Ok(output)
    })
    .await??;

    Ok(result)
}

/// Tool: update_memory — update an existing memory's content and/or tags.
pub(crate) async fn tool_update_memory(args: &Value) -> Result<String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?
        .to_string();

    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Run blocking backend operations on a blocking thread.
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let existing = backend.get(id)?;
        if existing.is_none() {
            return Ok(format!("Memory with id {} not found.", id));
        }

        backend.update(id, &content, &tags)?;

        Ok(format!("Memory updated (id: {}).", id))
    })
    .await??;

    Ok(result)
}

/// Tool: delete_memory — remove a memory by its ID.
pub(crate) async fn tool_delete_memory(args: &Value) -> Result<String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    // Run blocking backend operations on a blocking thread.
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        let existing = backend.get(id)?;
        if existing.is_none() {
            return Ok(format!("Memory with id {} not found.", id));
        }

        backend.delete(id)?;

        Ok(format!("Memory deleted (id: {}).", id))
    })
    .await??;

    Ok(result)
}

/// Tool: memory_get — retrieve a specific memory entry by ID with full content and metadata.
pub(crate) async fn tool_memory_get(args: &Value) -> Result<String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let backend = crate::get_memory_backend()
            .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;

        match backend.get(id)? {
            Some(mem) => {
                let scope_label = match &mem.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                };
                let tags_str = if mem.tags.is_empty() {
                    String::new()
                } else {
                    format!(" tags: [{}]", mem.tags.join(", "))
                };
                Ok(format!(
                    "Memory #{} [{}|{}]{}\nSource: {} | Created: {} | Updated: {}\n\n{}",
                    mem.id,
                    mem.memory_type.as_str(),
                    scope_label,
                    tags_str,
                    mem.source,
                    mem.created_at,
                    mem.updated_at,
                    mem.content,
                ))
            }
            None => Ok(format!("Memory with id {} not found.", id)),
        }
    })
    .await??;

    Ok(result)
}

/// Tool: update_core_memory — update the core memory file (memory.md) that is always visible
/// in the system prompt. Used for persistent rules, preferences, and standing instructions.
pub(crate) async fn tool_update_core_memory(args: &Value, agent_id: &str) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("append");

    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    // Determine file path based on scope
    let path = match scope {
        "global" => crate::paths::root_dir()?.join("memory.md"),
        _ => crate::paths::agent_dir(agent_id)?.join("memory.md"),
    };

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let action_owned = action.to_string();
    let scope_owned = scope.to_string();
    let agent_id_owned = agent_id.to_string();
    let content_owned = content.to_string();

    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        match action_owned.as_str() {
            "append" => {
                let existing = std::fs::read_to_string(&path).unwrap_or_default();
                let new_content = if existing.trim().is_empty() {
                    content_owned
                } else {
                    format!("{}\n{}", existing.trim_end(), content_owned)
                };
                std::fs::write(&path, &new_content)?;
            }
            "replace" => {
                std::fs::write(&path, &content_owned)?;
            }
            other => {
                anyhow::bail!("Invalid action: '{}'. Use 'append' or 'replace'.", other);
            }
        }

        // Emit event to notify frontend
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                "core_memory_updated",
                serde_json::json!({
                    "agentId": agent_id_owned,
                    "scope": scope_owned,
                }),
            );
        }

        Ok(format!(
            "Core memory updated (action: {}, scope: {})",
            action_owned, scope_owned
        ))
    })
    .await??;

    Ok(result)
}
