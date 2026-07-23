use serde_json::json;

use super::super::{
    TOOL_ACP_SPAWN, TOOL_AUDIO_GENERATE, TOOL_IMAGE_GENERATE, TOOL_SUBAGENT, TOOL_TEAM,
    TOOL_TOOL_SEARCH, TOOL_WORKFLOW,
};
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

/// Returns the subagent tool definition (conditionally injected when enabled).
pub fn get_subagent_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SUBAGENT.into(),
        description: "Spawn and manage sub-agents to delegate tasks. Sub-agents run asynchronously and keep a stable thread across immutable run attempts. Use send to follow up: it steers an active attempt or resumes a terminal one in the same child conversation. Results are durably pushed when complete; use check(wait=true) only as a fallback.".into(),
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Agents",
        },
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "send", "resume", "check", "list", "result", "kill", "kill_all", "steer", "batch_spawn", "wait_all", "spawn_and_wait"],
                    "description": "Action: spawn (delegate task), send (canonical follow-up: steer active or resume terminal), resume/steer (compatibility aliases), check (poll/wait), list (all runs), result (full output), kill/kill_all, batch_spawn, wait_all, spawn_and_wait"
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the sub-agent (required for spawn and resume; resume treats it as the follow-up task)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent to delegate to (defaults to the main Agent)"
                },
                "run_id": {
                    "type": "string",
                    "description": "Run ID (for resume/check/result/kill/steer). Resume accepts only a terminal run owned by the current parent session."
                },
                "thread_id": {
                    "type": "string",
                    "description": "Stable child-thread ID for send. Compatibility run_id is also accepted, but output always includes thread_id."
                },
                "mode": {
                    "type": "string",
                    "enum": ["auto", "steer_only", "resume_only"],
                    "description": "For send: auto chooses by current durable state; steer_only or resume_only fail rather than taking the other branch."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 1800,
                    "description": "Optional child run timeout in seconds. Omit by default to use the parent Agent's configured default (default 0/no timeout). 0 = no timeout. Set a positive value only when the user requested a deadline or this child task should be explicitly bounded; positive values are capped at 1800."
                },
                "wait": {
                    "type": "boolean",
                    "description": "For check: block until sub-agent completes (default false). Use as fallback if push notification was missed."
                },
                "wait_timeout": {
                    "type": "integer",
                    "description": "For check with wait=true: max seconds to wait (default 60, max 300)"
                },
                "partial": {
                    "type": "boolean",
                    "description": "For wait_all: whether a timeout may return completed child results as an accepted partial result. Defaults to false."
                },
                "result_mode": {
                    "type": "string",
                    "enum": ["status", "preview", "summary", "full"],
                    "description": "For wait_all: how much terminal child output to return. Defaults to preview for the generic subagent tool."
                },
                "model": {
                    "type": "string",
                    "description": "Model override: 'provider_id/model_id'"
                },
                "message": {
                    "type": "string",
                    "description": "For send/steer: follow-up message delivered to the same child conversation"
                },
                "label": {
                    "type": "string",
                    "description": "For spawn: display label for tracking this run (also usable in kill to target by label)"
                },
                "tasks": {
                    "type": "array",
                    "description": "For batch_spawn: array of task objects [{task, agent_id?, label?, timeout_secs?, model?, files?}]. Top-level files are shared by every task; task-level files are private to that child.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "task": { "type": "string" },
                            "agent_id": { "type": "string" },
                            "label": { "type": "string" },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 1800,
                                "description": "Optional timeout in seconds for this child task. Omit by default to use the parent Agent's configured default. 0 = no timeout. Use a positive value only for an explicitly bounded child task."
                            },
                            "model": { "type": "string" },
                            "files": {
                                "type": "array",
                                "description": "Attachments only for this child task; merged after top-level shared files.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "content": { "type": "string" },
                                        "mime_type": { "type": "string" },
                                        "encoding": { "type": "string", "enum": ["utf8", "base64"] }
                                    },
                                    "required": ["name", "content"]
                                }
                            }
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
                    "description": "For spawn/resume: attachments for the child turn. For batch_spawn: shared attachments passed to every child.",
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
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: false,
            default_deferred: true,
            config_hint: "Settings → Agents → ACP",
        },
        internal: false,
        concurrent_safe: false,
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
                    "description": "Optional ACP run timeout in seconds. Omit by default to use the ACP default (default 0/no timeout). 0 = no timeout. Use a positive value only when the user requested a deadline or this external run should be explicitly bounded; positive values are capped at 3600."
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
        tier: ToolTier::Core {
            subclass: CoreSubclass::Meta,
        },
        internal: true,
        concurrent_safe: false,
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

