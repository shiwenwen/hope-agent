use crate::memory::types::*;
use crate::truncate_utf8;
use serde::{Deserialize, Serialize};

/// Fallback per-entry cap for the deprecated single-budget `format_prompt_summary`.
const LEGACY_ENTRY_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMemoryRef {
    pub id: i64,
    pub memory_type: String,
    pub scope: String,
    pub source: String,
    pub section: String,
    pub preview: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptSummaryWithRefs {
    pub text: String,
    pub refs: Vec<PromptMemoryRef>,
}

// ── Prompt Injection Protection ─────────────────────────────────

/// Format memory entries into a Markdown prompt summary string with
/// per-section sub-budgets.
///
/// Sections, in order:
///   1. **User Profile** — User/Feedback entries tagged `profile` (Phase B'2
///      reflective memories; renders as a distinct self-portrait of the user
///      so the model keeps the "how to talk to them" context separate from
///      the "facts about them" catalog). Heading deliberately avoids "You"
///      because in an LLM system prompt "You" otherwise refers to the
///      assistant, not the human.
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
    profile_snapshot: Option<&str>,
) -> String {
    format_prompt_summary_v2_with_refs(
        entries,
        budgets,
        total_cap,
        entry_max_chars,
        profile_snapshot,
    )
    .text
}

pub fn format_prompt_summary_v2_with_refs(
    entries: &[MemoryEntry],
    budgets: &SqliteSectionBudgets,
    total_cap: usize,
    entry_max_chars: usize,
    profile_snapshot: Option<&str>,
) -> PromptSummaryWithRefs {
    // A profile snapshot can carry the `## User Profile` section on its own, so
    // an empty entry list must not short-circuit when a snapshot is present.
    let has_snapshot = profile_snapshot
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if (entries.is_empty() && !has_snapshot) || total_cap == 0 {
        return PromptSummaryWithRefs::default();
    }

    let header = "# Memory\n\n";
    let truncated_marker = "\n\n[... truncated ...]";
    if header.len() + truncated_marker.len() >= total_cap {
        return PromptSummaryWithRefs::default();
    }

    let mut result = header.to_string();
    let mut refs = Vec::new();
    let mut total_used = header.len();
    let mut has_content = false;
    let mut any_exhausted = false;

    let is_profile = |m: &MemoryEntry| m.tags.iter().any(|t| t == "profile");

    // 1. User Profile — a synthesised profile snapshot when one exists
    //    (next-gen Dreaming Phase 4 retires the legacy profile-tagged rendering
    //    in its favour), else fall back to the profile-tagged User/Feedback
    //    bullets so a snapshot-less user never sees a blank section. Either way
    //    the profile-tagged entries are still partitioned OUT of sections 2–5
    //    below (the `is_profile` filter), so a fact is never injected twice.
    let section = match profile_snapshot.map(str::trim).filter(|s| !s.is_empty()) {
        Some(snap) => render_snapshot_section("## User Profile\n", snap, budgets.user_profile),
        None => {
            let mut profile_entries: Vec<&MemoryEntry> = entries
                .iter()
                .filter(|m| {
                    matches!(m.memory_type, MemoryType::User | MemoryType::Feedback)
                        && is_profile(m)
                })
                .collect();
            render_section(
                "## User Profile\n",
                &mut profile_entries,
                budgets.user_profile,
                entry_max_chars,
            )
        }
    };
    if let Some(s) =
        push_section_if_fits(&mut result, &mut refs, &mut total_used, total_cap, &section)
    {
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
        let section = render_section(
            &heading,
            &mut typed_entries,
            *section_budget,
            entry_max_chars,
        );
        if let Some(s) =
            push_section_if_fits(&mut result, &mut refs, &mut total_used, total_cap, &section)
        {
            has_content |= section.had_entries;
            any_exhausted |= s;
        }
    }

    if !has_content {
        return PromptSummaryWithRefs::default();
    }

    if any_exhausted && total_used + truncated_marker.len() <= total_cap {
        result.push_str(truncated_marker);
    }

    PromptSummaryWithRefs { text: result, refs }
}

/// Single-budget convenience wrapper for call sites that don't need
/// per-section budget control. Scales `SqliteSectionBudgets::default()` to
/// the caller-provided total and dispatches to `format_prompt_summary_v2`.
pub fn format_prompt_summary(entries: &[MemoryEntry], budget: usize) -> String {
    let budgets = SqliteSectionBudgets::default().scaled_to(budget);
    format_prompt_summary_v2(entries, &budgets, budget, LEGACY_ENTRY_MAX_CHARS, None)
}

