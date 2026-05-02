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
  "name": "human-readable skill name in the user's language", // required for create
  "description": "one-sentence summary in the user's language", // required for create
  "body": "full SKILL.md markdown body, WITHOUT frontmatter, in the user's language", // required for create
  "old_approx": "short fragment from the existing skill body", // required for patch
  "new_text": "replacement fragment",                           // required for patch
  "rationale": "why this is reusable / what changed, in the user's language"
}

Rules (HARD):
- Skip trivial patterns: one-liner commands, one-off fixes, plain questions.
- The Skill must be reusable across sessions — not just a recap of THIS conversation.
- If an existing skill (provided in the prompt) is a close match, prefer "patch".
- Use the dominant natural language of the user's messages in the transcript for all human-facing generated skill content: name, description, body, rationale, and patch text.
- If the user language is mixed, use the language of the latest substantive user request. If the user writes Chinese, preserve the observed script (Simplified vs Traditional).
- Keep JSON keys, skill_id, code identifiers, commands, paths, API names, and literal tool names unchanged.
- body must include sections equivalent to Purpose / When to use / Steps / Pitfalls; localize those section headings when appropriate.
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
         Authoring language: infer it from the user's messages above and write \
         the skill's human-facing content in that language.\n\n\
         Emit ONE JSON object now.",
        existing = if existing_skills.trim().is_empty() {
            "(none)"
        } else {
            existing_skills
        },
        conversation = conversation,
    )
}
