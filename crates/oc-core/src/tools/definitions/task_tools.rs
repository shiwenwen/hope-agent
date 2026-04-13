use serde_json::json;

use super::super::{TOOL_TASK_CREATE, TOOL_TASK_LIST, TOOL_TASK_UPDATE};
use super::types::ToolDefinition;

pub fn get_task_create_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TASK_CREATE.into(),
        description: "Create a trackable task for the current session. Returns the full task list."
            .into(),
        internal: true,
        deferred: false,
        always_load: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Imperative description of what needs to be done."
                }
            },
            "required": ["content"],
            "additionalProperties": false
        }),
    }
}

pub fn get_task_update_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TASK_UPDATE.into(),
        description: "Update an existing task by id. Returns the full task list.".into(),
        internal: true,
        deferred: false,
        always_load: false,
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
                    "description": "New task description."
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
        parameters: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}
