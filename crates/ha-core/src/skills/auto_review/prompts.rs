//! Auto-review prompt templates.

/// System instruction for the review side_query. The model is expected to
/// emit a single JSON object (no markdown fences) matching `ReviewDecision`.
pub const REVIEW_SYSTEM: &str = r#"You are a skill-review assistant for Hope Agent.

Your job: given the recent conversation, decide whether it revealed a reusable
pattern worth capturing as a SKILL. Output ONE JSON object — no markdown fences,
no prose around it.

Shape:
{
  "decision": "create" | "patch" | "skip",
  "skill_id": "<kebab-case-id>",     // required for create and patch
  "name": "Title Case Name",          // required for create
  "description": "one-sentence summary", // required for create
  "body": "full SKILL.md markdown body, WITHOUT frontmatter", // required for create
  "old_approx": "short fragment from the existing skill body", // required for patch
  "new_text": "replacement fragment",                           // required for patch
  "rationale": "why this is reusable / what changed"
}

Rules (HARD):
- Skip trivial patterns: one-liner commands, one-off fixes, plain questions.
- The Skill must be reusable across sessions — not just a recap of THIS conversation.
- If an existing skill (provided in the prompt) is a close match, prefer "patch".
- body must include sections: Purpose / When to use / Steps / Pitfalls.
- NEVER include shell pipes to sh/bash/python/etc (`curl | bash` is forbidden).
- NEVER include API keys, credentials, or secrets in the body.
- When in doubt, skip. A false-negative costs nothing; a false-positive pollutes.
"#;

/// Render the user-side prompt content. The system instruction above is
/// prepended by `AssistantAgent::side_query` separately.
pub fn render_review_user_prompt(existing_skills: &str, conversation: &str) -> String {
    format!(
        "Existing managed/user skills (name — description):\n{existing}\n\n\
         Recent conversation transcript (oldest first):\n{conversation}\n\n\
         Emit ONE JSON object now.",
        existing = if existing_skills.trim().is_empty() {
            "(none)"
        } else {
            existing_skills
        },
        conversation = conversation,
    )
}