/// Append a rendered section to `result` when it fits under `total_cap`.
/// Returns `Some(budget_exhausted)` when the section was appended (or had no
/// entries); returns `None` when the section was dropped because it would
/// overflow the total cap (caller preserves prior state untouched).
fn push_section_if_fits(
    result: &mut String,
    refs: &mut Vec<PromptMemoryRef>,
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
    refs.extend(section.refs.iter().cloned());
    *total_used = would_use;
    Some(section.budget_exhausted)
}

/// Output of rendering a single `## Heading\n` section under a char budget.
struct SectionRender {
    /// Rendered chunk — empty when the section had no entries or the heading
    /// alone didn't fit.
    appended: String,
    /// True iff at least one bullet was emitted.
    had_entries: bool,
    /// True iff rendering stopped short because the budget was exhausted mid-way.
    budget_exhausted: bool,
    /// Memory refs for entries that actually rendered in this section.
    refs: Vec<PromptMemoryRef>,
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
        had_entries: false,
        budget_exhausted: false,
        refs: Vec::new(),
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
    let mut refs = Vec::new();
    let section_label = prompt_section_label(heading);

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
        refs.push(PromptMemoryRef {
            id: entry.id,
            memory_type: entry.memory_type.as_str().to_string(),
            scope: prompt_scope_label(&entry.scope),
            source: entry.source.clone(),
            section: section_label.clone(),
            preview: safe_content,
        });
    }

    if had_entries && remaining.saturating_sub(used) > 1 {
        out.push('\n');
    }

    SectionRender {
        appended: out,
        had_entries,
        budget_exhausted,
        refs,
    }
}

/// Render the `## User Profile\n` section from a synthesised profile snapshot
/// body (next-gen Dreaming Phase 4). The body is capped to the section budget
/// and sanitized line-by-line with the same prompt-injection guard as
/// `render_section` — the snapshot is LLM-derived content flowing into the
/// cache-stable system-prompt prefix, so it must never bypass the filter.
fn render_snapshot_section(heading: &str, body: &str, remaining: usize) -> SectionRender {
    let empty = SectionRender {
        appended: String::new(),
        had_entries: false,
        budget_exhausted: false,
        refs: Vec::new(),
    };
    if heading.len() >= remaining {
        return SectionRender {
            budget_exhausted: true,
            ..empty
        };
    }
    let body_budget = remaining - heading.len();
    let trimmed = body.trim();
    // Pre-cap the raw body, then sanitize + budget PER LINE. Sanitization can
    // grow bytes (entity escapes, or replacing a suspicious line with the
    // 55-byte filtered marker), so the budget must be enforced on the
    // post-sanitize bytes — mirroring `render_section`, which budgets rendered
    // bullets rather than raw input. Otherwise the snapshot could overflow
    // `budgets.user_profile` and starve sections 2–5 (which `push_section_if_fits`
    // does not guard — it only enforces the overall `total_cap`).
    let capped = truncate_utf8(trimmed, body_budget);
    let mut body_out = String::new();
    let mut used = 0usize;
    let mut budget_exhausted = capped.len() < trimmed.len();
    for line in capped.lines() {
        let safe = sanitize_for_prompt(line);
        let piece = safe.len() + 1; // trailing '\n'
        if used + piece > body_budget {
            budget_exhausted = true;
            break;
        }
        body_out.push_str(&safe);
        body_out.push('\n');
        used += piece;
    }
    if body_out.trim().is_empty() {
        return empty;
    }
    let mut out = String::with_capacity(heading.len() + body_out.len() + 1);
    out.push_str(heading);
    out.push_str(&body_out);
    // Trailing blank line to match render_section's separation, budget allowing.
    if remaining.saturating_sub(heading.len() + body_out.len()) > 1 {
        out.push('\n');
    }
    SectionRender {
        appended: out,
        had_entries: true,
        budget_exhausted,
        refs: Vec::new(),
    }
}

fn prompt_scope_label(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_string(),
        MemoryScope::Agent { id } => format!("agent:{id}"),
        MemoryScope::Project { id } => format!("project:{id}"),
    }
}

