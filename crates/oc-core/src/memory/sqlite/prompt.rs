use crate::memory::types::*;
use crate::truncate_utf8;

/// Fallback per-entry cap for the deprecated single-budget `format_prompt_summary`.
const LEGACY_ENTRY_MAX_CHARS: usize = 500;

// ── Prompt Injection Protection ─────────────────────────────────

/// Format memory entries into a Markdown prompt summary string with
/// per-section sub-budgets.
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
/// Each section is sorted pinned-first, then by recency. Each section has an
/// **independent** character budget supplied via `budgets` (optionally scaled
/// into `total_cap` first); unused section budget is NOT forwarded to later
/// sections so a popular type (e.g. Project Context) can never starve the
/// others. `entry_max_chars` caps each bullet's first-line rendering.
///
/// `total_cap` is an upper bound on the entire output of this function —
/// when it's smaller than `budgets.total()` the caller should pass
/// `budgets.scaled_to(total_cap)`; we still honour the raw `total_cap` here
/// as a defensive final clip.
pub fn format_prompt_summary_v2(
    entries: &[MemoryEntry],
    budgets: &SqliteSectionBudgets,
    total_cap: usize,
    entry_max_chars: usize,
) -> String {
    if entries.is_empty() || total_cap == 0 {
        return String::new();
    }

    let header = "# Memory\n\n";
    let truncated_marker = "\n\n[... truncated ...]";
    if header.len() + truncated_marker.len() >= total_cap {
        return String::new();
    }

    let mut result = header.to_string();
    let mut total_used = header.len();
    let mut has_content = false;
    let mut any_exhausted = false;

    let is_profile = |m: &MemoryEntry| m.tags.iter().any(|t| t == "profile");

    // 1. About You — profile-tagged User/Feedback.
    let mut profile_entries: Vec<&MemoryEntry> = entries
        .iter()
        .filter(|m| {
            matches!(m.memory_type, MemoryType::User | MemoryType::Feedback) && is_profile(m)
        })
        .collect();
    let section = render_section(
        "## About You\n",
        &mut profile_entries,
        budgets.about_you,
        entry_max_chars,
    );
    if let Some(s) = push_section_if_fits(&mut result, &mut total_used, total_cap, &section) {
        has_content |= section.had_entries;
        any_exhausted |= s;
    }

    // 2–5. User, Feedback, Project, Reference — each with its own sub-budget.
    let specs: &[(MemoryType, usize)] = &[
        (MemoryType::User, budgets.about_user),
        (MemoryType::Feedback, budgets.preferences),
        (MemoryType::Project, budgets.project_context),
        (MemoryType::Reference, budgets.references),
    ];
    for (mem_type, section_budget) in specs {
        let mut typed_entries: Vec<&MemoryEntry> = entries
            .iter()
            .filter(|m| {
                &m.memory_type == mem_type
                    && !(matches!(mem_type, MemoryType::User | MemoryType::Feedback)
                        && is_profile(m))
            })
            .collect();
        let heading = format!("## {}\n", mem_type.heading());
        let section =
            render_section(&heading, &mut typed_entries, *section_budget, entry_max_chars);
        if let Some(s) = push_section_if_fits(&mut result, &mut total_used, total_cap, &section) {
            has_content |= section.had_entries;
            any_exhausted |= s;
        }
    }

    if !has_content {
        return String::new();
    }

    if any_exhausted && total_used + truncated_marker.len() <= total_cap {
        result.push_str(truncated_marker);
    }

    result
}

/// Legacy single-budget API. Preserves the old "uniform bullet list under
/// one shared budget" behavior for call sites not yet migrated to v2.
#[deprecated(
    note = "use `format_prompt_summary_v2` with an explicit SqliteSectionBudgets + entry_max_chars"
)]
pub fn format_prompt_summary(entries: &[MemoryEntry], budget: usize) -> String {
    let budgets = SqliteSectionBudgets::default().scaled_to(budget);
    format_prompt_summary_v2(entries, &budgets, budget, LEGACY_ENTRY_MAX_CHARS)
}

