pub mod agent;
pub mod memory;
pub mod model;
pub mod plan;
pub mod recap;
pub mod session;
pub mod utility;

use crate::get_memory_backend;
use crate::globals::AppState;
use crate::slash_commands::types::CommandResult;

/// Dispatch a parsed command to the appropriate handler.
pub async fn dispatch(
    state: &AppState,
    session_id: Option<&str>,
    agent_id: &str,
    command: &str,
    args: &str,
) -> Result<CommandResult, String> {
    match command {
        // ── Session ──
        "new" => session::handle_new(&state.session_db, agent_id),
        "clear" => session::handle_clear(&state.session_db, session_id),
        "stop" => Ok(session::handle_stop()),
        "rename" => session::handle_rename(&state.session_db, session_id, args),
        "compact" => {
            // Return Compact action — frontend delegates to existing compact_context_now
            Ok(CommandResult {
                content: "Compacting context...".into(),
                action: Some(crate::slash_commands::types::CommandAction::Compact),
            })
        }

        // ── Model ──
        "model" => {
            let store = state.config.lock().await;
            model::handle_model(&store, args)
        }
        "models" => {
            let store = state.config.lock().await;
            model::handle_model(&store, "")
        }
        "think" => model::handle_think(args),

        // ── Memory ──
        "remember" => {
            let backend = get_memory_backend().ok_or("Memory backend not initialized")?;
            memory::handle_remember(backend, args, session_id)
        }
        "forget" => {
            let backend = get_memory_backend().ok_or("Memory backend not initialized")?;
            memory::handle_forget(backend, args)
        }
        "memories" => {
            let backend = get_memory_backend().ok_or("Memory backend not initialized")?;
            memory::handle_memories(backend)
        }

        // ── Agent ──
        "agent" => agent::handle_agent(&state.session_db, args),
        "agents" => agent::handle_agents(),

        // ── Plan ──
        "plan" => plan::handle_plan(&state.session_db, session_id, args).await,

        // ── Utility ──
        "permission" => utility::handle_permission(args),
        "help" => Ok(utility::handle_help()),
        "status" => {
            let store = state.config.lock().await;
            utility::handle_status(&state.session_db, &store, session_id, agent_id)
        }
        "export" => utility::handle_export(&state.session_db, session_id),
        "usage" => utility::handle_usage(&state.session_db, session_id),
        "recap" => {
            let state_arc = crate::globals::get_app_state()
                .ok_or_else(|| "AppState not initialized".to_string())?
                .clone();
            recap::handle_recap(&state_arc, session_id, args).await
        }
        "search" => utility::handle_search(args),
        "prompts" => Ok(utility::handle_prompts()),

        _ => {
            // Check if it matches a user-invocable skill command
            if let Some(result) =
                handle_skill_command(state, command, args, session_id, agent_id).await
            {
                result
            } else {
                Err(format!("Unknown command: /{}", command))
            }
        }
    }
}

/// Expand a prompt template, replacing `$ARGUMENTS` with the user's args.
/// If the template doesn't contain `$ARGUMENTS` and args are provided,
/// appends them as a "User input:" section.
fn expand_prompt_template(template: &str, args: &str) -> String {
    let normalized = args.trim();
    if template.contains("$ARGUMENTS") {
        template.replace("$ARGUMENTS", normalized)
    } else if !normalized.is_empty() {
        format!("{}\n\nUser input:\n{}", template.trim(), normalized)
    } else {
        template.trim().to_string()
    }
}

