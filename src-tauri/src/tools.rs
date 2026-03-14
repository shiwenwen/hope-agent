use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use tokio::process::Command;

const EXEC_TIMEOUT_SECS: u64 = 30;

// ── Provider Enum ─────────────────────────────────────────────────

/// Supported LLM provider types for tool schema adaptation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Anthropic,
    OpenAI,
}

// ── Tool Definition (provider-agnostic) ───────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool parameters
    pub parameters: Value,
}

impl ToolDefinition {
    /// Convert to Anthropic Messages API tool format:
    /// `{ "name": "...", "description": "...", "input_schema": {...} }`
    pub fn to_anthropic_schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.parameters,
        })
    }

    /// Convert to OpenAI Responses API tool format:
    /// `{ "type": "function", "name": "...", "description": "...", "parameters": {...} }`
    pub fn to_openai_schema(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "parameters": self.parameters,
        })
    }

    /// Convert to the schema format required by the given provider
    pub fn to_provider_schema(&self, provider: ToolProvider) -> Value {
        match provider {
            ToolProvider::Anthropic => self.to_anthropic_schema(),
            ToolProvider::OpenAI => self.to_openai_schema(),
        }
    }
}

// ── Tool Catalog ──────────────────────────────────────────────────

/// Returns the list of built-in tools available to the agent.
pub fn get_available_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "exec".into(),
            description: "Execute a shell command on the user's computer. Returns stdout and stderr.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the contents of a file at the specified path.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path to read"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file at the specified path. Creates parent directories if needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List files and directories in the specified path. Returns names with type indicators.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list. Defaults to current directory if not specified."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        },
    ]
}

/// Returns all tool schemas formatted for the given provider
pub fn get_tools_for_provider(provider: ToolProvider) -> Vec<Value> {
    get_available_tools()
        .iter()
        .map(|t| t.to_provider_schema(provider))
        .collect()
}

// ── Tool Execution (provider-agnostic) ────────────────────────────

/// Execute a tool by name with the given JSON arguments. Returns the result as a string.
pub async fn execute_tool(name: &str, args: &Value) -> Result<String> {
    match name {
        "exec" => tool_exec(args).await,
        "read_file" => tool_read_file(args).await,
        "write_file" => tool_write_file(args).await,
        "list_dir" => tool_list_dir(args).await,
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

// ── Tool Implementations ──────────────────────────────────────────

async fn tool_exec(args: &Value) -> Result<String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

    log::info!("Executing command: {}", command);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(EXEC_TIMEOUT_SECS),
        Command::new("sh").arg("-c").arg(command).output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Command timed out after {}s", EXEC_TIMEOUT_SECS))?
    .map_err(|e| anyhow::anyhow!("Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("[stderr] ");
        result.push_str(&stderr);
    }
    if result.is_empty() {
        result = format!("Command completed with exit code {}", exit_code);
    } else if exit_code != 0 {
        result.push_str(&format!("\n[exit code: {}]", exit_code));
    }

    // Truncate very long output to avoid overwhelming the context window
    const MAX_OUTPUT_LEN: usize = 16000;
    if result.len() > MAX_OUTPUT_LEN {
        result.truncate(MAX_OUTPUT_LEN);
        result.push_str("\n... (output truncated)");
    }

    Ok(result)
}

async fn tool_read_file(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    log::info!("Reading file: {}", path);

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    // Truncate very long files
    const MAX_FILE_LEN: usize = 32000;
    if content.len() > MAX_FILE_LEN {
        let truncated = &content[..MAX_FILE_LEN];
        Ok(format!("{}\n... (file truncated, {} bytes total)", truncated, content.len()))
    } else {
        Ok(content)
    }
}

async fn tool_write_file(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

    log::info!("Writing file: {}", path);

    // Create parent directories if needed
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create directories: {}", e))?;
    }

    tokio::fs::write(path, content)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {}", path, e))?;

    Ok(format!("Successfully wrote {} bytes to {}", content.len(), path))
}

async fn tool_list_dir(args: &Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    log::info!("Listing directory: {}", path);

    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read directory '{}': {}", path, e))?;

    let mut items = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().await?;
        let indicator = if file_type.is_dir() {
            "/"
        } else if file_type.is_symlink() {
            "@"
        } else {
            ""
        };
        items.push(format!("{}{}", name, indicator));
    }

    items.sort();

    if items.is_empty() {
        Ok(format!("Directory '{}' is empty", path))
    } else {
        Ok(items.join("\n"))
    }
}
