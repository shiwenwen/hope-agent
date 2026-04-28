use serde_json::json;

use super::super::{
    TOOL_AMEND_PLAN, TOOL_ASK_USER_QUESTION, TOOL_SUBMIT_PLAN, TOOL_UPDATE_PLAN_STEP,
};
use super::types::ToolDefinition;

/// Tool for updating plan step status (conditionally injected during Executing state).
pub fn get_plan_step_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_UPDATE_PLAN_STEP.into(),
        description: "Update the status of a plan step during plan execution. Call this after starting or completing each step to track progress in the Plan panel.".into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "step_index": {
                    "type": "integer",
                    "description": "Zero-based index of the plan step to update"
                },
                "status": {
                    "type": "string",
                    "enum": ["in_progress", "completed", "skipped", "failed"],
                    "description": "New status for the step"
                }
            },
            "required": ["step_index", "status"],
            "additionalProperties": false
        }),
    }
}

/// Tool for asking the user structured questions at any point in a conversation.
///
/// Available in any conversation (not only Plan Mode). Supports rich
/// markdown/image previews, per-question timeouts with default fall-backs,
/// IM channel native buttons, and persistence across app restarts.
pub fn get_ask_user_question_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ASK_USER_QUESTION.into(),
        description: "Ask the user one or more structured questions with multiple-choice options. \
Use this whenever you need to clarify requirements, pick between approaches, or confirm a \
decision before continuing. Each question renders as an interactive UI in the desktop app, \
as native buttons in IM channels that support them (Telegram, Slack, Feishu, QQ, Discord, \
LINE, Google Chat), and as a text fallback (reply 1a/1b/2a) in the rest. \n\n\
Guidelines: 1–4 questions per call, 2–4 options per question. Prefer single-select. Mark your \
recommended choice as the first option with '(Recommended)' in the label. Use `preview` for \
mockups, code comparisons or diagram snippets. Set `default_values` + `timeout_secs` when the \
answer can safely fall back (useful for cron / background / IM async flows). Do NOT use this \
tool to ask 'is my plan ready?' — in Plan Mode use `submit_plan` instead."
            .into(),
        internal: true,
        deferred: false,
        always_load: true,
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
                            "options": {
                                "type": "array",
                                "description": "Suggested options (2-4 recommended). A free-form custom input is also rendered alongside the options so the user can reply with a value you didn't list.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "value": { "type": "string", "description": "Option identifier" },
                                        "label": { "type": "string", "description": "Display text (1-5 words)" },
                                        "description": { "type": "string", "description": "Additional explanation of the option or its trade-offs" },
                                        "recommended": { "type": "boolean", "description": "Mark as recommended (renders with ★ badge). Put recommended option first.", "default": false },
                                        "preview": { "type": "string", "description": "Optional rich preview body for visual comparison: markdown (code/tables), image URL, or mermaid source. Displayed side-by-side with the option list." },
                                        "previewKind": { "type": "string", "description": "Preview kind: 'markdown' (default), 'image', or 'mermaid'", "enum": ["markdown", "image", "mermaid"] }
                                    },
                                    "required": ["value", "label"]
                                }
                            },
                            "allow_custom": {
                                "type": "boolean",
                                "description": "Whether to show a free-form custom input field. Currently always treated as true by the runtime regardless of the value sent — kept in the schema for forward compatibility.",
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
                                "description": "Per-question timeout in seconds. When exceeded, default_values are auto-applied. 0 or missing = use global default.",
                                "minimum": 0
                            },
                            "default_values": {
                                "type": "array",
                                "description": "Option values used automatically if the question times out. Each entry must be an existing option value, or a free-form custom string.",
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

/// Tool for submitting the final plan after interactive Q&A.
pub fn get_submit_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBMIT_PLAN.into(),
        description: "Submit the final implementation plan after gathering requirements through ask_user_question. The plan should be structured as markdown with concise sections and regular ordered/unordered lists, not checkbox task lists. This transitions the plan to Review mode where the user can approve and start execution.".into(),
        internal: true,
        deferred: false,
        always_load: false,
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
                    "description": "Full plan content in markdown format. Must include concise context, major implementation steps as headings or regular ordered/unordered list items, and verification. Do not use markdown checkbox items (- [ ])."
                }
            },
            "required": ["title", "content"],
            "additionalProperties": false
        }),
    }
}

/// Tool for amending the plan during execution (insert/delete/update steps).
pub fn get_amend_plan_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_AMEND_PLAN.into(),
        description: "Modify the current plan during execution. Use this when you discover the plan needs changes (new steps needed, steps should be removed, or step descriptions need updating). Available actions: insert (add a new step), delete (remove a pending step), update (modify a pending step's title/description).".into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The amendment action to perform",
                    "enum": ["insert", "delete", "update"]
                },
                "step_index": {
                    "type": "integer",
                    "description": "Target step index (required for delete and update actions)"
                },
                "after_index": {
                    "type": "integer",
                    "description": "Insert new step after this index (for insert action). Omit to append to end."
                },
                "title": {
                    "type": "string",
                    "description": "Step title (required for insert, optional for update)"
                },
                "description": {
                    "type": "string",
                    "description": "Step description (optional)"
                },
                "phase": {
                    "type": "string",
                    "description": "Phase name (optional, defaults to 'Amended' for insert)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}
