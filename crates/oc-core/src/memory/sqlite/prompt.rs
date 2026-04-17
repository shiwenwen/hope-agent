use crate::memory::types::*;

// ── Prompt Injection Protection ─────────────────────────────────

/// Format memory entries into a Markdown prompt summary string.
///
/// Sections, in order:
///   1. **About You** — User/Feedback entries tagged `profile` (Phase B'2
///      reflective memories; renders as a distinct self-portrait of the user
///      so the model keeps the "how to talk to them" context separate from
///      the "facts about them" catalog).
///   2. About the User   — remaining User entries (non-profile).
///   3. Preferences & Feedback — remaining Feedback entries (non-profile).
///   4. Project Context  — all Project entries.
///   5. References       — all Reference entries.
///
/// Each section is sorted pinned-first, then by recency, and respects the
/// char budget. Used by `build_prompt_summary` and LLM selection.
pub fn format_prompt_summary(entries: &[MemoryEntry], budget: usize) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let header = "# Memory\n\n";
    let truncated_marker = "\n\n[... truncated ...]";
    let mut result = header.to_string();
    let mut remaining = budget.saturating_sub(header.len() + truncated_marker.len());
    let mut has_content = false;
    let mut budget_exhausted = false;

    let is_profile = |m: &MemoryEntry| m.tags.iter().any(|t| t == "profile");

    // 1. About You — profile-tagged User/Feedback
    let mut profile_entries: Vec<&MemoryEntry> = entries
        .iter()
        .filter(|m| {
            matches!(m.memory_type, MemoryType::User | MemoryType::Feedback) && is_profile(m)
        })
        .collect();
    if !profile_entries.is_empty() {
        render_section(
            "## About You\n",
            &mut profile_entries,
            &mut result,
            &mut remaining,
            &mut has_content,
            &mut budget_exhausted,
        );
    }

    let type_order = [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ];

    for mem_type in &type_order {
        if budget_exhausted {
            break;
        }

        // Non-profile entries only in the per-type sections (profile items
        // were rendered above).
        let mut typed_entries: Vec<&MemoryEntry> = entries
            .iter()
            .filter(|m| {
                &m.memory_type == mem_type
                    && !(matches!(mem_type, MemoryType::User | MemoryType::Feedback)
                        && is_profile(m))
            })
            .collect();
        let heading = format!("## {}\n", mem_type.heading());
        render_section(
            &heading,
            &mut typed_entries,
            &mut result,
            &mut remaining,
            &mut has_content,
            &mut budget_exhausted,
        );
    }

    if !has_content {
        return String::new();
    }

    if budget_exhausted {
        result.push_str(truncated_marker);
    }

    result
}

/// Render one `## Heading\n` section with bulleted entries under the budget.
/// Updates `result`/`remaining`/`has_content`/`budget_exhausted` in place so
/// the caller can chain multiple sections and short-circuit when space runs out.
fn render_section(
    heading: &str,
    entries: &mut Vec<&MemoryEntry>,
    result: &mut String,
    remaining: &mut usize,
    has_content: &mut bool,
    budget_exhausted: &mut bool,
) {
    if *budget_exhausted || entries.is_empty() {
        return;
    }
    entries.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    if heading.len() > *remaining {
        *budget_exhausted = true;
        return;
    }
    *remaining -= heading.len();
    result.push_str(heading);
    let mut section_has_entries = false;

    for entry in entries.iter() {
        let prefix = if entry.pinned { "★ " } else { "" };
        let att_prefix = match (&entry.attachment_path, &entry.attachment_mime) {
            (Some(_), Some(mime)) if mime.starts_with("image/") => "[img] ",
            (Some(_), Some(mime)) if mime.starts_with("audio/") => "[audio] ",
            _ => "",
        };
        let content_line = entry.content.lines().next().unwrap_or(&entry.content);
        let safe_content = sanitize_for_prompt(content_line);
        let line = format!("- {}{}{}\n", prefix, att_prefix, safe_content);
        if line.len() > *remaining {
            *budget_exhausted = true;
            break;
        }
        *remaining -= line.len();
        result.push_str(&line);
        section_has_entries = true;
    }

    if section_has_entries {
        *has_content = true;
        if *remaining > 1 {
            result.push('\n');
            *remaining = remaining.saturating_sub(1);
        }
    }
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
