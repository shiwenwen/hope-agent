Help migrate context from one AI assistant to another. Review our past conversations and summarize what has been learned about the user, then emit the result in the strict format below.

Referent rules:
- Do NOT use first-person or second-person pronouns ("I / my / you / your"). Refer to the person as "the user" or neutral phrasing.
- Preserve the user's exact wording whenever possible, especially for preferences and explicit instructions.

Categories (use the value in the `type` field):
- `user`       — demographic / profile facts: name, occupation, education, location, long-term relationships
- `feedback`   — explicit rules the user wants followed going forward: preferences, taboos, corrections, style instructions
- `project`    — dated events, ongoing projects, near-term plans with concrete context
- `reference`  — external systems, dashboards, links, documents the user pointed to

Output format (STRICT — plain JSON, nothing else):
- Return ONLY a JSON array. No prose, no explanation, no Markdown code fence.
- Each element is an object with these fields:
  - `content` (string, required): a self-contained sentence. Embed the evidence inline, e.g. `Evidence: user said "call me Alex".`
  - `type` (string, required): one of `user` | `feedback` | `project` | `reference`. These four values MUST stay in English even if the rest of the content is in another language — the importer only recognizes these exact tokens.
  - `tags` (array of 1–4 short lowercase English words, optional)

Example:
[
  {
    "content": "The user prefers to be called Alex. Evidence: user said \"call me Alex\".",
    "type": "user",
    "tags": ["name"]
  },
  {
    "content": "The user requires replies in Chinese by default. Evidence: user said \"always reply in Chinese\".",
    "type": "feedback",
    "tags": ["language", "communication"]
  }
]

Rules:
1. Every memory is one self-contained sentence; downstream consumers read items in isolation.
2. Keep evidence inline with the `Evidence:` prefix and quote the original phrasing.
3. `type` MUST be one of the four enum values above — any other value will be silently coerced to `user`.
4. Return `[]` if nothing memorable has been shared.
5. Output ONLY the JSON array. Do not wrap it in a code fence. Do not add any leading or trailing text.
