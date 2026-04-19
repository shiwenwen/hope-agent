use serde_json::json;

use super::super::{TOOL_TASK_CREATE, TOOL_TASK_LIST, TOOL_TASK_UPDATE};
use super::types::ToolDefinition;

pub fn get_task_create_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TASK_CREATE.into(),
        description: "Create a batch of trackable todos for the current session. \
Returns the full task list as JSON.\n\n\
Use this tool proactively when:\n\
- A task requires 3+ distinct steps or actions\n\
- Work is non-trivial and involves multiple operations (refactors, migrations, multi-file edits)\n\
- The user provides multiple tasks as a list (numbered, bulleted, or comma-separated)\n\
- You receive new user instructions — capture requirements as todos immediately\n\
- You start working on a task — mark it in_progress via task_update BEFORE beginning\n\
- You complete a task — mark it completed and append any newly discovered follow-ups\n\n\
Do NOT use this tool when:\n\
- There is only a single, straightforward action\n\
- The task is purely conversational or informational\n\
- The work can be completed in ≤3 trivial steps\n\n\
Batching rule: pass ALL todos in one call as an array via `tasks: [...]`. \
Do NOT chain multiple task_create calls to build a list — create them all at once.\n\n\
Each task has:\n\
- content: imperative form (\"Run tests\", \"Refactor parseConfig\")\n\
- activeForm (optional): present-continuous form (\"Running tests\", \"Refactoring parseConfig\") \
shown in the UI when this task's status is in_progress"
            .into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Array of tasks to create in one call. Always batch all todos here — do not chain multiple task_create calls.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Imperative form: what needs to be done. e.g. \"Refactor parseConfig to support hot reload\"."
                            },
                            "activeForm": {
                                "type": "string",
                                "description": "Present continuous form shown when this task is in_progress. e.g. \"Refactoring parseConfig to support hot reload\". If omitted, UI falls back to content."
                            }
                        },
                        "required": ["content"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["tasks"],
            "additionalProperties": false
        }),
    }
}

pub fn get_task_update_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TASK_UPDATE.into(),
        description: "Update an existing task by id. Returns the full task list.\n\
Lifecycle: pending → in_progress → completed. Only ONE task should be in_progress at a time. \
Mark completed only when fully done, and call immediately after finishing (do not batch completions)."
            .into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "description": "Task id." },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "New status for the task."
                },
                "content": {
                    "type": "string",
                    "description": "New imperative description (optional)."
                },
                "activeForm": {
                    "type": "string",
                    "description": "New present-continuous form shown when in_progress (optional)."
                }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    }
}

pub fn get_task_list_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TASK_LIST.into(),
        description: "List all tasks in the current session as JSON.".into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}
