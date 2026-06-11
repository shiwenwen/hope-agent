//! `@skill` mention support — a curated, fixed allowlist of built-in skills the
//! user can activate inline from the chat composer's `@` menu.
//!
//! The mention is a Codex-style markdown link `[@<label>](#skill:<name>)`
//! (friendly localized label as visible text, stable id in the fragment href)
//! so the **same token renders as a chip in both the composer and the message
//! history**; the resolver reads only the id from the href. Unlike the `skill`
//! tool (model-invoked) or `/skill-name` slash command, the `@skill` mention is
//! a **user** gesture baked into the composer: the user picks one or more
//! built-in skills from the `@` popper and they ride the next turn as activated
//! skills. The set is intentionally fixed (office trio + browser + mac control)
//! — `@skill` is NOT a general skill-injection vector; arbitrary / disabled /
//! wrong-OS skill names are silently ignored at resolve time and stay in the
//! message as plain text.
//!
//! Resolution mirrors the inline slash/tool activation path: read SKILL.md,
//! substitute `$ARGUMENTS` (always empty for a mention), wrap with
//! [`build_skill_context_payload`], and inject into the turn's system context
//! (see `chat_engine::engine`), parallel to `knowledge::inject`.

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use super::{build_skill_context_payload, get_invocable_skills, SkillEntry};

/// Curated, fixed allowlist of built-in skills offered by the `@skill` menu.
/// Order here is the menu display order. `ha-mac-control` is macOS-only (gated
/// in [`is_mentionable_on_this_os`]); the rest are cross-platform.
pub const AT_MENTIONABLE_SKILLS: &[&str] = &[
    "office-docx",
    "office-pptx",
    "office-xlsx",
    "ha-browser",
    "ha-mac-control",
];

/// One row of the `@skill` menu. `name` is the canonical skill id (also the
/// `@skill:<name>` token); `description` is the skill's frontmatter blurb
/// (used as a tooltip). Friendly labels + icons are mapped frontend-side by
/// `name`, so the menu stays localized without round-tripping copy here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionableSkill {
    pub name: String,
    pub description: String,
}

/// macOS-only hard gate. `ha-mac-control` drives the native macOS desktop and
/// is meaningless elsewhere, so it's hidden from the menu and not resolvable
/// off macOS.
// `cfg!` folds to a literal per target, collapsing this into `true`/`!=` so
// clippy flags `needless_bool` on every target — keep the explicit form for
// readability across platforms.
#[allow(clippy::needless_bool)]
fn is_mentionable_on_this_os(name: &str) -> bool {
    if name == "ha-mac-control" {
        cfg!(target_os = "macos")
    } else {
        true
    }
}

/// Built-in skills eligible for `@skill` on this host: allowlist ∩ currently
/// invocable (respects `disabled_skills` / `user_invocable` / discoverable) ∩
/// OS gate. Returned in [`AT_MENTIONABLE_SKILLS`] order.
fn mentionable_entries() -> Vec<SkillEntry> {
    let cfg = crate::config::cached_config();
    let invocable = get_invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
    AT_MENTIONABLE_SKILLS
        .iter()
        .filter(|n| is_mentionable_on_this_os(n))
        .filter_map(|n| invocable.iter().find(|s| s.name == **n).cloned())
        .collect()
}

/// Menu rows for the composer `@skill` section (allowlist ∩ invocable ∩ OS).
pub fn list_mentionable_skills() -> Vec<MentionableSkill> {
    mentionable_entries()
        .into_iter()
        .map(|s| MentionableSkill {
            name: s.name,
            description: s.description,
        })
        .collect()
}

/// Matches the markdown-link mention `[@<label>](#skill:<name>)` — label is any
/// non-`]` run, captured group 1 is the skill id from the fragment href. Tied to
/// the composer's insertion + history chip form, so a stray `#skill:` in prose
/// (no link wrapper) never triggers.
fn skill_mention_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Label is `+` (non-empty), matching the frontend `parseSkillMentions`
    // grammar exactly so an empty-label `[@](#skill:x)` token resolves on
    // neither side (no chip + no inject, rather than inject-without-chip).
    RE.get_or_init(|| Regex::new(r"\[@[^\]\n]+\]\(#skill:([a-z0-9-]+)\)").unwrap())
}

