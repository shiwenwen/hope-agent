use anyhow::Result;
use serde_json::Value;

use crate::memory::{self, MemoryScope, MemoryType, NewMemory, MemorySearchQuery, AddResult};

/// Tool: save_memory — persist information for future conversations.
pub(crate) async fn tool_save_memory(args: &Value) -> Result<String> {
    let content = args.get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    let memory_type = args.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("user");

    let scope_str = args.get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("global");

    let agent_id = args.get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let tags: Vec<String> = args.get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let scope = if scope_str == "agent" {
        MemoryScope::Agent { id: agent_id.to_string() }
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
            AddResult::Created { id } => {
                Ok(format!("Memory saved (id: {}, type: {}, scope: {})", id, memory_type, scope_str))
            }
            AddResult::Duplicate { existing_id, score } => {
                Ok(format!("Similar memory already exists (id: {}, similarity: {:.1}%). Not saved.", existing_id, score * 100.0))
            }
            AddResult::Updated { id } => {
                Ok(format!("Merged with existing memory (id: {}, type: {}, scope: {})", id, memory_type, scope_str))
            }
        }
    }).await??;

    Ok(result)
}

/// Tool: recall_memory — search persistent memories by keyword or semantic query.
pub(crate) async fn tool_recall_memory(args: &Value) -> Result<String> {
    let query_text = args.get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?
        .to_string();

    let limit = args.get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    let type_filter = args.get("type")
        .and_then(|v| v.as_str())
        .map(|t| vec![MemoryType::from_str(t)]);

    let agent_id = args.get("agent_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Run blocking backend operations (embedding API + SQLite) on a blocking thread
    // to avoid blocking the tokio runtime.
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

        if results.is_empty() {
            return Ok("No memories found matching the query.".to_string());
        }

        let mut output = format!("Found {} memories:\n\n", results.len());
        for (i, mem) in results.iter().enumerate() {
            let scope_label = match &mem.scope {
                MemoryScope::Global => "global".to_string(),
                MemoryScope::Agent { id } => format!("agent:{}", id),
            };
            let tags_str = if mem.tags.is_empty() { String::new() } else { format!(" [{}]", mem.tags.join(", ")) };
            output.push_str(&format!(
                "{}. (id: {}) [{}|{}]{}\n{}\n\n",
                i + 1,
                mem.id,
                mem.memory_type.as_str(),
                scope_label,
                tags_str,
                mem.content,
            ));
        }

        Ok(output)
    }).await??;

    Ok(result)
}

/// Tool: update_memory — update an existing memory's content and/or tags.
pub(crate) async fn tool_update_memory(args: &Value) -> Result<String> {
    let id = args.get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter (integer)"))?;

    let content = args.get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?
        .to_string();

    let tags: Vec<String> = args.get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
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
    }).await??;

    Ok(result)
}

/// Tool: delete_memory — remove a memory by its ID.
pub(crate) async fn tool_delete_memory(args: &Value) -> Result<String> {
    let id = args.get("id")
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
    }).await??;

    Ok(result)
}

/// Tool: memory_get — retrieve a specific memory entry by ID with full content and metadata.
pub(crate) async fn tool_memory_get(args: &Value) -> Result<String> {
    let id = args.get("id")
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
    }).await??;

    Ok(result)
}