/// Returns the session-gated Workflow Mode orchestration and control tool definition.
///
/// This tool is not part of the static dispatch catalog: `AssistantAgent`
/// injects it only when the current session has Workflow Mode enabled, and
/// execution re-checks the persisted session mode.
pub fn get_workflow_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_WORKFLOW.into(),
        description: "Create, inspect, trace, and control observable durable workflow runs. Use this only when Workflow Mode is enabled. The assistant writes workflow scripts itself when orchestration helps; do not ask the user to provide a script or enter a coding-only mode first. Workflows are not coding-only: use them for substantial research, writing, data, connector, operations, knowledge, or coding tasks where durable, inspectable orchestration improves reliability. Call action=guide immediately before authoring a script to load the current V5 API, including stable Agent threads, resumeAgent, and failure resolution, without keeping a large guide in the system prompt. Use action=create to start, list/status/trace to inspect, control to pause/resume/cancel, and followup to repair or continue. The model must not approve user permissions; approval remains with the user.".into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::Meta,
        },
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["guide", "create", "list", "status", "trace", "control", "followup"],
                    "description": "Workflow control action. guide returns the current on-demand authoring contract; create starts a workflow from a script; list/status/trace inspect visible runs; control pauses/resumes/cancels; followup creates a repair or continuation workflow from an existing run."
                },
                "script": {
                    "type": "string",
                    "description": "For action=create/followup: complete JavaScript workflow script. For V5 define `export default async function main(workflow, args) { ... }`, use the workflow host APIs from action=guide, and finish via `workflow.finish(...)`."
                },
                "kind": {
                    "type": "string",
                    "description": "Optional run kind for display and filtering. Use a domain-neutral value such as `general.workflow`, `research.workflow`, `document.workflow`, or `coding.workflow`."
                },
                "executionMode": {
                    "type": "string",
                    "enum": ["guarded", "deep", "autonomous"],
                    "description": "Optional execution policy for the run. Omit to inherit the session execution mode, falling back to `guarded` when the session mode is off. `autonomous` requires an explicit budget with max runtime and max output tokens."
                },
                "budget": {
                    "type": "object",
                    "description": "Optional run budget, for example `{ \"maxScriptSecs\": 900, \"maxOps\": 64, \"maxOutputTokens\": 20000 }`. Required for `executionMode: \"autonomous\"`."
                },
                "apiVersion": {
                    "type": "integer",
                    "enum": [4, 5],
                    "description": "Workflow runtime API version. New scripts should use 5; version 4 remains available for explicit replay compatibility."
                },
                "meta": {
                    "type": "object",
                    "description": "Small immutable literal metadata such as name, description, tags, and authoring intent. Metadata is not executable and grants no permissions."
                },
                "args": {
                    "type": "object",
                    "description": "Immutable JSON inputs exposed as workflow.args and the second main(workflow, args) argument. Args are included in durable op identity."
                },
                "resumeFromRunId": {
                    "type": "string",
                    "description": "Optional terminal source run for edited-script resume. Only the longest matching shared_read_only Agent prefix may be reused; the first fingerprint difference closes reuse."
                },
                "sizeGuideline": {
                    "type": "string",
                    "enum": ["unrestricted", "small", "medium", "large"],
                    "description": "Advisory workflow scale for action=create/followup. This helps users and future model turns understand intent, but it does not bypass runtime caps, budgets, approval, or safety. Use small for a few bounded steps, medium for normal multi-step orchestration, large for broad fan-out/migrations/verification, and unrestricted only when the user explicitly wants exhaustive/Ultracode-style coverage and budgets still bound execution."
                },
                "runImmediately": {
                    "type": "boolean",
                    "description": "For action=create/followup: start the run immediately after creation. Defaults to true. When permission preview requires approval, the run will stop in the approval state for the user."
                },
                "parentRunId": {
                    "type": "string",
                    "description": "For action=create/followup: optional parent workflow run id when creating a repair or follow-up workflow."
                },
                "runId": {
                    "type": "string",
                    "description": "Workflow run id for action=status/trace/control/followup. Omit for status to inspect the most relevant active or recent run in the current session."
                },
                "scope": {
                    "type": "string",
                    "enum": ["active", "recent", "session", "goal"],
                    "description": "For action=list: which visible runs to list. Defaults to active."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Bounded result limit for action=list/status/trace."
                },
                "sinceSeq": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "For action=trace: only return workflow events with seq greater than this value."
                },
                "includePayload": {
                    "type": "boolean",
                    "description": "For action=trace: include bounded event payloads. Defaults to true; set false for summaries only."
                },
                "command": {
                    "type": "string",
                    "enum": ["pause", "resume", "cancel"],
                    "description": "For action=control: run-control command. There is intentionally no approval command; user permissions cannot be approved by the model."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional concise reason for a control or follow-up action."
                },
                "inheritGoal": {
                    "type": "boolean",
                    "description": "For action=followup: inherit the parent run's goal and criterion binding unless explicitly overridden. Defaults to true."
                },
                "origin": {
                    "type": "string",
                    "description": "Optional origin label for traceability, such as `agent:workflow_mode` or `repair:<run_id>`."
                },
                "goalId": {
                    "type": "string",
                    "description": "Optional goal id. Omit to let the runtime auto-bind the active goal for this session."
                },
                "goalCriterionId": {
                    "type": "string",
                    "description": "Optional active-goal completion criterion id, such as `criterion-1`, when the workflow is meant to advance a specific required/optional/follow-up criterion. It is validated against the bound goal revision."
                },
                "worktreeId": {
                    "type": "string",
                    "description": "Optional managed worktree id when the workflow is explicitly tied to an isolated worktree."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

/// Returns the `image_generate` tool definition with a dynamic description
/// built from the unified media-gen config (chain-aware candidates + data-
/// driven per-model capabilities).
pub fn get_image_generate_tool_dynamic(
    config: &crate::media_gen::MediaGenConfig,
) -> ToolDefinition {
    use crate::media_gen::{resolve_candidates, MediaFunction, MediaModality};

    // Chain-aware candidate list — exactly what auto mode would try.
    let candidates = resolve_candidates(config, MediaFunction::Image, None).unwrap_or_default();
    let models_desc = if candidates.is_empty() {
        "No models configured".to_string()
    } else {
        candidates
            .iter()
            .take(12)
            .map(|c| c.label())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let chain_desc = if config.chains.image.is_some() {
        "the configured default chain"
    } else {
        "provider priority order"
    };

    // Capability summaries from data-driven model caps (all usable
    // providers' image models, not just the default candidates).
    let mut edit_providers: Vec<String> = Vec::new();
    let mut multi_image_providers: Vec<String> = Vec::new();
    let mut ar_providers: Vec<String> = Vec::new();
    let mut res_providers: Vec<String> = Vec::new();
    let mut all_model_ids: Vec<String> = Vec::new();
    let mut max_n: u32 = 4;
    for provider in config.providers.iter().filter(|p| p.is_usable()) {
        for model in provider
            .models
            .iter()
            .filter(|m| m.modality == MediaModality::Image)
        {
            all_model_ids.push(model.id.clone());
            let Some(caps) = &model.image else { continue };
            if let Some(edit) = &caps.edit {
                let detail = if edit.max_input_images > 1 {
                    format!("{} (up to {})", provider.name, edit.max_input_images)
                } else {
                    provider.name.clone()
                };
                if !edit_providers.contains(&detail) {
                    edit_providers.push(detail);
                }
                if edit.max_input_images > 1 && !multi_image_providers.contains(&provider.name) {
                    multi_image_providers.push(provider.name.clone());
                }
                max_n = max_n.max(edit.max_n);
            }
            if caps.supports_aspect_ratio && !ar_providers.contains(&provider.name) {
                ar_providers.push(provider.name.clone());
            }
            if caps.supports_resolution && !res_providers.contains(&provider.name) {
                res_providers.push(provider.name.clone());
            }
            max_n = max_n.max(caps.max_n);
        }
    }
    all_model_ids.sort();
    all_model_ids.dedup();

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
         Default candidates ({chain_desc}, automatic failover): {models_desc}.{edit_desc} \
         Use action='list' to see all providers with detailed capabilities. \
         Images are saved to disk and returned for visual inspection."
    );

    let model_param_desc = if all_model_ids.is_empty() {
        "Specify a model. Default: auto.".to_string()
    } else {
        let model_list = all_model_ids
            .iter()
            .take(16)
            .map(|m| format!("'{m}'"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Specify a model. Available: {model_list}. When the same model id exists on \
             multiple providers, use the 'provider-id::model-id' form to disambiguate. \
             Default: auto (default chain / priority order with failover)."
        )
    };

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
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Model Providers → Generation Models",
        },
        internal: false,
        concurrent_safe: false,
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
                    "description": "Image dimensions (e.g. '1024x1024', '1024x1536', '1536x1024', '1024x1792', '1792x1024'). Default: the configured global default."
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

/// Returns the `audio_generate` tool definition with a dynamic description
/// listing per-kind candidates from the unified media-gen config.
pub fn get_audio_generate_tool_dynamic(
    config: &crate::media_gen::MediaGenConfig,
) -> ToolDefinition {
    use crate::media_gen::{resolve_candidates, AudioKind, MediaFunction};

    let kind_summary = |kind: AudioKind| -> String {
        let candidates =
            resolve_candidates(config, MediaFunction::Audio(kind), None).unwrap_or_default();
        if candidates.is_empty() {
            format!("{}: none", kind.as_str())
        } else {
            format!(
                "{}: {}",
                kind.as_str(),
                candidates
                    .iter()
                    .take(6)
                    .map(|c| c.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };
    let models_desc = [
        kind_summary(AudioKind::Speech),
        kind_summary(AudioKind::Music),
        kind_summary(AudioKind::Sfx),
    ]
    .join("; ");

    let description = format!(
        "Generate audio from text: speech narration (TTS), music, or sound effects. \
         Default candidates per kind (automatic failover): {models_desc}. \
         Use action='list' to see providers, models, and voices. \
         Generated audio is saved to disk and returned as a playable attachment."
    );

    ToolDefinition {
        name: TOOL_AUDIO_GENERATE.into(),
        description,
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Model Providers → Generation Models",
        },
        internal: false,
        concurrent_safe: false,
        // Billed side effect: must stay OUT of `async_jobs::retry::is_retry_eligible`.
        async_capable: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["generate", "list"],
                    "description": "Action: 'generate' (default) creates audio, 'list' shows available providers and capabilities."
                },
                "prompt": {
                    "type": "string",
                    "description": "For speech: the exact text to narrate. For music/sfx: a description of the desired sound."
                },
                "kind": {
                    "type": "string",
                    "enum": ["speech", "music", "sfx"],
                    "description": "Audio kind: 'speech' (TTS narration, default), 'music', or 'sfx' (short sound effect)."
                },
                "voice": {
                    "type": "string",
                    "description": "Voice id for speech (provider-specific, e.g. 'alloy' for OpenAI or an ElevenLabs voice id). Default: the configured default voice cascade."
                },
                "durationSeconds": {
                    "type": "number",
                    "description": "Target duration in seconds for music/sfx (provider clamps to its legal range). Ignored for speech."
                },
                "model": {
                    "type": "string",
                    "description": "Specify a model id, or 'provider-id::model-id' to disambiguate. Default: auto (default chain / priority order with failover)."
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
        tier: ToolTier::Standard {
            default_for_main: true,
            default_for_others: true,
            default_deferred: true,
        },
        internal: true,
        concurrent_safe: false,
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

#[cfg(test)]
mod tests {
    #[test]
    fn subagent_schema_advertises_terminal_resume() {
        let def = super::get_subagent_tool();
        let actions = def.parameters["properties"]["action"]["enum"]
            .as_array()
            .expect("subagent action enum");
        assert!(actions.iter().any(|action| action == "resume"));
        assert!(def.parameters["properties"].get("task").is_some());
        assert!(def.parameters["properties"].get("run_id").is_some());
    }

    #[test]
    fn workflow_schema_requires_action_and_supports_control() {
        let def = super::get_workflow_tool();
        assert_eq!(def.name, crate::tools::TOOL_WORKFLOW);
        assert!(def
            .description
            .contains("The assistant writes workflow scripts itself"));
        assert!(def.description.contains("research, writing, data"));
        assert!(def
            .description
            .contains("must not approve user permissions"));
        let properties = def
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("workflow properties");
        assert!(properties.contains_key("action"));
        assert!(properties.contains_key("script"));
        assert!(properties.contains_key("sizeGuideline"));
        assert!(properties.contains_key("runId"));
        assert!(properties.contains_key("command"));
        assert!(
            !properties.contains_key("scriptSource"),
            "scriptSource remains an execution-layer compatibility alias, but the model schema should not advertise it while `script` is required"
        );
        let required = def
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .expect("workflow required fields");
        assert_eq!(
            required
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>(),
            vec!["action"]
        );
    }
}
