use serde_json::json;

use super::super::{TOOL_ASK_USER_QUESTION, TOOL_ENTER_PLAN_MODE, TOOL_SUBMIT_PLAN};
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

/// Tool for asking the user structured questions at any point in a conversation.
///
/// Available in any conversation (not only Plan Mode). Supports rich
/// markdown/image previews, optional per-question timeouts with default
/// fall-backs, IM channel native buttons, and persistence across app restarts.
pub fn get_ask_user_question_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ASK_USER_QUESTION.into(),
        description:
            "Ask the user one or more structured questions with multiple-choice options. \
Use this whenever you need to clarify requirements, pick between approaches, or confirm a \
decision before continuing. Each question renders as an interactive UI in the desktop app, \
as native buttons in IM channels that support them (Telegram, Slack, Feishu, QQ, Discord, \
LINE, Google Chat), and as a text fallback (reply 1a/1b/2a) in the rest. \n\n\
Guidelines: 1–4 questions per call, 2–4 options per question. Prefer single-select. Mark your \
recommended choice as the first option with '(Recommended)' in the label. Use `preview` for \
mockups, code comparisons or diagram snippets. Set `default_values` + `timeout_secs` only when \
the answer can safely fall back; they take effect only if ask-user auto-timeout is enabled in \
settings. Do NOT use this tool to ask 'is my plan ready?' — in Plan Mode use `submit_plan` instead."
                .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::Interaction,
        },
        internal: true,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "List of questions to ask the user (1-4 recommended)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question_id": {
                                "type": "string",
                                "description": "Unique identifier for this question (e.g. 'q_framework', 'q_scope')"
                            },
                            "text": {
                                "type": "string",
                                "description": "The question text to display to the user. Should end with '?'."
                            },
                            "header": {
                                "type": "string",
                                "description": "Very short chip/tag label (max ~12 chars) shown next to the question, e.g. 'Auth', 'Framework', 'Scope'"
                            },
                            "input_kind": {
                                "type": "string",
                                "description": "Primary input shape (frontend rendering hint; answer channel is unchanged). Omit for the default single/multi behavior. 'text'/'textarea' render a free-text box (answer comes back as the custom input) — use for open-ended discovery questions. 'direction-cards' renders each option as a rich VISUAL STYLE CARD (palette swatches + live 'Aa' type sample + mood + reference names) in the Design Space chat, degrading to a plain option list elsewhere — use it to let the user pick a visual design direction; each option must also carry a 'card' payload.",
                                "enum": ["single", "multi", "text", "textarea", "direction-cards"]
                            },
                            "options": {
                                "type": "array",
                                "description": "Suggested options (2-4 recommended). The UI also renders an explicit 'Other' choice that reveals a free-form custom input, so do not add your own duplicate Other option. Omit entirely for input_kind 'text'/'textarea'.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "value": { "type": "string", "description": "Option identifier" },
                                        "label": { "type": "string", "description": "Display text (1-5 words)" },
                                        "description": { "type": "string", "description": "Additional explanation of the option or its trade-offs" },
                                        "recommended": { "type": "boolean", "description": "Mark as recommended (renders with ★ badge). Put recommended option first.", "default": false },
                                        "preview": { "type": "string", "description": "Optional rich preview body for visual comparison: markdown (code/tables), image URL, or mermaid source. Displayed side-by-side with the option list." },
                                        "previewKind": { "type": "string", "description": "Preview kind: 'markdown' (default), 'image', or 'mermaid'", "enum": ["markdown", "image", "mermaid"] },
                                        "card": {
                                            "type": "object",
                                            "description": "Visual style-card payload — only for input_kind 'direction-cards'. Rendered as a rich card in the Design Space chat; ignored (option shows as a plain row) elsewhere.",
                                            "properties": {
                                                "palette": { "type": "array", "items": { "type": "string" }, "description": "4–6 swatch colors (hex/rgb/oklch), shown as a color row. Order suggestion: [bg, surface, border, muted, fg, accent]." },
                                                "displayFont": { "type": "string", "description": "Headline font stack (CSS font-family), used to render the live 'Aa' sample." },
                                                "bodyFont": { "type": "string", "description": "Body font stack (CSS font-family) for the secondary sample." },
                                                "mood": { "type": "string", "description": "One- or two-sentence mood blurb." },
                                                "references": { "type": "array", "items": { "type": "string" }, "description": "Up to 4 real-world exemplars (e.g. 'Linear', 'Stripe'), shown on the refs line." }
                                            }
                                        }
                                    },
                                    "required": ["value", "label"]
                                }
                            },
                            "allow_custom": {
                                "type": "boolean",
                                "description": "Whether to offer a free-form custom input through an explicit 'Other' choice. Currently always treated as true by the runtime regardless of the value sent — kept in the schema for forward compatibility.",
                                "default": true
                            },
                            "multi_select": {
                                "type": "boolean",
                                "description": "Whether the user can select multiple options (default: false)",
                                "default": false
                            },
                            "template": {
                                "type": "string",
                                "description": "Optional UI category: 'scope', 'tech_choice', 'priority'",
                                "enum": ["scope", "tech_choice", "priority"]
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "description": "Per-question timeout in seconds. Only takes effect when ask-user auto-timeout is enabled in settings. When exceeded, default_values are auto-applied. 0 or missing = use global default.",
                                "minimum": 0
                            },
                            "default_values": {
                                "type": "array",
                                "description": "Option values used automatically if the question times out. Ignored unless ask-user auto-timeout is enabled. Each entry must be an existing option value, or a free-form custom string.",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["question_id", "text", "options"]
                    }
                },
                "context": {
                    "type": "string",
                    "description": "Optional context text explaining why these questions are being asked"
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    }
}

