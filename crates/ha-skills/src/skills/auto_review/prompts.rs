//! Auto-review prompt templates.
//!
//! The review model returns a single JSON `ReviewDecision` object. The
//! pipeline (gates 3 & 4 in `pipeline.rs`) enforces additional structural
//! invariants after parsing — never trust the prompt to be the only line of
//! defense.

use std::fmt::Write as _;

/// Built-in system prompt for the review side_query. Users can override the
/// whole text via `SkillsAutoReviewConfig::review_system_override`; gates 2,
/// 4 and 5 still run unconditionally so the override cannot lower the
/// quality floor.
pub const REVIEW_SYSTEM: &str = r#"You are a skill-review assistant. Decide whether the recent conversation
revealed a CLASS-LEVEL, reusable methodology worth capturing — or whether
nothing durable came out of it.

CLASS-LEVEL means: the skill names a recurring category of work the user
or a future agent will face again ("debug-streaming-tool-loop",
"audit-rust-clippy-warnings"). Not a one-time task ("fix-issue-123",
"investigate-todays-deploy"), not a personal life decision, not a recap of
this very conversation.

OUTPUT — emit exactly ONE JSON object. No markdown fences, no prose around
it. Schema:

{
  "decision": "create" | "patch" | "skip",
  "skill_id": "<kebab-case-id>",           // create + patch
  "name": "<short human name in the user's language>",  // create
  "description": "<one sentence in the user's language>", // create
  "body": "<full SKILL.md markdown body, NO frontmatter, in the user's language>", // create
  "reuse_scenarios": ["<scenario 1>", "<scenario 2>", "<scenario 3>"], // create — 3 entries, each >= 20 chars, each describing a CONCRETE future situation: how the user might phrase the request, what file / command / context will be involved
  "reuse_probability": 0.0,                // create — your honest 0..1 estimate that this skill gets used in the next 30 days
  "class_level_name": true,                // create — is `skill_id` class-level (true) or session-specific (false)?
  "old_approx": "<short fragment from the existing skill body>", // patch
  "new_text": "<replacement fragment>",    // patch
  "rationale": "<why this is durable / what changed, in the user's language>"
}

PREFER PATCH OVER CREATE. If any of the top-K existing skill bodies in the
user prompt covers the same territory, return `decision="patch"` against
that skill_id. Only return `decision="create"` when no existing skill is
even adjacent.

REJECT (set `decision="skip"` with a short `rationale`) when the
conversation falls into ANY of these 6 categories:

1. ENV-FAILURE — missing binaries, fresh-install errors, "command not
   found", unconfigured credentials. These resolve themselves; capturing
   them produces self-imposed constraints that bite later.
2. NEGATIVE-CLAIM — "tool X is broken", "feature Y does not work". These
   harden into refusals the agent cites against itself for months.
3. TRANSIENT-ERROR — session-specific failures that resolved before the
   conversation ended. If retrying worked, the lesson is the retry pattern
   (under an existing umbrella), not the original failure.
4. ONE-OFF-TASK — "summarize today's market", "analyze this PR",
   "investigate THIS deploy". One-shot narratives, not a class of work.
5. PERSONAL-LIFE-DECISION — pets, family logistics, shopping, travel
   planning. The agent's skill library is for engineering / knowledge
   methodology, not life advice.
6. ECHO-OF-USER-INPUT — the body would only restate what the user already
   said, with no insight the model would not have produced unaided.

Additional hard rules:
- If `previously_rejected_skill_ids` is non-empty and the topic you would
  create overlaps with any of them, return skip.
- "Nothing to save." is a legitimate answer. Most conversations should
  return skip. A skip costs nothing; a bad create pollutes the library.
- Never include shell pipes to sh/bash/python in the body (`curl | bash`
  forbidden).
- Never include API keys, credentials, or secrets in any field.
- Use the dominant natural language of the user's messages for `name`,
  `description`, `body`, `reuse_scenarios`, and `rationale`. Preserve
  observed script (Simplified vs Traditional Chinese).
- Keep JSON keys, `skill_id`, code identifiers, commands, paths, API
  names, and literal tool names unchanged across languages.
- `body` for create must include sections equivalent to Purpose / When to
  use / Steps / Pitfalls (localize headings as appropriate).
"#;

/// Render the user-side prompt content. The system instruction is prepended
/// by the caller (allowing a user override of `REVIEW_SYSTEM` while still
/// reusing this user-content template).
///
/// `dedup_candidates` is a pre-formatted block of top-K existing skills with
/// their full (truncated) bodies; the model is told to prefer patching one
/// of them. `discarded_blacklist` lists recent `skill_discarded` ids so the
/// model can refuse near-duplicates the user has already rejected.
pub fn render_review_user_prompt(
    dedup_candidates: &str,
    discarded_blacklist: &str,
    extra_reject_categories: &[String],
    conversation: &str,
) -> String {
    let mut out = String::with_capacity(conversation.len() + dedup_candidates.len() + 512);

    out.push_str("Top-K existing skills (prefer patching one of these over creating a new one):\n");
    if dedup_candidates.trim().is_empty() {
        out.push_str("(none)\n");
    } else {
        out.push_str(dedup_candidates.trim_end());
        out.push('\n');
    }

    out.push_str("\nPreviously rejected (`skill_discarded`) ids — do NOT propose creating anything topically overlapping with these:\n");
    if discarded_blacklist.trim().is_empty() {
        out.push_str("(none)\n");
    } else {
        out.push_str(discarded_blacklist.trim_end());
        out.push('\n');
    }

    if !extra_reject_categories.is_empty() {
        out.push_str("\nAdditional reject categories defined by the user — treat these like the built-in 6:\n");
        for (idx, cat) in extra_reject_categories.iter().enumerate() {
            let _ = writeln!(out, "{}. {}", idx + 1, cat.trim());
        }
    }

    out.push_str("\nRecent conversation transcript (oldest first):\n");
    out.push_str(conversation);
    out.push_str("\n\nAuthoring language: infer from the user's messages above.\n");
    out.push_str("Emit ONE JSON object now.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_prompt_handles_empty_lists() {
        let s = render_review_user_prompt("", "", &[], "[user]: hi");
        assert!(s.contains("(none)"));
        assert!(s.contains("Recent conversation"));
        assert!(s.contains("[user]: hi"));
    }

    #[test]
    fn user_prompt_includes_extra_categories() {
        let cats = vec!["MEDICAL-ADVICE".to_string(), "LEGAL-ADVICE".to_string()];
        let s = render_review_user_prompt("", "", &cats, "[user]: hi");
        assert!(s.contains("MEDICAL-ADVICE"));
        assert!(s.contains("LEGAL-ADVICE"));
    }

    #[test]
    fn user_prompt_includes_blacklist_and_candidates() {
        let s = render_review_user_prompt(
            "- foo - desc\n--- body of foo ---",
            "- pet-advice",
            &[],
            "[user]: x",
        );
        assert!(s.contains("body of foo"));
        assert!(s.contains("pet-advice"));
    }
}
