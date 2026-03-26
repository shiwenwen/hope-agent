use anyhow::{Context, Result};

use super::types::*;

// ── Import Parsers ──────────────────────────────────────────────

/// Parse JSON import format: array of { content, type?, scope?, tags? }
pub fn parse_import_json(json_str: &str) -> Result<Vec<NewMemory>> {
    let items: Vec<serde_json::Value> = serde_json::from_str(json_str)
        .with_context(|| "Invalid JSON: expected an array of memory objects")?;

    let mut entries = Vec::new();
    for item in items {
        let content = item.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Each memory must have a 'content' field"))?;

        let memory_type = item.get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("user");

        let scope_str = item.get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("global");

        let agent_id = item.get("agentId")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let tags: Vec<String> = item.get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let scope = if scope_str == "agent" {
            MemoryScope::Agent { id: agent_id.to_string() }
        } else {
            MemoryScope::Global
        };

        entries.push(NewMemory {
            memory_type: MemoryType::from_str(memory_type),
            scope,
            content: content.to_string(),
            tags,
            source: "import".to_string(),
            source_session_id: None,
            pinned: false,
        });
    }
    Ok(entries)
}

/// Parse Markdown import format:
/// ## About the User / Preferences & Feedback / Project Context / References
/// ### Entry title
/// Tags: tag1, tag2
/// Scope: global | Source: user | Updated: ...
///
/// Content here...
///
/// ---
pub fn parse_import_markdown(md_str: &str) -> Result<Vec<NewMemory>> {
    let mut entries = Vec::new();
    let mut current_type = MemoryType::User;
    let mut current_content = String::new();
    let mut current_tags: Vec<String> = Vec::new();
    let mut in_entry = false;

    for line in md_str.lines() {
        let trimmed = line.trim();

        // Type heading
        if trimmed.starts_with("## ") {
            // Flush previous entry
            if in_entry && !current_content.trim().is_empty() {
                entries.push(NewMemory {
                    memory_type: current_type.clone(),
                    scope: MemoryScope::Global,
                    content: current_content.trim().to_string(),
                    tags: std::mem::take(&mut current_tags),
                    source: "import".to_string(),
                    source_session_id: None,
                    pinned: false,
                });
                current_content.clear();
                in_entry = false;
            }

            let heading = trimmed.trim_start_matches("## ").trim();
            current_type = match heading {
                "Preferences & Feedback" => MemoryType::Feedback,
                "Project Context" => MemoryType::Project,
                "References" => MemoryType::Reference,
                _ => MemoryType::User,
            };
        } else if trimmed.starts_with("### ") {
            // Flush previous entry
            if in_entry && !current_content.trim().is_empty() {
                entries.push(NewMemory {
                    memory_type: current_type.clone(),
                    scope: MemoryScope::Global,
                    content: current_content.trim().to_string(),
                    tags: std::mem::take(&mut current_tags),
                    source: "import".to_string(),
                    source_session_id: None,
                    pinned: false,
                });
                current_content.clear();
            }
            in_entry = true;
        } else if trimmed.starts_with("Tags:") {
            current_tags = trimmed.trim_start_matches("Tags:")
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        } else if trimmed.starts_with("Scope:") || trimmed == "---" {
            // Skip metadata lines and separators
        } else if in_entry {
            if !current_content.is_empty() || !trimmed.is_empty() {
                current_content.push_str(line);
                current_content.push('\n');
            }
        }
    }

    // Flush last entry
    if in_entry && !current_content.trim().is_empty() {
        entries.push(NewMemory {
            memory_type: current_type,
            scope: MemoryScope::Global,
            content: current_content.trim().to_string(),
            tags: current_tags,
            source: "import".to_string(),
            source_session_id: None,
            pinned: false,
        });
    }

    Ok(entries)
}