/// Tool the model uses to proactively enter Plan Mode before tackling a
/// non-trivial task.
pub fn get_enter_plan_mode_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ENTER_PLAN_MODE.into(),
        description: "Enter Plan Mode to explore, gather context, and draft a written plan \
before doing the work. After entering, you can read files / search / ask the user clarifying \
questions, and you must call `submit_plan` with the finalized plan when ready.\n\n\
## When to Use\n\
Prefer entering plan mode for non-trivial tasks across all domains:\n\
- **Programming**: new features, multi-file refactors, architectural choices, performance work, \
anything touching 3+ files, or unclear requirements.\n\
- **Writing**: pieces longer than ~1000 words, multi-section docs, anything with structural \
decisions (outline, audience, tone).\n\
- **Research / investigation**: comparing 3+ sources or angles, building a structured argument.\n\
- **Information organization**: sorting/categorizing 50+ items, building a knowledge structure, \
designing a taxonomy.\n\
- **Decision support**: trade-offs across 3+ dimensions, multi-step decisions with downstream \
consequences.\n\
- **Whenever you would otherwise use ask_user_question to clarify the *approach*** (not just \
requirements) — plan mode lets you explore first and then present a vetted plan.\n\n\
## When NOT to Use\n\
- Single-step or trivial tasks (typo fixes, single-line changes, single function with clear \
requirements).\n\
- Pure Q&A or research lookups (use the Explore subagent or just answer directly).\n\
- The user gave very specific step-by-step instructions.\n\
- The work can be done in fewer than 3 steps.\n\n\
## Behavior\n\
Calling this tool surfaces a Yes/No prompt to the user. The user has the final say — if \
they accept, the session transitions to Planning state and the tool returns a success \
message; if they decline, the session stays in normal mode and the tool returns a \
message saying so. Only call this once per task; if the user declined, do not retry — \
proceed with the task directly."
            .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::PlanMode,
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "One-line reason explaining why this task benefits from a written plan. Shown to the user as context for the Yes/No prompt."
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Tool for submitting the final plan after interactive Q&A.
pub fn get_submit_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBMIT_PLAN.into(),
        description:
            "Submit the finalized plan as the design contract for the user to review and approve. \
The plan is a stable design document — once approved, it is frozen for the duration of execution. \
To revise an approved plan, exit plan mode and re-enter it.\n\n\
Recommended structure (any markdown is accepted, sections are guidance not enforcement):\n\
- Context — why this change is being made\n\
- Approach — the recommended approach (no alternatives needed)\n\
- Files — paths of critical files to be modified\n\
- Reuse — existing functions/utilities to reuse, with file paths\n\
- Verification — how to confirm the work was done correctly\n\n\
Do NOT include progress markers (no checkboxes, no status emojis, no \"TODO/DONE\" annotations). \
Progress is tracked separately via the task_create / task_update tools after the plan is approved."
                .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::PlanMode,
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title for the plan (e.g. 'Refactor Auth Module')"
                },
                "content": {
                    "type": "string",
                    "description": "Plan content in markdown. Free-form structure (Context / Approach / Files / Reuse / Verification recommended). Do not include progress markers — those belong in task_* tools."
                }
            },
            "required": ["title", "content"],
            "additionalProperties": false
        }),
    }
}
