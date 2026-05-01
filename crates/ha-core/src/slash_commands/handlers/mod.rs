pub mod agent;
pub mod awareness;
pub mod context;
pub mod memory;
pub mod model;
pub mod plan;
pub mod project;
pub mod recap;
pub mod session;
pub mod team;
pub mod utility;

use crate::get_memory_backend;
use crate::require_session_db;
use crate::slash_commands::types::CommandResult;

fn session_db() -> Result<&'static std::sync::Arc<crate::session::SessionDB>, String> {
    require_session_db().map_err(|e| e.to_string())
}

/// Dispatch a parsed command to the appropriate handler.
pub async fn dispatch(
    session_id: Option<&str>,
    agent_id: &str,
    command: &str,
    args: &str,
) -> Result<CommandResult, String> {
    match command {
        // ── Session ──
        "new" => session::handle_new(session_db()?, agent_id),
        "clear" => session::handle_clear(session_db()?, session_id),
        "stop" => Ok(session::handle_stop()),
        "rename" => session::handle_rename(session_db()?, session_id, args),
        "compact" => {
            // Return Compact action — frontend delegates to existing compact_context_now
            Ok(CommandResult {
                content: "Compacting context...".into(),
                action: Some(crate::slash_commands::types::CommandAction::Compact),
            })
        }

        // ── Model ──
        "model" => {
            let store = crate::config::cached_config();
            model::handle_model(&store, args)
        }
        "models" => {
            let store = crate::config::cached_config();
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
        "agent" => agent::handle_agent(session_db()?, session_id, args),
        "agents" => agent::handle_agents(),

        // ── Plan ──
        "plan" => plan::handle_plan(session_db()?, session_id, args).await,

        // ── Project ──
        "project" => project::handle_project(session_db()?, session_id, args),

        // ── Team ──
        "team" => team::handle_team(args),

        // ── Utility ──
        "permission" => utility::handle_permission(args),
        "help" => Ok(utility::handle_help(session_id).await),
        "status" => {
            let store = crate::config::cached_config();
            utility::handle_status(session_db()?, &store, session_id, agent_id)
        }
        "export" => utility::handle_export(session_db()?, session_id),
        "usage" => utility::handle_usage(session_db()?, session_id),
        "recap" => recap::handle_recap(args).await,
        "search" => utility::handle_search(args),
        "prompts" => Ok(utility::handle_prompts()),
        "context" => context::handle_context(session_id, agent_id, args).await,
        "awareness" => awareness::handle_awareness(args),

        _ => {
            // Check if it matches a user-invocable skill command
            if let Some(result) = handle_skill_command(command, args, session_id, agent_id).await {
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
    command: &str,
    args: &str,
    session_id: Option<&str>,
    agent_id: &str,
) -> Option<Result<CommandResult, String>> {
    let store = crate::config::cached_config();
    let skills =
        crate::skills::get_invocable_skills(&store.extra_skills_dirs, &store.disabled_skills);
    drop(store);

    // Resolve via the shared collision-aware table so `/new_skill` (a skill named
    // `new` shadowed by built-in `/new`) dispatches to what the UI menu rendered.
    let reserved = crate::slash_commands::builtin_command_names();
    let resolved = crate::slash_commands::resolve_skill_command_names(&skills, reserved);
    let matched: crate::skills::SkillEntry = resolved
        .into_iter()
        .find(|r| r.typed_name == command)
        .map(|r| r.skill.clone())?;

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

            // Build execution context. Skill-triggered tools auto-approve via
            // `auto_approve_tools` rather than the (now-removed) `require_approval`
            // tool list — the legacy field was unread after the permission v2
            // refactor.
            let ctx = crate::tools::ToolExecContext {
                session_id: session_id.map(String::from),
                agent_id: Some(agent_id.to_string()),
                home_dir: dirs::home_dir().map(|p| p.to_string_lossy().to_string()),
                session_working_dir: crate::session::effective_session_working_dir(session_id),
                auto_approve_tools: true,
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

        // ── Path 3: Default — template if available, otherwise inline SKILL.md ──
        _ => {
            let message = if let Some(template) = &matched.command_prompt_template {
                expand_prompt_template(template, args)
            } else {
                // Inline SKILL.md so the LLM skips the tool_search → read indirection
                // that the old "Read the skill file at <path>" prompt forced when
                // deferred tools were enabled.
                match crate::tools::skill::render_inline(&matched, args).await {
                    Ok(skill_content) => {
                        build_skill_activation_prompt(&matched.name, args, &skill_content)
                    }
                    Err(e) => {
                        crate::app_warn!(
                            "slash_cmd",
                            "skill_inline",
                            "Failed to inline SKILL.md for '{}': {}; falling back to path reference",
                            matched.name,
                            e
                        );
                        build_skill_path_pointer_prompt(&matched.name, &matched.file_path, args)
                    }
                }
            };
            Ok(CommandResult {
                content: format!("Invoking skill **{}**...", matched.name),
                action: Some(CommandAction::PassThrough { message }),
            })
        }
    };

    Some(result)
}

/// Wrap a SKILL.md body in the activation preamble the LLM reads as "skill
/// already loaded, don't go looking for it".
fn build_skill_activation_prompt(name: &str, args: &str, skill_content: &str) -> String {
    let args_clause = if args.is_empty() {
        String::new()
    } else {
        format!(" with arguments: \"{}\"", args)
    };
    format!(
        "[SYSTEM: The user has invoked the '{name}' skill via slash command{args_clause}. \
         Follow the instructions in the skill content below without calling `read` or \
         `tool_search` — the full skill is already loaded.]\n\n---\n\n{skill_content}"
    )
}

/// Fallback prompt used when SKILL.md can't be read inline. Degrades the
/// activation rather than failing the command outright.
fn build_skill_path_pointer_prompt(name: &str, file_path: &str, args: &str) -> String {
    if args.is_empty() {
        format!("Use the skill '{name}'. Read the skill file at {file_path} for instructions.")
    } else {
        format!("Use the skill '{name}' to: {args}. Read the skill file at {file_path} for instructions.")
    }
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

    // Slash command path keeps skip_parent_injection=false so the existing
    // injection UX (result posted back as a user message) is preserved.
    // The `skill` tool path sets skip_parent_injection=true and synthesizes
    // its own tool_result.
    let run_id = crate::skills::spawn_skill_fork(skill, args, parent_session_id, agent_id, false)
        .await
        .map_err(|e| e.to_string())?;

    Ok(CommandResult {
        content: format!(
            "Skill **{}** forked to sub-agent (run: {}). Result will be injected when complete.",
            skill.name,
            crate::truncate_utf8(&run_id, 8)
        ),
        action: Some(CommandAction::SkillFork {
            run_id,
            skill_name: skill.name.clone(),
        }),
    })
}