/// Append a rendered section to `result` when it fits under `total_cap`.
/// Returns `Some(budget_exhausted)` when the section was appended (or had no
/// entries); returns `None` when the section was dropped because it would
/// overflow the total cap (caller preserves prior state untouched).
fn push_section_if_fits(
    result: &mut String,
    total_used: &mut usize,
    total_cap: usize,
    section: &SectionRender,
) -> Option<bool> {
    if section.appended.is_empty() {
        return Some(section.budget_exhausted);
    }
    let would_use = *total_used + section.appended.len();
    if would_use > total_cap {
        return None;
    }
    result.push_str(&section.appended);
    *total_used = would_use;
    Some(section.budget_exhausted)
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
    #[allow(dead_code)]
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
    entry_max_chars: usize,
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
        let capped_line = truncate_utf8(content_line, entry_max_chars);
        let safe_content = sanitize_for_prompt(capped_line);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, ty: MemoryType, content: &str, tags: &[&str]) -> MemoryEntry {
        MemoryEntry {
            id,
            memory_type: ty,
            scope: MemoryScope::Global,
            content: content.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            source: "user".to_string(),
            source_session_id: None,
            pinned: false,
            created_at: "2026-04-18T00:00:00Z".into(),
            updated_at: "2026-04-18T00:00:00Z".into(),
            relevance_score: None,
            attachment_path: None,
            attachment_mime: None,
        }
    }

    #[test]
    fn scaled_to_preserves_ratio_when_over_cap() {
        let budgets = SqliteSectionBudgets::default(); // 1500+2000+2000+3000+1500 = 10_000
        let s = budgets.scaled_to(5000);
        // Integer division with a 0.5 ratio over exact multiples — no rounding loss.
        assert!(s.total() <= 5000);
        assert!(s.total() >= 4997, "within ±3 of requested cap: {}", s.total());
        assert_eq!(s.about_you, 750);
        assert_eq!(s.about_user, 1000);
        assert_eq!(s.preferences, 1000);
        assert_eq!(s.project_context, 1500);
        assert_eq!(s.references, 750);
    }

    #[test]
    fn scaled_to_passthrough_when_within_cap() {
        let budgets = SqliteSectionBudgets::default();
        let s = budgets.scaled_to(20_000);
        assert_eq!(s, budgets);
    }

    #[test]
    fn scaled_to_zero_produces_empty() {
        let budgets = SqliteSectionBudgets::default();
        let s = budgets.scaled_to(0);
        assert_eq!(s.total(), 0);
    }

    #[test]
    fn per_section_budget_isolates_project_overflow() {
        // 6 project entries of ~40 chars each = ~240 chars total.
        let project_entries: Vec<MemoryEntry> = (0..6)
            .map(|i| entry(i, MemoryType::Project, &format!("project fact {i} — with padding"), &[]))
            .collect();
        let user_entry = entry(100, MemoryType::User, "user loves ramen", &[]);
        let mut all = project_entries;
        all.push(user_entry);

        // Give Project 50 chars, User 200 — Project should overflow but
        // User section must still render.
        let budgets = SqliteSectionBudgets {
            about_you: 0,
            about_user: 200,
            preferences: 0,
            project_context: 50, // too small for even one entry + heading
            references: 0,
        };
        let out = format_prompt_summary_v2(&all, &budgets, 1000, 500);
        assert!(out.contains("About the User"), "user section kept: {out}");
        assert!(out.contains("user loves ramen"), "user content kept: {out}");
    }

    #[test]
    fn entry_max_chars_caps_first_line() {
        let long = "x".repeat(2000);
        let e = entry(1, MemoryType::User, &long, &[]);
        let budgets = SqliteSectionBudgets {
            about_you: 0,
            about_user: 10_000,
            preferences: 0,
            project_context: 0,
            references: 0,
        };
        let out = format_prompt_summary_v2(&[e], &budgets, 10_000, 500);
        // The rendered bullet line is "- <500 chars of x>\n" — verify we
        // don't see a 2000-long "x" run anywhere.
        assert!(
            !out.contains(&"x".repeat(501)),
            "entry_max_chars=500 must cap the first line"
        );
    }

    #[test]
    fn empty_entries_returns_empty_string() {
        let budgets = SqliteSectionBudgets::default();
        let out = format_prompt_summary_v2(&[], &budgets, 10_000, 500);
        assert_eq!(out, "");
    }

    #[test]
    fn zero_total_cap_returns_empty_string() {
        let e = entry(1, MemoryType::User, "hi", &[]);
        let budgets = SqliteSectionBudgets::default();
        let out = format_prompt_summary_v2(&[e], &budgets, 0, 500);
        assert_eq!(out, "");
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_wrapper_delegates_to_v2() {
        let e = entry(1, MemoryType::User, "fact about user", &[]);
        let out = format_prompt_summary(&[e], 2_000);
        assert!(out.contains("About the User"));
        assert!(out.contains("fact about user"));
    }
}
