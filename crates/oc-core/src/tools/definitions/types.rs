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
        }
    }

    pub fn to_anthropic_schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.parameters,
        })
    }

    pub fn to_openai_schema(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "parameters": self.parameters,
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
];

/// Check if a tool name is a core tool (always loaded).
pub fn is_core_tool(name: &str) -> bool {
    CORE_TOOL_NAMES.contains(&name)
}