/// Try to handle a command as a skill slash command.
/// Returns None if no matching skill found.
///
/// Supports three dispatch modes:
/// - `"tool"`: Execute the tool directly in the backend (zero LLM round-trip).
/// - `"prompt"`: Expand a prompt template and pass through to LLM.
/// - Default: Pass skill context to LLM, or use prompt template if available.
async fn handle_skill_command(
    state: &AppState,
    command: &str,
    args: &str,
    session_id: Option<&str>,
    agent_id: &str,
) -> Option<Result<CommandResult, String>> {
    let store = state.config.lock().await;
    let skills =
        crate::skills::get_invocable_skills(&store.extra_skills_dirs, &store.disabled_skills);
    drop(store);

    // Find a skill whose normalized name matches the command
    let matched = skills
        .into_iter()
        .find(|s| crate::skills::normalize_skill_command_name(&s.name) == command)?;

    use crate::slash_commands::types::CommandAction;

    // ── Fork mode: dispatch skill to sub-agent ──
    if matched.context_mode.as_deref() == Some("fork") {
        return Some(dispatch_skill_fork(&matched, args, session_id, agent_id).await);
    }

    let result = match matched.command_dispatch.as_deref() {
        // ── Path 1: Direct tool execution (zero LLM round-trip) ──
        Some("tool") => {
            let tool_name = match &matched.command_tool {
                Some(t) => t.clone(),
                None => {
                    return Some(Err(format!(
                        "❌ Skill '{}': command-dispatch is 'tool' but command-tool is not set",
                        matched.name
                    )));
                }
            };

            // Build tool arguments as JSON
            let tool_args = if matched.command_arg_mode.as_deref() == Some("raw") {
                serde_json::json!({ "command": args.trim() })
            } else {
                // Try to parse as JSON; fall back to wrapping in {"query": ...}
                serde_json::from_str(args.trim())
                    .unwrap_or_else(|_| serde_json::json!({ "query": args.trim() }))
            };

            // Build execution context
            let ctx = crate::tools::ToolExecContext {
                session_id: session_id.map(String::from),
                agent_id: Some(agent_id.to_string()),
                home_dir: dirs::home_dir().map(|p| p.to_string_lossy().to_string()),
                require_approval: vec![], // Skill-triggered tools auto-approve
                ..Default::default()
            };

            match crate::tools::execute_tool_with_context(&tool_name, &tool_args, &ctx).await {
                Ok(output) => {
                    let display = crate::truncate_utf8(&output, 4096);
                    Ok(CommandResult {
                        content: format!("**{}** → `{}`\n\n{}", matched.name, tool_name, display),
                        action: Some(CommandAction::DisplayOnly),
                    })
                }
                Err(e) => Ok(CommandResult {
                    content: format!("❌ Tool `{}` failed: {}", tool_name, e),
                    action: Some(CommandAction::DisplayOnly),
                }),
            }
        }

        // ── Path 2: Prompt template expansion ──
        Some("prompt") => {
            let template = matched.command_prompt_template.as_deref().unwrap_or("");
            let message = expand_prompt_template(template, args);
            Ok(CommandResult {
                content: format!("Using skill **{}**...", matched.name),
                action: Some(CommandAction::PassThrough { message }),
            })
        }

        // ── Path 3: Default — template if available, otherwise skill context ──
        _ => {
            if let Some(template) = &matched.command_prompt_template {
                let message = expand_prompt_template(template, args);
                Ok(CommandResult {
                    content: format!("Using skill **{}**...", matched.name),
                    action: Some(CommandAction::PassThrough { message }),
                })
            } else {
                let message = if args.is_empty() {
                    format!(
                        "Use the skill '{}'. Read the skill file at {} for instructions.",
                        matched.name, matched.file_path
                    )
                } else {
                    format!(
                        "Use the skill '{}' to: {}. Read the skill file at {} for instructions.",
                        matched.name, args, matched.file_path
                    )
                };
                Ok(CommandResult {
                    content: format!("Invoking skill **{}**...", matched.name),
                    action: Some(CommandAction::PassThrough { message }),
                })
            }
        }
    };

    Some(result)
}

/// Dispatch a skill in fork mode: spawn a sub-agent to execute the skill.
/// The skill's SKILL.md content is injected as extra system context.
async fn dispatch_skill_fork(
    skill: &crate::skills::SkillEntry,
    args: &str,
    session_id: Option<&str>,
    agent_id: &str,
) -> Result<CommandResult, String> {
    use crate::slash_commands::types::CommandAction;

    let parent_session_id =
        session_id.ok_or_else(|| "Cannot fork skill: no session context".to_string())?;

    // Build task from user args or a default instruction
    let task = if args.is_empty() {
        format!(
            "Execute the skill '{}'. Follow the instructions in the skill context.",
            skill.name
        )
    } else {
        format!(
            "Execute the skill '{}' to: {}. Follow the instructions in the skill context.",
            skill.name, args
        )
    };

    // Read SKILL.md content for extra system context
    let skill_content = std::fs::read_to_string(&skill.file_path)
        .unwrap_or_else(|_| format!("Skill: {}\n{}", skill.name, skill.description));

    let session_db = crate::globals::get_session_db()
        .ok_or_else(|| "Session DB not initialized".to_string())?
        .clone();
    let cancel_registry = crate::globals::get_subagent_cancels()
        .ok_or_else(|| "Cancel registry not initialized".to_string())?
        .clone();

    let params = crate::subagent::SpawnParams {
        task,
        agent_id: agent_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: agent_id.to_string(),
        depth: 1,
        timeout_secs: Some(600), // 10 minutes for skill execution
        model_override: None,
        label: Some(format!("Skill: {}", skill.name)),
        attachments: Vec::new(),
        plan_agent_mode: None,
        plan_mode_allow_paths: Vec::new(),
        skip_parent_injection: false,
        extra_system_context: Some(skill_content),
        skill_allowed_tools: skill.allowed_tools.clone(),
    };

    let run_id = crate::subagent::spawn_subagent(params, session_db, cancel_registry)
        .await
        .map_err(|e| format!("Failed to fork skill: {}", e))?;

    Ok(CommandResult {
        content: format!(
            "Skill **{}** forked to sub-agent (run: {}). Result will be injected when complete.",
            skill.name,
            &run_id[..8]
        ),
        action: Some(CommandAction::SkillFork {
            run_id,
            skill_name: skill.name.clone(),
        }),
    })
}
