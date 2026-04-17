use std::collections::{HashMap, HashSet};

use super::requirements::check_requirements;
use super::types::*;

// ── Prompt Generation ────────────────────────────────────────────

/// Build the skills section of the system prompt with lazy-load pattern.
///
/// Three-tier progressive degradation:
/// 1. Full format: `- name: description`
/// 2. Compact format: `- name` — when full exceeds budget
/// 3. Truncated: binary-search largest prefix that fits compact budget
///
/// Skills with `disable_model_invocation == true` are excluded from the prompt.
/// Disabled skills and skills failing env_check are also excluded.
/// `allow_bundled` restricts which bundled skills are included (empty = all allowed).
///
/// Activation uses the internal `skill` tool rather than `read`-ing SKILL.md,
/// so file paths are deliberately absent from catalog entries — the tool looks
/// skills up by name.
pub fn build_skills_prompt(
    skills: &[SkillEntry],
    disabled: &[String],
    env_check: bool,
    skill_env: &HashMap<String, HashMap<String, String>>,
    budget: &SkillPromptBudget,
    allow_bundled: &[String],
    activated_conditional: &HashSet<String>,
) -> String {
    let active: Vec<&SkillEntry> = skills
        .iter()
        .filter(|s| !disabled.contains(&s.name))
        // Filter by invocation policy: hide from model if disabled
        .filter(|s| s.disable_model_invocation != Some(true))
        // Draft / Archived skills are never surfaced to the model
        .filter(|s| s.status.is_discoverable())
        // Bundled allowlist
        .filter(|s| {
            if allow_bundled.is_empty() || s.source != "bundled" {
                return true;
            }
            let key = s.skill_key.as_deref().unwrap_or(&s.name);
            allow_bundled.iter().any(|a| a == key || a == &s.name)
        })
        .filter(|s| !env_check || check_requirements(&s.requires, skill_env.get(&s.name)))
        // `paths:` skills are hidden until activated for this session.
        // Skills without `paths:` are always visible (unchanged behavior).
        .filter(|s| match &s.paths {
            Some(p) if !p.is_empty() => activated_conditional.contains(&s.name),
            _ => true,
        })
        .collect();

    if active.is_empty() {
        return String::new();
    }

    let max_count = budget.max_count.min(active.len());
    let active = &active[..max_count];

    // Header
    let header = "\n\nThe following skills provide specialized instructions for specific tasks.\n\
        Use the `skill` tool to activate a skill by name — e.g. `skill({ name: \"<skill-name>\", args: \"<optional>\" })`.\n\
        Do NOT `read` SKILL.md files to activate a skill; the `skill` tool handles loading, \
        argument substitution, and (for `context: fork` skills) sub-agent isolation.\n\
        Only activate the skill most relevant to the current task — do not activate more than one up front.";

    // Try full format first
    let full_lines: Vec<String> = active
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect();

    let full_text = format!("{}\n{}", header, full_lines.join("\n"));

    if full_text.len() <= budget.max_chars {
        return full_text;
    }

    // Fall back to compact format (no descriptions)
    let compact_lines: Vec<String> = active
        .iter()
        .map(|s| format!("- {}", s.name))
        .collect();

    let compact_text = format!("{}\n{}", header, compact_lines.join("\n"));

    if compact_text.len() <= budget.max_chars {
        let warning = format!(
            "\n\n\u{26a0}\u{fe0f} Skills catalog using compact format (descriptions omitted). {} skills available.",
            active.len()
        );
        return format!("{}{}", compact_text, warning);
    }

    // Binary search for largest prefix that fits
    let mut lo: usize = 0;
    let mut hi: usize = compact_lines.len();

    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate = format!("{}\n{}", header, compact_lines[..mid].join("\n"));
        // Reserve space for truncation warning (~120 chars)
        if candidate.len() + 120 <= budget.max_chars {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    let truncated = if lo > 0 {
        format!(
            "{}\n{}\n\n\u{26a0}\u{fe0f} Skills truncated: showing {} of {} (compact format, descriptions omitted).",
            header,
            compact_lines[..lo].join("\n"),
            lo,
            active.len()
        )
    } else {
        // Even one skill doesn't fit — just show the header
        header.to_string()
    };

    truncated
}
