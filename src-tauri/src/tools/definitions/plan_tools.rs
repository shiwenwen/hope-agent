use serde_json::json;

use super::super::{TOOL_AMEND_PLAN, TOOL_PLAN_QUESTION, TOOL_SUBMIT_PLAN, TOOL_UPDATE_PLAN_STEP};
use super::types::ToolDefinition;

/// Tool for updating plan step status (conditionally injected during Executing state).
pub fn get_plan_step_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_UPDATE_PLAN_STEP.into(),
        description: "Update the status of a plan step during plan execution. Call this after starting or completing each step to track progress in the Plan panel.".into(),
        internal: true,
        deferred: false,
        always_load: false,
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

/// Tool for sending structured questions to the user during plan creation.
pub fn get_plan_question_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_PLAN_QUESTION.into(),
        description: "Send structured questions to the user during plan creation. Each question includes suggested options that render as an interactive UI. The user can select options or provide custom input. Use this to clarify requirements, confirm design decisions, and gather preferences before submitting the final plan.".into(),
        internal: true,
        deferred: false,
        always_load: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "List of questions to ask the user",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question_id": {
                                "type": "string",
                                "description": "Unique identifier for this question (e.g. 'q_framework', 'q_scope')"
                            },
                            "text": {
                                "type": "string",
                                "description": "The question text to display to the user"
                            },
                            "options": {
                                "type": "array",
                                "description": "Suggested options for the user to choose from (2-5 recommended)",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "value": { "type": "string", "description": "Option identifier" },
                                        "label": { "type": "string", "description": "Display text" },
                                        "description": { "type": "string", "description": "Additional explanation" },
                                        "recommended": { "type": "boolean", "description": "Mark as recommended option (renders with ★ badge)", "default": false }
                                    },
                                    "required": ["value", "label"]
                                }
                            },
                            "allow_custom": {
                                "type": "boolean",
                                "description": "Whether to show a custom input field (default: true)",
                                "default": true
                            },
                            "multi_select": {
                                "type": "boolean",
                                "description": "Whether the user can select multiple options (default: false)",
                                "default": false
                            },
                            "template": {
                                "type": "string",
                                "description": "Question template category for specialized UI rendering: 'scope', 'tech_choice', 'priority'",
                                "enum": ["scope", "tech_choice", "priority"]
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
        description: "Submit the final implementation plan after gathering requirements through plan_question. The plan should be structured as markdown with phased checklists. This transitions the plan to Review mode where the user can approve and start execution.".into(),
        internal: true,
        deferred: false,
        always_load: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title for the plan (e.g. 'Refactor Auth Module')"
                },
                "content": {
                    "type": "string",
                    "description": "Full plan content in markdown format. Must include: ## Background section, then ### Phase N: <title> headers with - [ ] checklist items"
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
