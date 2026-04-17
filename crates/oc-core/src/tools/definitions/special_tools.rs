use serde_json::json;

use super::super::{TOOL_ACP_SPAWN, TOOL_IMAGE_GENERATE, TOOL_SUBAGENT, TOOL_TEAM, TOOL_TOOL_SEARCH};
use super::types::ToolDefinition;

/// Returns the subagent tool definition (conditionally injected when enabled).
pub fn get_subagent_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBAGENT.into(),
        description: "Spawn and manage sub-agents to delegate tasks. Sub-agents run asynchronously — their results are automatically pushed to you when complete. Use steer to redirect a running sub-agent. Use check(wait=true) as fallback if you need to actively wait for a result.".into(),
        internal: false,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "check", "list", "result", "kill", "kill_all", "steer", "batch_spawn", "wait_all", "spawn_and_wait"],
                    "description": "Action: spawn (delegate task), check (poll/wait), list (all runs), result (full output), kill (terminate one), kill_all (terminate all), steer (redirect running sub-agent), batch_spawn (spawn multiple), wait_all (wait for multiple), spawn_and_wait (spawn + auto-background on timeout)"
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the sub-agent (required for spawn)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent to delegate to (default: 'default')"
                },
                "run_id": {
                    "type": "string",
                    "description": "Run ID (for check/result/kill/steer)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds for spawn (default 300, max 1800)"
                },
                "wait": {
                    "type": "boolean",
                    "description": "For check: block until sub-agent completes (default false). Use as fallback if push notification was missed."
                },
                "wait_timeout": {
                    "type": "integer",
                    "description": "For check with wait=true: max seconds to wait (default 60, max 300)"
                },
                "model": {
                    "type": "string",
                    "description": "Model override: 'provider_id/model_id'"
                },
                "message": {
                    "type": "string",
                    "description": "For steer: message to inject into the running sub-agent to redirect its behavior"
                },
                "label": {
                    "type": "string",
                    "description": "For spawn: display label for tracking this run (also usable in kill to target by label)"
                },
                "tasks": {
                    "type": "array",
                    "description": "For batch_spawn: array of task objects [{task, agent_id?, label?, timeout_secs?, model?}]",
                    "items": {
                        "type": "object",
                        "properties": {
                            "task": { "type": "string" },
                            "agent_id": { "type": "string" },
                            "label": { "type": "string" },
                            "timeout_secs": { "type": "integer" },
                            "model": { "type": "string" }
                        },
                        "required": ["task"]
                    }
                },
                "run_ids": {
                    "type": "array",
                    "description": "For wait_all: array of run IDs to wait for",
                    "items": { "type": "string" }
                },
                "files": {
                    "type": "array",
                    "description": "For spawn: file attachments to pass to the sub-agent",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "File name" },
                            "content": { "type": "string", "description": "File content (UTF-8 text or base64 encoded)" },
                            "mime_type": { "type": "string", "description": "MIME type (default: text/plain)" },
                            "encoding": { "type": "string", "enum": ["utf8", "base64"], "description": "Content encoding (default: utf8)" }
                        },
                        "required": ["name", "content"]
                    }
                },
                "foreground_timeout": {
                    "type": "integer",
                    "description": "For spawn_and_wait: seconds to wait before auto-backgrounding (default 30, max 120). If the sub-agent completes within this time, result is returned inline."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

/// Get the ACP spawn tool definition (conditionally injected).
pub fn get_acp_spawn_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_ACP_SPAWN.into(),
        description: "Spawn and manage external ACP agents (Claude Code, Codex CLI, Gemini CLI, etc.). External agents run as separate processes with their own tools, context, and capabilities. Use for tasks that benefit from a specialized external coding agent.".into(),
        internal: false,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "check", "list", "result", "kill", "kill_all", "steer", "backends"],
                    "description": "Action: spawn (start external agent), check (poll/wait), list (all runs), result (full output), kill (terminate), kill_all (terminate all), steer (send follow-up), backends (list available)"
                },
                "backend": {
                    "type": "string",
                    "description": "ACP backend ID (e.g. 'claude-code', 'codex-cli', 'gemini-cli'). Required for spawn."
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the external agent (required for spawn)"
                },
                "run_id": {
                    "type": "string",
                    "description": "Run ID (for check/result/kill/steer)"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the external agent"
                },
                "model": {
                    "type": "string",
                    "description": "Model override for the external agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 600, max 3600)"
                },
                "message": {
                    "type": "string",
                    "description": "Follow-up message to send (for steer action)"
                },
                "wait": {
                    "type": "boolean",
                    "description": "For check: block until completion (default false)"
                },
                "label": {
                    "type": "string",
                    "description": "Optional label for tracking"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

/// Get the tool_search meta-tool definition.
/// This tool enables on-demand discovery of deferred tool schemas.
pub fn get_tool_search_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TOOL_SEARCH.into(),
        description: "Search for available tools by keyword query. Returns full tool schemas \
            for matched tools. Use this to discover tools not listed in the main tool catalog."
            .into(),
        internal: true,
        deferred: false,
        always_load: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query: use 'select:name1,name2' for exact match, or keywords for fuzzy search"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (default 5, max 20)"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// Returns the image_generate tool definition (static fallback for is_internal_tool etc.).
pub fn get_image_generate_tool() -> ToolDefinition {
    get_image_generate_tool_dynamic(&crate::tools::image_generate::ImageGenConfig::default())
}

/// Returns the image_generate tool definition with dynamic description based on enabled providers.
pub fn get_image_generate_tool_dynamic(
    config: &crate::tools::image_generate::ImageGenConfig,
) -> ToolDefinition {
    use crate::tools::image_generate;

    // Build available models list from enabled providers
    let enabled: Vec<_> = config
        .providers
        .iter()
        .filter(|p| p.enabled && p.api_key.as_ref().map_or(false, |k| !k.is_empty()))
        .collect();

    let models_desc = if enabled.is_empty() {
        "No models configured".to_string()
    } else {
        enabled
            .iter()
            .map(|p| {
                let model = image_generate::effective_model(p);
                let display = image_generate::provider_display_name(p);
                format!("{} ({})", model, display)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    // Build dynamic capability summaries from enabled providers
    let mut edit_providers: Vec<String> = Vec::new();
    let mut multi_image_providers: Vec<String> = Vec::new();
    let mut ar_providers: Vec<String> = Vec::new();
    let mut res_providers: Vec<String> = Vec::new();
    let mut max_n: u32 = 4;

    for p in &enabled {
        if let Some(impl_) = image_generate::resolve_provider(&p.id) {
            let caps = impl_.capabilities();
            let name = impl_.display_name().to_string();
            if caps.edit.enabled {
                let detail = if caps.edit.max_input_images > 1 {
                    format!("{} (up to {})", name, caps.edit.max_input_images)
                } else {
                    name.clone()
                };
                edit_providers.push(detail);
                if caps.edit.max_input_images > 1 {
                    multi_image_providers.push(name.clone());
                }
            }
            if caps.generate.supports_aspect_ratio {
                ar_providers.push(name.clone());
            }
            if caps.generate.supports_resolution {
                res_providers.push(name.clone());
            }
            max_n = max_n.max(caps.generate.max_count);
            if caps.edit.enabled {
                max_n = max_n.max(caps.edit.max_count);
            }
        }
    }

    let edit_desc = if edit_providers.is_empty() {
        String::new()
    } else {
        format!(
            " Supports image editing with reference images ({}).",
            edit_providers.join(", ")
        )
    };

    let description = format!(
        "Generate or edit images from text descriptions. \
         Available models (priority order): {}.{} \
         Use action='list' to see all providers with detailed capabilities. \
         Images are saved to disk and returned for visual inspection. \
         Default: auto — tries models in order with automatic failover on failure.",
        models_desc, edit_desc
    );

    let model_param_desc = if enabled.is_empty() {
        "Specify a model. Default: auto.".to_string()
    } else {
        let model_list = enabled
            .iter()
            .map(|p| format!("'{}'", image_generate::effective_model(p)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Specify a model. Available: {}. Default: auto (uses priority order with failover).",
            model_list
        )
    };

    // Dynamic descriptions for parameters
    let image_desc = if edit_providers.is_empty() {
        "Path or URL of a reference/input image for editing.".to_string()
    } else {
        format!(
            "Path or URL of a reference/input image for editing. Supported by: {}.",
            edit_providers.join(", ")
        )
    };

    let images_desc = if multi_image_providers.is_empty() {
        "Array of paths/URLs for multiple reference images (max 5 total).".to_string()
    } else {
        format!(
            "Array of paths/URLs for multiple reference images (max 5 total). Supported by: {}.",
            multi_image_providers.join(", ")
        )
    };

    let ar_desc = if ar_providers.is_empty() {
        "Aspect ratio hint: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, or 21:9.".to_string()
    } else {
        format!(
            "Aspect ratio hint: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, or 21:9. Supported by: {}.",
            ar_providers.join(", ")
        )
    };

    let res_desc = if res_providers.is_empty() {
        "Output resolution: 1K=1024px, 2K=2048px, 4K=4096px. Auto-inferred from input images when editing.".to_string()
    } else {
        format!(
            "Output resolution: 1K=1024px, 2K=2048px, 4K=4096px. Supported by: {}. Auto-inferred from input images when editing.",
            res_providers.join(", ")
        )
    };

    ToolDefinition {
        name: TOOL_IMAGE_GENERATE.into(),
        description,
        internal: false,
        deferred: false,
        always_load: false,
        async_capable: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["generate", "list"],
                    "description": "Action: 'generate' (default) creates images, 'list' shows available providers and capabilities."
                },
                "prompt": {
                    "type": "string",
                    "description": "Text description of the image to generate or edit"
                },
                "image": {
                    "type": "string",
                    "description": image_desc
                },
                "images": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": images_desc
                },
                "size": {
                    "type": "string",
                    "description": "Image dimensions (e.g. '1024x1024', '1024x1536', '1536x1024', '1024x1792', '1792x1024'). Default: 1024x1024"
                },
                "aspectRatio": {
                    "type": "string",
                    "description": ar_desc
                },
                "resolution": {
                    "type": "string",
                    "enum": ["1K", "2K", "4K"],
                    "description": res_desc
                },
                "n": {
                    "type": "integer",
                    "description": format!("Number of images to generate (1-{} depending on provider, default 1)", max_n),
                    "minimum": 1,
                    "maximum": max_n
                },
                "model": {
                    "type": "string",
                    "description": model_param_desc
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        }),
    }
}

/// Returns the team tool definition (deferred — discovered via tool_search).
pub fn get_team_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_TEAM.into(),
        description: "Create and manage agent teams for coordinated multi-agent parallel work. Teams have named members (each backed by a subagent), a shared task board, and inter-member messaging. Use for complex tasks that benefit from parallel specialization (e.g., frontend + backend + tester).\n\nBefore creating a team, call `action=\"list_templates\"` to see user-configured presets that may already match your task. Use `template=\"<templateId>\"` in `create` to spawn from a preset (each member can be bound to a specific Agent with its own model/identity). Fall back to inline `members=[{name, task, agent_id?, role?, description?}]` when no preset fits.".into(),
        internal: false,
        deferred: true,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "dissolve", "add_member", "remove_member",
                             "send_message", "create_task", "update_task", "list_tasks",
                             "list_members", "status", "pause", "resume", "list_templates"],
                    "description": "Team action to perform. `list_templates` returns user-configured preset templates (no other arguments needed)."
                },
                "team_id": {
                    "type": "string",
                    "description": "Team ID (required for all actions except create and list_templates)"
                },
                "name": {
                    "type": "string",
                    "description": "Team name (for create) or member name (for add_member)"
                },
                "description": {
                    "type": "string",
                    "description": "Team description (for create) or member role identity description (for add_member — injected into the member's subagent system prompt)."
                },
                "members": {
                    "type": "array",
                    "description": "Initial members for create: [{name, agent_id?, role?, task, model?, description?}]. When used together with `template`, inline members override the template's defaults.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "agent_id": { "type": "string" },
                            "role": { "type": "string", "enum": ["worker", "reviewer"] },
                            "task": { "type": "string" },
                            "model": { "type": "string" },
                            "description": { "type": "string", "description": "Role identity injected into this member's subagent system prompt" }
                        },
                        "required": ["name", "task"]
                    }
                },
                "template": {
                    "type": "string",
                    "description": "Template ID (or case-insensitive name) for create. Call action=\"list_templates\" first to discover available presets."
                },
                "agent_id": { "type": "string", "description": "Agent ID for add_member" },
                "role": { "type": "string", "enum": ["worker", "reviewer"], "description": "Member role" },
                "task": { "type": "string", "description": "Task description for add_member" },
                "member_id": { "type": "string", "description": "Member ID for remove_member" },
                "to": { "type": "string", "description": "Recipient name or '*' for broadcast (send_message)" },
                "content": { "type": "string", "description": "Message or task content" },
                "task_id": { "type": "integer", "description": "Task ID for update_task" },
                "status": { "type": "string", "description": "Task status filter or update value" },
                "owner": { "type": "string", "description": "Task owner member name" },
                "priority": { "type": "integer", "description": "Task priority (lower = higher)" },
                "blocked_by": { "type": "array", "items": { "type": "integer" }, "description": "Task IDs that block this task" },
                "column": { "type": "string", "enum": ["backlog", "todo", "doing", "review", "done"], "description": "Kanban column" },
                "model": { "type": "string", "description": "Model override for member" }
            },
            "required": ["action"]
        }),
    }
}
