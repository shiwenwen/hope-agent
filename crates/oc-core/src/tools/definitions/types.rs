use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ToolProvider;

// ── Tool Definition (provider-agnostic) ───────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool parameters
    pub parameters: Value,
    /// Internal capability tools never require user approval.
    /// These are autonomous agent abilities (memory, cron, notification, read-only analysis)
    /// rather than system-interacting tools (exec, write, edit, etc.)
    #[serde(default)]
    pub internal: bool,
    /// Whether this tool is deferred (schema not sent to LLM by default).
    /// Deferred tools are discoverable via `tool_search`.
    #[serde(default)]
    pub deferred: bool,
    /// When true, always load this tool even when deferred loading is enabled.
    #[serde(default)]
    pub always_load: bool,
    /// Async-capable tools may be backgrounded: the model sets `run_in_background: true`
    /// in the arguments and the tool returns an immediate synthetic job_id. The real
    /// execution continues in a tokio task and the result is delivered to the parent
    /// session via the async_jobs injection pipeline when the session becomes idle.
    /// Also participates in the sync-execution auto-background budget.
    #[serde(default)]
    pub async_capable: bool,
}

impl ToolDefinition {
    #[allow(dead_code)]
    fn new(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            internal: false,
            deferred: false,
            always_load: false,
            async_capable: false,
        }
    }

    #[allow(dead_code)]
    fn new_internal(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            internal: true,
            deferred: false,
            always_load: false,
            async_capable: false,
        }
    }

    /// When this tool is async-capable, inject an optional `run_in_background`
    /// parameter into the tool's JSON schema so the model can discover and opt
    /// into background execution. Idempotent.
    fn augmented_parameters(&self) -> Value {
        if !self.async_capable {
            return self.parameters.clone();
        }
        let mut params = self.parameters.clone();
        let Some(obj) = params.as_object_mut() else {
            return params;
        };
        let props = obj
            .entry("properties".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let Some(props_obj) = props.as_object_mut() else {
            return params;
        };
        if props_obj.contains_key("run_in_background") {
            return params;
        }
        props_obj.insert(
            "run_in_background".to_string(),
            json!({
                "type": "boolean",
                "description": "Run in background and return immediately with a job_id. Set to true when: (1) the task is expected to take more than a few seconds (long builds, lengthy web searches, image generation, network-heavy operations), AND (2) you can make progress on other things while it runs, OR (3) the user explicitly asked you to continue working in parallel. Set to false (default) when you need the result to decide your very next step. Results are auto-injected into the conversation when ready; you can also call job_status(job_id, block=true) to actively wait."
            }),
        );
        params
    }

    pub fn to_anthropic_schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.augmented_parameters(),
        })
    }

    pub fn to_openai_schema(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "parameters": self.augmented_parameters(),
        })
    }

    pub fn to_provider_schema(&self, provider: ToolProvider) -> Value {
        match provider {
            ToolProvider::Anthropic => self.to_anthropic_schema(),
            ToolProvider::OpenAI => self.to_openai_schema(),
        }
    }
}

// ── Tool Catalog ──────────────────────────────────────────────────

/// Core tools that are always loaded (never deferred).
pub(crate) const CORE_TOOL_NAMES: &[&str] = &[
    "exec",
    "process",
    "read",
    "write",
    "edit",
    "ls",
    "grep",
    "find",
    "apply_patch",
    "ask_user_question",
    "task_create",
    "task_update",
    "task_list",
];

/// Check if a tool name is a core tool (always loaded).
pub fn is_core_tool(name: &str) -> bool {
    CORE_TOOL_NAMES.contains(&name)
}
