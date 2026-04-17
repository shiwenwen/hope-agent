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

    // 1. About You — profile-tagged User/Feedback.
    let mut profile_entries: Vec<&MemoryEntry> = entries
        .iter()
        .filter(|m| {
            matches!(m.memory_type, MemoryType::User | MemoryType::Feedback) && is_profile(m)
        })
        .collect();
    let section = render_section("## About You\n", &mut profile_entries, remaining);
    result.push_str(&section.appended);
    remaining = remaining.saturating_sub(section.consumed);
    has_content |= section.had_entries;
    budget_exhausted |= section.budget_exhausted;

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
        let section = render_section(&heading, &mut typed_entries, remaining);
        result.push_str(&section.appended);
        remaining = remaining.saturating_sub(section.consumed);
        has_content |= section.had_entries;
        budget_exhausted |= section.budget_exhausted;
    }

    if !has_content {
        return String::new();
    }

    if budget_exhausted {
        result.push_str(truncated_marker);
    }

    result
}

/// Output of rendering a single `## Heading\n` section under a char budget.
/// Returned as a value so `format_prompt_summary` can fold it into the running
/// state without needing six mutable out-parameters.
struct SectionRender {
    /// Rendered chunk — empty when the section had no entries or the heading
    /// alone didn't fit.
    appended: String,
    /// How many chars of the budget this chunk consumed (`appended.len()`
    /// plus one more for the trailing blank line when present).
    consumed: usize,
    /// True iff at least one bullet was emitted.
    had_entries: bool,
    /// True iff rendering stopped short because the budget was exhausted mid-way.
    budget_exhausted: bool,
}

/// Render one `## Heading\n` section with bulleted entries under the budget.
/// Caller is responsible for folding the result into its running state.
fn render_section(
    heading: &str,
    entries: &mut Vec<&MemoryEntry>,
    remaining: usize,
) -> SectionRender {
    let empty = SectionRender {
        appended: String::new(),
        consumed: 0,
        had_entries: false,
        budget_exhausted: false,
    };
    if entries.is_empty() {
        return empty;
    }
    if heading.len() > remaining {
        return SectionRender {
            budget_exhausted: true,
            ..empty
        };
    }
    entries.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });

    let mut out = String::with_capacity(heading.len());
    out.push_str(heading);
    let mut used = heading.len();
    let mut had_entries = false;
    let mut budget_exhausted = false;

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
        if used + line.len() > remaining {
            budget_exhausted = true;
            break;
        }
        used += line.len();
        out.push_str(&line);
        had_entries = true;
    }

    if had_entries && remaining.saturating_sub(used) > 1 {
        out.push('\n');
        used += 1;
    }

    SectionRender {
        appended: out,
        consumed: used,
        had_entries,
        budget_exhausted,
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
