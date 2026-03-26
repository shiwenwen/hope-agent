use crate::memory::{self, MemoryBackend};
use crate::slash_commands::types::{CommandAction, CommandResult};
use std::sync::Arc;

/// /remember <text> — Save a new memory.
pub fn handle_remember(
    backend: &Arc<dyn MemoryBackend>,
    args: &str,
    session_id: Option<&str>,
) -> Result<CommandResult, String> {
    let text = args.trim();
    if text.is_empty() {
        return Err("Usage: /remember <text>".into());
    }

    let entry = memory::NewMemory {
        memory_type: memory::MemoryType::User,
        scope: memory::MemoryScope::Global,
        content: text.to_string(),
        tags: vec![],
        source: "slash_command".to_string(),
        source_session_id: session_id.map(|s| s.to_string()),
        pinned: false,
    };

    let id = backend.add(entry).map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!("Memory saved (id: {})", id),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /forget <query> — Search and delete a matching memory.
pub fn handle_forget(
    backend: &Arc<dyn MemoryBackend>,
    args: &str,
) -> Result<CommandResult, String> {
    let query_text = args.trim();
    if query_text.is_empty() {
        return Err("Usage: /forget <query>".into());
    }

    let query = memory::MemorySearchQuery {
        query: query_text.to_string(),
        types: None,
        scope: None,
        agent_id: None,
        limit: Some(1),
    };

    let results = backend.search(&query).map_err(|e| e.to_string())?;
    if results.is_empty() {
        return Ok(CommandResult {
            content: format!("No memory matching `{}`", query_text),
            action: Some(CommandAction::DisplayOnly),
        });
    }

    let entry = &results[0];
    let preview = if entry.content.len() > 80 {
        format!("{}...", crate::truncate_utf8(&entry.content, 77))
    } else {
        entry.content.clone()
    };

    backend.delete(entry.id).map_err(|e| e.to_string())?;
    Ok(CommandResult {
        content: format!("Deleted memory: *{}*", preview),
        action: Some(CommandAction::DisplayOnly),
    })
}

/// /memories — List saved memories.
pub fn handle_memories(backend: &Arc<dyn MemoryBackend>) -> Result<CommandResult, String> {
    let entries = backend
        .list(None, None, 20, 0)
        .map_err(|e| e.to_string())?;

    if entries.is_empty() {
        return Ok(CommandResult {
            content: "No memories saved yet.".into(),
            action: Some(CommandAction::DisplayOnly),
        });
    }

    let mut lines = vec![format!("**Memories** ({} shown)\n", entries.len())];
    for e in &entries {
        let type_tag = format!("{:?}", e.memory_type).to_lowercase();
        let preview = if e.content.len() > 60 {
            format!("{}...", crate::truncate_utf8(&e.content, 57))
        } else {
            e.content.clone()
        };
        lines.push(format!("- [{}] `#{}` {}", type_tag, e.id, preview));
    }

    Ok(CommandResult {
        content: lines.join("\n"),
        action: Some(CommandAction::DisplayOnly),
    })
}