/// Unique skill ids in first-seen order. Pure string scan — no allowlist / disk
/// check (those happen in [`resolve_inline_skill_mentions`]).
fn scan_skill_mention_names(message: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for caps in skill_mention_re().captures_iter(message) {
        if let Some(m) = caps.get(1) {
            let name = m.as_str();
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Scan `message` for `@skill:<name>` tokens, resolve each against the fixed
/// allowlist (∩ invocable ∩ OS gate), read its SKILL.md, and return a single
/// system-context block activating them — or `None` when nothing resolves.
///
/// Deterministic + user-controlled (parallel to `knowledge::inject`). Unknown /
/// disabled / wrong-OS names are silently skipped, leaving the raw `@skill:`
/// text in the user message. Deduped by name in first-seen order.
pub fn resolve_inline_skill_mentions(message: &str) -> Option<String> {
    let names = scan_skill_mention_names(message);
    if names.is_empty() {
        return None;
    }

    let entries = mentionable_entries();
    let mut activated: Vec<String> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    for name in &names {
        let Some(entry) = entries.iter().find(|s| &s.name == name) else {
            continue; // unknown / disabled / wrong-OS — leave as raw text
        };
        let content = match std::fs::read_to_string(&entry.file_path) {
            Ok(c) => c,
            Err(e) => {
                crate::app_warn!(
                    "skill",
                    "mention",
                    "Failed to read SKILL.md for @skill:{} ({}): skipping",
                    entry.name,
                    e
                );
                continue;
            }
        };
        // A mention carries no arguments — strip the placeholder so it doesn't
        // leak into the activated instructions.
        let substituted = content.replace("$ARGUMENTS", "");
        blocks.push(build_skill_context_payload(entry, &substituted));
        activated.push(entry.name.clone());
    }

    if blocks.is_empty() {
        return None;
    }

    crate::app_info!(
        "skill",
        "mention",
        "Activated {} @skill mention(s): {}",
        activated.len(),
        activated.join(", ")
    );

    Some(format!(
        "# Activated Skills (@skill)\n\n\
         The user activated the following built-in skill(s) for this turn via `@skill`. \
         Treat each skill's instructions below as authoritative guidance for completing \
         the request.\n\n{}",
        blocks.join("\n\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_multiple_mentions_in_order() {
        let names = scan_skill_mention_names(
            "make a deck [@PPT](#skill:office-pptx) then screenshot it [@Browser](#skill:ha-browser)",
        );
        assert_eq!(names, vec!["office-pptx", "ha-browser"]);
    }

    #[test]
    fn dedupes_repeated_mentions() {
        let names = scan_skill_mention_names(
            "[@Word](#skill:office-docx) and again [@Word doc](#skill:office-docx)",
        );
        assert_eq!(names, vec!["office-docx"]);
    }

    #[test]
    fn label_allows_spaces_and_cjk() {
        // The visible label is free-form (localized, may contain spaces / CJK);
        // only the href id matters.
        let names = scan_skill_mention_names("做个表格 [@Excel 表格](#skill:office-xlsx)");
        assert_eq!(names, vec!["office-xlsx"]);
    }

    #[test]
    fn bare_fragment_without_link_does_not_match() {
        // A stray `#skill:` (or email-like glob) without the `[@…](…)` link
        // wrapper must not trigger.
        assert!(scan_skill_mention_names("see #skill:office-docx for notes").is_empty());
        assert!(scan_skill_mention_names("reach me at user@skill:office-docx").is_empty());
    }

    #[test]
    fn matches_at_start_of_input() {
        let names = scan_skill_mention_names("[@Excel](#skill:office-xlsx) build a budget");
        assert_eq!(names, vec!["office-xlsx"]);
    }

    #[test]
    fn stops_at_non_token_chars() {
        // The closing `)` delimits the id; trailing punctuation is excluded.
        let names = scan_skill_mention_names("use [@Mac](#skill:ha-mac-control), please");
        assert_eq!(names, vec!["ha-mac-control"]);
    }

    #[test]
    fn no_mentions_returns_empty() {
        assert!(scan_skill_mention_names("just a normal message").is_empty());
        assert!(resolve_inline_skill_mentions("just a normal message").is_none());
    }

    #[test]
    fn allowlist_covers_office_browser_mac() {
        assert!(AT_MENTIONABLE_SKILLS.contains(&"office-docx"));
        assert!(AT_MENTIONABLE_SKILLS.contains(&"office-pptx"));
        assert!(AT_MENTIONABLE_SKILLS.contains(&"office-xlsx"));
        assert!(AT_MENTIONABLE_SKILLS.contains(&"ha-browser"));
        assert!(AT_MENTIONABLE_SKILLS.contains(&"ha-mac-control"));
    }

    #[test]
    fn mac_control_gated_to_macos() {
        assert_eq!(
            is_mentionable_on_this_os("ha-mac-control"),
            cfg!(target_os = "macos")
        );
        assert!(is_mentionable_on_this_os("office-docx"));
        assert!(is_mentionable_on_this_os("ha-browser"));
    }
}
