pub mod agent;
pub mod awareness;
pub mod context;
pub mod goal;
pub mod loop_control;
pub mod memory;
pub mod model;
pub mod pet;
pub mod plan;
pub mod project;
pub mod recap;
pub mod review;
pub mod session;
pub mod team;
pub mod utility;
pub mod workflow;

use crate::channel::db::ChannelConversation;
use crate::get_memory_backend;
use crate::require_session_db;
use crate::slash_commands::types::CommandResult;

fn session_db() -> Result<&'static std::sync::Arc<crate::session::SessionDB>, String> {
    require_session_db().map_err(|e| e.to_string())
}

/// The model pinned on the calling session, if any. `/model` and `/status`
/// must reflect what this session actually runs with (#786) instead of the
/// global `active_model`, which only applies while no session pin exists.
pub(super) fn session_pinned_model(
    db: &crate::session::SessionDB,
    session_id: Option<&str>,
) -> Option<crate::provider::ActiveModel> {
    let meta = db.get_session(session_id?).ok().flatten()?;
    pinned_model_from_meta(&meta)
}

/// The session's own model pin read off a session row. Empty strings are
/// treated like absence so a half-written pin never masks the global model.
pub(super) fn pinned_model_from_meta(
    meta: &crate::session::SessionMeta,
) -> Option<crate::provider::ActiveModel> {
    let provider_id = meta.provider_id.as_deref().filter(|id| !id.is_empty())?;
    let model_id = meta.model_id.as_deref().filter(|id| !id.is_empty())?;
    Some(crate::provider::ActiveModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    })
}

/// The model this conversation's next turn would run with, for display
/// surfaces (`/model` ✓ and the `/status` Model line): session pin, then
/// the Agent's configured primary, then the global `active_model` — the
/// same configured-chain order `resolve_model_chain` applies before a turn
/// (provider-system.md §7.2). Unavailable references are skipped exactly
/// as at runtime; the chat-only catalog-recovery tier is not displayed.
pub(super) fn effective_session_model(
    agent_id: &str,
    session_pin: Option<crate::provider::ActiveModel>,
    store: &crate::config::AppConfig,
) -> Option<crate::provider::ActiveModel> {
    let agent_model = crate::agent_loader::load_agent(agent_id)
        .map(|definition| definition.config.model)
        .unwrap_or_default();
    let preferred = session_pin.map(|m| format!("{}::{}", m.provider_id, m.model_id));
    crate::provider::resolve_configured_model_chain_with_preferred(
        preferred.as_deref(),
        &agent_model,
        store,
    )
    .0
}

async fn effective_session_model_for_dispatch(
    session_id: Option<&str>,
    agent_id: &str,
    store: &crate::config::AppConfig,
) -> Result<Option<crate::provider::ActiveModel>, String> {
    let db = session_db()?.clone();
    let session_id = session_id.map(str::to_string);
    let agent_id = agent_id.to_string();
    let store = store.clone();

    Ok(crate::blocking::run_blocking(move || {
        let session_pin = session_pinned_model(&db, session_id.as_deref());
        effective_session_model(&agent_id, session_pin, &store)
    })
    .await)
}