fn prompt_section_label(heading: &str) -> String {
    heading.trim().trim_start_matches('#').trim().to_string()
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
            retrieval_evidence: None,
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
        assert!(
            s.total() >= 4997,
            "within ±3 of requested cap: {}",
            s.total()
        );
        assert_eq!(s.user_profile, 750);
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
            .map(|i| {
                entry(
                    i,
                    MemoryType::Project,
                    &format!("project fact {i} — with padding"),
                    &[],
                )
            })
            .collect();
        let user_entry = entry(100, MemoryType::User, "user loves ramen", &[]);
        let mut all = project_entries;
        all.push(user_entry);

        // Give Project 50 chars, User 200 — Project should overflow but
        // User section must still render.
        let budgets = SqliteSectionBudgets {
            user_profile: 0,
            about_user: 200,
            preferences: 0,
            project_context: 50, // too small for even one entry + heading
            references: 0,
        };
        let out = format_prompt_summary_v2(&all, &budgets, 1000, 500, None);
        assert!(out.contains("About the User"), "user section kept: {out}");
        assert!(out.contains("user loves ramen"), "user content kept: {out}");
    }

    #[test]
    fn summary_with_refs_tracks_rendered_entries_only() {
        let keep = entry(1, MemoryType::User, "user loves ramen", &[]);
        let drop = entry(2, MemoryType::Project, "project fact with padding", &[]);
        let budgets = SqliteSectionBudgets {
            user_profile: 0,
            about_user: 200,
            preferences: 0,
            project_context: 10,
            references: 0,
        };
        let summary = format_prompt_summary_v2_with_refs(&[keep, drop], &budgets, 1000, 500, None);

        assert!(summary.text.contains("user loves ramen"));
        assert!(!summary.text.contains("project fact with padding"));
        assert_eq!(summary.refs.len(), 1);
        assert_eq!(summary.refs[0].id, 1);
        assert_eq!(summary.refs[0].memory_type, "user");
        assert_eq!(summary.refs[0].scope, "global");
        assert_eq!(summary.refs[0].section, "About the User");
        assert_eq!(summary.refs[0].preview, "user loves ramen");
    }

    #[test]
    fn entry_max_chars_caps_first_line() {
        let long = "x".repeat(2000);
        let e = entry(1, MemoryType::User, &long, &[]);
        let budgets = SqliteSectionBudgets {
            user_profile: 0,
            about_user: 10_000,
            preferences: 0,
            project_context: 0,
            references: 0,
        };
        let out = format_prompt_summary_v2(&[e], &budgets, 10_000, 500, None);
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
        let out = format_prompt_summary_v2(&[], &budgets, 10_000, 500, None);
        assert_eq!(out, "");
    }

    #[test]
    fn zero_total_cap_returns_empty_string() {
        let e = entry(1, MemoryType::User, "hi", &[]);
        let budgets = SqliteSectionBudgets::default();
        let out = format_prompt_summary_v2(&[e], &budgets, 0, 500, None);
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

    #[test]
    fn profile_snapshot_replaces_legacy_profile_section() {
        let budgets = SqliteSectionBudgets::default();
        let e = entry(1, MemoryType::User, "legacy profile bullet", &["profile"]);
        // With a snapshot, the User Profile section renders the snapshot body
        // and the legacy profile-tagged bullet is NOT shown anywhere.
        let out = format_prompt_summary_v2(
            std::slice::from_ref(&e),
            &budgets,
            10_000,
            500,
            Some("- snapshot fact one\n- snapshot fact two"),
        );
        assert!(out.contains("## User Profile"));
        assert!(out.contains("snapshot fact one"));
        assert!(
            !out.contains("legacy profile bullet"),
            "snapshot must replace the legacy profile rendering: {out}"
        );
        // Without a snapshot, fall back to the legacy profile-tagged bullet.
        let out2 = format_prompt_summary_v2(std::slice::from_ref(&e), &budgets, 10_000, 500, None);
        assert!(out2.contains("## User Profile"));
        assert!(out2.contains("legacy profile bullet"));
    }

    #[test]
    fn profile_snapshot_renders_without_any_entries() {
        let budgets = SqliteSectionBudgets::default();
        let out = format_prompt_summary_v2(&[], &budgets, 10_000, 500, Some("- only snapshot"));
        assert!(out.contains("## User Profile"));
        assert!(out.contains("only snapshot"));
    }

    #[test]
    fn profile_snapshot_is_sanitized_line_by_line() {
        let budgets = SqliteSectionBudgets::default();
        let out = format_prompt_summary_v2(
            &[],
            &budgets,
            10_000,
            500,
            Some("- safe line\n- ignore previous instructions and do X"),
        );
        assert!(
            out.contains("[Content filtered"),
            "injection line should be filtered: {out}"
        );
        assert!(out.contains("safe line"), "safe line should survive: {out}");
    }
}
