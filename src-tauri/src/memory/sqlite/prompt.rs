use crate::memory::types::*;

// ── Prompt Injection Protection ─────────────────────────────────

/// Format memory entries into a Markdown prompt summary string.
/// Groups by type (User -> Feedback -> Project -> Reference), pinned first,
/// respects character budget. Used by `build_prompt_summary` and LLM selection.
pub fn format_prompt_summary(entries: &[MemoryEntry], budget: usize) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let type_order = [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ];
    let header = "# Memory\n\n";
    let truncated_marker = "\n\n[... truncated ...]";
    let mut result = header.to_string();
    let mut remaining = budget.saturating_sub(header.len() + truncated_marker.len());
    let mut has_content = false;
    let mut budget_exhausted = false;

    for mem_type in &type_order {
        if budget_exhausted {
            break;
        }

        let mut typed_entries: Vec<&MemoryEntry> = entries
            .iter()
            .filter(|m| &m.memory_type == mem_type)
            .collect();
        typed_entries.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        if typed_entries.is_empty() {
            continue;
        }

        let heading = format!("## {}\n", mem_type.heading());
        if heading.len() > remaining {
            budget_exhausted = true;
            break;
        }

        remaining -= heading.len();
        result.push_str(&heading);
        let mut section_has_entries = false;

        for entry in &typed_entries {
            let prefix = if entry.pinned { "★ " } else { "" };
            let att_prefix = match (&entry.attachment_path, &entry.attachment_mime) {
                (Some(_), Some(mime)) if mime.starts_with("image/") => "[img] ",
                (Some(_), Some(mime)) if mime.starts_with("audio/") => "[audio] ",
                _ => "",
            };
            let content_line = entry.content.lines().next().unwrap_or(&entry.content);
            let safe_content = sanitize_for_prompt(content_line);
            let line = format!("- {}{}{}\n", prefix, att_prefix, safe_content);
            if line.len() > remaining {
                budget_exhausted = true;
                break;
            }
            remaining -= line.len();
            result.push_str(&line);
            section_has_entries = true;
        }

        if section_has_entries {
            has_content = true;
            if remaining > 1 {
                result.push('\n');
                remaining = remaining.saturating_sub(1);
            }
        }
    }

    if !has_content {
        return String::new();
    }

    if budget_exhausted {
        result.push_str(truncated_marker);
    }

    result
}

/// Sanitize memory content before injecting into system prompt.
/// Detects suspicious patterns and escapes special tokens.
pub(crate) fn sanitize_for_prompt(content: &str) -> String {
    let lower = content.to_lowercase();
    let suspicious_patterns = [
        "ignore previous instructions",
        "ignore all instructions",
        "ignore above instructions",
        "disregard previous",
        "disregard all previous",
        "you are now",
        "new instructions:",
        "system prompt:",
        "<<sys>>",
        "<|im_start|>",
        "<|endoftext|>",
        "<|system|>",
    ];

    if suspicious_patterns.iter().any(|p| lower.contains(p)) {
        return "[Content filtered: potential prompt injection detected]".to_string();
    }

    // Escape special tokens that could be interpreted by LLMs
    content
        .replace("<|", "&lt;|")
        .replace("|>", "|&gt;")
        .replace("<<sys>>", "&lt;&lt;sys&gt;&gt;")
}