/// Format the (sole, with 1:1 attach) IM-attach row as a markdown
/// bullet line. Used by `/status` and `/session` (info form) so both
/// surfaces stay consistent.
pub(super) fn format_attached_channel_line(
    a: &ChannelConversation,
    include_attached_at: bool,
) -> String {
    let label = a.sender_name.as_deref().unwrap_or(&a.chat_id);
    let attached = if include_attached_at {
        a.attached_at
            .as_deref()
            .map(|t| format!(" · attached `{}`", t))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "- **{}** · {} ({}){}",
        a.channel_id, label, a.chat_type, attached
    )
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
        "fork" => session::handle_fork(session_db()?, session_id, args).await,
        "side" => session::handle_side(session_db()?, session_id, args).await,
        "clear" => session::handle_clear(session_db()?, session_id),
        "stop" => Ok(session::handle_stop()),
        "rename" => session::handle_rename(session_db()?, session_id, args),
        "compact" => {
            // Return Compact action — frontend delegates to existing compact_context_now
            Ok(CommandResult {
                content: String::new(),
                action: Some(crate::slash_commands::types::CommandAction::Compact),
            })
        }

        // ── Model ──
        "model" => {
            let store = crate::config::cached_config();
            let effective =
                effective_session_model_for_dispatch(session_id, agent_id, &store).await?;
            model::handle_model(&store, args, effective.as_ref())
        }
        "models" => {
            let store = crate::config::cached_config();
            let effective =
                effective_session_model_for_dispatch(session_id, agent_id, &store).await?;
            model::handle_model(&store, "", effective.as_ref())
        }
        // `think` is a silent alias for `thinking` (only `thinking` is in the
        // registry / slash menu).
        "thinking" | "think" => model::handle_think(args),

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
        "plan" => plan::handle_plan(session_id, args).await,

        // ── Project ──
        "project" => project::handle_project(session_db()?, session_id, args),
        "projects" => project::handle_projects(),

        // ── Session picker / attach / handover ──
        "sessions" => session::handle_sessions(session_db()?, args),
        "session" => session::handle_session(session_db()?, session_id, args),
        "handover" => session::handle_handover(session_db()?, session_id, args),

        // ── Team ──
        "team" => team::handle_team(args),

        // ── Utility ──
        "permission" => utility::handle_permission(args),
        "help" => Ok(utility::handle_help(session_id)),
        "status" => {
            let store = crate::config::cached_config();
            let effective =
                effective_session_model_for_dispatch(session_id, agent_id, &store).await?;
            utility::handle_status(session_db()?, &store, session_id, agent_id, effective).await
        }
        "export" => utility::handle_export(session_db()?, session_id, args),
        "usage" => utility::handle_usage(session_db()?, session_id),
        "recap" => recap::handle_recap(args).await,
        "search" => utility::handle_search(args),
        "prompts" => Ok(utility::handle_prompts()),
        "context" => context::handle_context(session_id, agent_id, args).await,
        "pet" => pet::handle_pet(session_db()?.clone(), session_id, args).await,
        // `handle_workflow` / `handle_loop` / `handle_mode` are fully synchronous
        // (SessionDB / CronDB reads + writes under the global write lock). Route
        // them through the blocking pool so they never pin the async worker.
        "workflow" => {
            let db = session_db()?.clone();
            let session_id = session_id.map(str::to_string);
            let args = args.to_string();
            crate::blocking::run_blocking(move || {
                workflow::handle_workflow(&db, session_id.as_deref(), &args)
            })
            .await
        }
        "review" => review::handle_review(session_db()?, session_id, args).await,
        "loop" => {
            let sid = session_id
                .ok_or_else(|| "No active session for /loop".to_string())?
                .to_string();
            let cron_db = crate::require_cron_db().map_err(|e| e.to_string())?.clone();
            let db = session_db()?.clone();
            let args = args.to_string();
            crate::blocking::run_blocking(move || {
                loop_control::handle_loop(&db, &cron_db, &sid, &args)
            })
            .await
        }
        "mode" => {
            let db = session_db()?.clone();
            let session_id = session_id.map(str::to_string);
            let args = args.to_string();
            crate::blocking::run_blocking(move || {
                workflow::handle_mode(&db, session_id.as_deref(), &args)
            })
            .await
        }
        "goal" => goal::handle_goal(session_db()?, session_id, args).await,
        "awareness" => awareness::handle_awareness(args),
        "imreply" => utility::handle_imreply(session_id, args).await,
        // `reasoning` is a silent alias for `reason` (only `reason` is in the
        // registry / slash menu).
        "reason" | "reasoning" => utility::handle_reason(session_id, args).await,
        "kb" => utility::handle_kb(session_id, args).await,

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
    let env_check =
        crate::skills::skill_env_check_enabled_for_agent(Some(agent_id), store.skill_env_check);
    let skill_env = store.skill_env.clone();
    let working_dir = crate::session::effective_session_working_dir(session_id);
    let skills = crate::skills_hooks::invocable_skills(
        &store.extra_skills_dirs,
        &store.disabled_skills,
        working_dir.as_deref().map(std::path::Path::new),
    );
    // Command ownership uses the session-scoped catalog also rendered by
    // `/help`; the per-Agent switch below still decides whether activation
    // returns a setup diagnostic after a command has been matched.
    let skills = crate::skills::filter_catalog_eligible_skills(
        skills,
        store.skill_env_check,
        &store.skill_env,
    );
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

    if env_check {
        let detail = crate::skills::check_requirements_detail(
            &matched.requires,
            skill_env.get(&matched.name),
        );
        if !detail.eligible {
            return Some(Ok(CommandResult {
                content: crate::skills::format_requirements_diagnostic(&matched, &detail),
                action: Some(CommandAction::DisplayOnly),
            }));
        }
    }

    let result = match crate::skills::resolve_skill_slash_dispatch(&matched, args) {
        // ── Fork mode: dispatch skill to sub-agent ──
        crate::skills::SkillSlashDispatch::Fork => {
            return Some(dispatch_skill_fork(&matched, args, session_id, agent_id).await);
        }

        // ── Path 1: Direct tool execution (zero LLM round-trip) ──
        crate::skills::SkillSlashDispatch::Tool => {
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

            // Selecting a skill is not approval for the tool it dispatches.
            // Keep the normal permission engine in the loop.
            let ctx = crate::tool_defs::ToolExecContext {
                session_id: session_id.map(String::from),
                agent_id: Some(agent_id.to_string()),
                home_dir: dirs::home_dir().map(|p| p.to_string_lossy().to_string()),
                session_working_dir: crate::session::effective_session_working_dir(session_id),
                skill_allowed_tools: matched.tool_ceiling().execution_filter(),
                auto_approve_tools: false,
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
        crate::skills::SkillSlashDispatch::ModelTemplate { message } => {
            if message.trim().is_empty() {
                return Some(Err(format!(
                    "Skill '{}' produced no model prompt; activation stopped before model dispatch",
                    matched.name
                )));
            }
            Ok(CommandResult {
                content: format!("Using skill **{}**...", matched.name),
                action: Some(CommandAction::PassThrough {
                    message,
                    skill_activation: Some(crate::slash_defs::types::SlashSkillActivation {
                        skill_name: matched.name.clone(),
                        command_name: command.to_string(),
                        skill_allowed_tools: matched.tool_ceiling().execution_filter(),
                    }),
                }),
            })
        }

        // ── Path 3: Default with no template — inline SKILL.md ──
        crate::skills::SkillSlashDispatch::ModelInline => {
            // Inline SKILL.md so the LLM skips the tool_search → read indirection
            // that the old "Read the skill file at <path>" prompt forced when
            // deferred tools were enabled.
            let rendered = crate::skills_hooks::render_skill_inline(&matched, args).await;
            let message = match require_materialized_skill_prompt(&matched.name, args, rendered) {
                Ok(message) => message,
                Err(error) => {
                    crate::app_warn!(
                        "slash_cmd",
                        "skill_inline",
                        "Explicit slash skill could not be materialized; dispatch stopped"
                    );
                    return Some(Err(error));
                }
            };
            Ok(CommandResult {
                content: format!("Invoking skill **{}**...", matched.name),
                action: Some(CommandAction::PassThrough {
                    message,
                    skill_activation: Some(crate::slash_defs::types::SlashSkillActivation {
                        skill_name: matched.name.clone(),
                        command_name: command.to_string(),
                        skill_allowed_tools: matched.tool_ceiling().execution_filter(),
                    }),
                }),
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
        "<explicit_skill_command name=\"{name}\">The user invoked this skill via a slash command{args_clause}. The full skill is already loaded below; follow it as user-level workflow guidance without reloading it. This command does not bypass tool permission or sandbox policy.</explicit_skill_command>\n\n{skill_content}"
    )
}

/// Pair the materialized Skill body with the slash activation prompt or fail
/// before any transport can turn a missing body into an ordinary Provider
/// request. This chokepoint is shared by Desktop/HTTP and IM dispatch.
fn require_materialized_skill_prompt(
    name: &str,
    args: &str,
    rendered: anyhow::Result<String>,
) -> Result<String, String> {
    rendered
        .map(|content| build_skill_activation_prompt(name, args, &content))
        .map_err(|_| {
            format!(
                "Skill '{name}' could not be materialized; activation stopped before model dispatch"
            )
        })
}

/// Dispatch a skill in fork mode: spawn a sub-agent to execute the skill.
/// The skill's SKILL.md content is carried in the child user task; it is not
/// promoted into the child's system prompt.
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
    let run_id =
        crate::skills_hooks::spawn_skill_fork(skill, args, parent_session_id, agent_id, false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slash_commands::types::CommandAction;

    #[tokio::test]
    async fn think_alias_dispatches_like_thinking() {
        let result = dispatch(None, crate::agent_loader::DEFAULT_AGENT_ID, "think", "high")
            .await
            .expect("/think should dispatch to /thinking");

        assert_eq!(result.content, "Thinking effort set to **high**");
        match result.action {
            Some(CommandAction::SetEffort { effort }) => assert_eq!(effort, "high"),
            other => panic!("expected SetEffort action, got {other:?}"),
        }
    }

    #[test]
    fn slash_skill_materialization_failure_cannot_fall_back_to_provider_prompt() {
        let error = require_materialized_skill_prompt(
            "restricted-review",
            "check this",
            Err(anyhow::anyhow!("SKILL.md disappeared")),
        )
        .expect_err("a missing Skill body must stop every transport before Provider dispatch");

        assert!(error.contains("stopped before model dispatch"));
        assert!(!error.contains("SKILL.md disappeared"));
    }

    #[test]
    fn session_pinned_model_reads_the_pin_and_treats_empty_strings_as_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
        let db = crate::session::SessionDB::open_ephemeral_for_test(&path).expect("open");
        // The session projection LEFT JOINs channel_conversations; create the
        // minimal channel fixture table so get_session can run.
        db.with_conn_for_test(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    thread_id TEXT,
                    session_id TEXT NOT NULL,
                    sender_id TEXT,
                    sender_name TEXT,
                    chat_type TEXT NOT NULL DEFAULT 'dm',
                    is_primary INTEGER NOT NULL DEFAULT 1,
                    source TEXT NOT NULL DEFAULT 'inbound',
                    attached_at TEXT,
                    created_at TEXT NOT NULL
                )",
            )?;
            Ok(())
        })
        .expect("channel table");
        let meta = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("create");
        let sid = meta.id.clone();

        // Fresh sessions snapshot the agent chain's model at creation, so the
        // hermetic assertion is about explicit writes: pin one model, read it
        // back, then clear it and read absence.
        db.update_session_model(&sid, Some("p1"), Some("Provider One"), Some("m2"))
            .expect("pin model");
        let pinned = session_pinned_model(&db, Some(&sid)).expect("pinned");
        assert_eq!(pinned.provider_id, "p1");
        assert_eq!(pinned.model_id, "m2");

        // An explicit un-pin (NULLs) must not yield a phantom model.
        db.update_session_model(&sid, None, None, None)
            .expect("unpin model");
        assert!(session_pinned_model(&db, Some(&sid)).is_none());

        // A half-written pin (empty strings) must not mask the global model.
        db.update_session_model(&sid, Some(""), Some(""), Some(""))
            .expect("clear pin");
        assert!(session_pinned_model(&db, Some(&sid)).is_none());

        assert!(session_pinned_model(&db, None).is_none());
    }

    fn display_store_with_models(active: Option<(&str, &str)>) -> crate::config::AppConfig {
        use crate::provider::{ApiType, ModelConfig, ProviderConfig};

        let mut provider = ProviderConfig::new(
            "Provider One".into(),
            ApiType::OpenaiChat,
            "https://example.test".into(),
            "test-key".into(),
        );
        provider.id = "p1".into();
        provider.enabled = true;
        provider.models = ["m1", "m2", "m3"]
            .iter()
            .map(|id| ModelConfig {
                id: (*id).into(),
                name: format!("Model {}", id.to_uppercase()),
                input_types: vec!["text".into()],
                context_window: 128_000,
                max_tokens: 8192,
                reasoning: false,
                thinking_style: None,
                cost_input: None,
                cost_output: None,
            })
            .collect();
        crate::config::AppConfig {
            providers: vec![provider],
            active_model: active.map(|(provider_id, model_id)| crate::provider::ActiveModel {
                provider_id: provider_id.into(),
                model_id: model_id.into(),
            }),
            ..Default::default()
        }
    }

    /// Materialize `agents/{default}/agent.json` under a temp `HA_DATA_DIR`
    /// so `load_agent` resolves the fixture instead of the host config.
    fn write_agent_fixture(root: &std::path::Path, primary: &str) {
        let agent_dir = root
            .join("agents")
            .join(crate::agent_loader::DEFAULT_AGENT_ID);
        std::fs::create_dir_all(&agent_dir).expect("agent dir");
        std::fs::write(
            agent_dir.join("agent.json"),
            serde_json::json!({ "model": { "primary": primary } }).to_string(),
        )
        .expect("agent.json");
    }

    #[test]
    fn effective_session_model_prefers_the_session_pin_over_agent_and_global() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_agent_fixture(temp.path(), "p1::m1");
        let store = display_store_with_models(Some(("p1", "m2")));
        let pin = crate::provider::ActiveModel {
            provider_id: "p1".into(),
            model_id: "m3".into(),
        };

        let effective = crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            super::effective_session_model(crate::agent_loader::DEFAULT_AGENT_ID, Some(pin), &store)
        })
        .expect("effective");
        assert_eq!(effective.provider_id, "p1");
        assert_eq!(effective.model_id, "m3");
    }

    #[test]
    fn effective_session_model_falls_back_to_the_agent_primary_without_a_pin() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_agent_fixture(temp.path(), "p1::m1");
        let store = display_store_with_models(Some(("p1", "m2")));

        let effective = crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            super::effective_session_model(crate::agent_loader::DEFAULT_AGENT_ID, None, &store)
        })
        .expect("effective");
        // The Agent primary is what the session would run with — not the
        // global active model.
        assert_eq!(effective.provider_id, "p1");
        assert_eq!(effective.model_id, "m1");
    }

    #[test]
    fn effective_session_model_falls_back_to_global_without_pin_or_agent_primary() {
        // No agents/ fixture at all: `load_agent` fails closed to the default
        // (no primary), so the global model is the remaining configured tier.
        let temp = tempfile::tempdir().expect("tempdir");
        let store = display_store_with_models(Some(("p1", "m2")));

        let effective = crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            super::effective_session_model(crate::agent_loader::DEFAULT_AGENT_ID, None, &store)
        })
        .expect("effective");
        assert_eq!(effective.provider_id, "p1");
        assert_eq!(effective.model_id, "m2");
    }

    #[test]
    fn effective_session_model_skips_an_unavailable_agent_primary() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_agent_fixture(temp.path(), "p1::gone");
        let store = display_store_with_models(Some(("p1", "m2")));

        let effective = crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            super::effective_session_model(crate::agent_loader::DEFAULT_AGENT_ID, None, &store)
        })
        .expect("effective");
        assert_eq!(effective.provider_id, "p1");
        assert_eq!(effective.model_id, "m2");
    }
}
