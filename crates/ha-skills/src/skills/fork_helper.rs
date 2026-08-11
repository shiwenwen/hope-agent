//! Shared fork-dispatch helpers for skills.
//!
//! Two entry points activate a `context: fork` skill:
//!   1. The model calling the internal `skill` tool.
//!   2. The user typing `/skill-name` (slash command).
//!
//! Both must spawn the sub-agent with identical SpawnParams and extract the
//! terminal result the same way. This module owns that shared logic so the
//! two entry points don't drift.

use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::subagent::{self, SpawnParams, SubagentStatus};

use super::types::SkillEntry;

/// Upper bound on the text returned to the LLM as the skill tool_result.
/// 64 KB matches ~16K tokens — enough for a final summary, not enough to
/// reintroduce the "dump the subagent transcript into main context" problem.
pub const MAX_RESULT_CHARS: usize = 64_000;

/// Pair the exact SKILL.md bytes used by a fork with the catalog entry that
/// supplies its execution ceiling, agent, effort, and dispatch semantics.
/// Missing content or control-frontmatter drift must stop before a child run
/// is created; falling back to a description would execute a different task
/// under the selected Skill's authority envelope.
pub(crate) fn require_fork_skill_materialization(
    skill: &SkillEntry,
    args: &str,
    raw_content: Result<String>,
) -> Result<String> {
    let raw_content = raw_content?;
    crate::skills::validate_materialized_skill_snapshot(skill, &raw_content)?;
    let substituted = raw_content.replace("$ARGUMENTS", args);
    Ok(crate::skills::build_skill_context_payload(
        skill,
        &substituted,
    ))
}

/// Spawn a sub-agent to execute the given skill. Returns the `run_id`.
///
/// Caller is responsible for waiting on and extracting the result via
/// [`extract_fork_result`] (skill tool path), or registering the `run_id` as
/// a pending injection (slash-command path — legacy behavior).
pub async fn spawn_skill_fork(
    skill: &SkillEntry,
    args: &str,
    parent_session_id: &str,
    parent_agent_id: &str,
    skip_parent_injection: bool,
) -> Result<String> {
    let requested_task = if args.is_empty() {
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

    let skill_path = skill.file_path.clone();
    let skill_name = skill.name.clone();
    let raw_skill_content = tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&skill_path)
            .map_err(|error| anyhow!("Failed to read SKILL.md for '{skill_name}': {error}"))
    })
    .await
    .map_err(|error| anyhow!("SKILL.md read task failed for '{}': {error}", skill.name))?;
    let skill_content = require_fork_skill_materialization(skill, args, raw_skill_content)?;
    let task = format!(
        "{}\n\n<current_user_request>\n{}\n</current_user_request>",
        skill_content, requested_task
    );

    let session_db = ha_core::globals::get_session_db()
        .ok_or_else(|| anyhow!("Session DB not initialized"))?
        .clone();
    let cancel_registry = ha_core::globals::get_subagent_cancels()
        .ok_or_else(|| anyhow!("Sub-agent cancel registry not initialized"))?
        .clone();

    // `agent:` frontmatter — validate-and-fallback. If the declared agent id
    // can't be loaded we stick with the parent agent so the skill still runs
    // instead of erroring out.
    let resolved_agent = match skill.agent.as_deref() {
        Some(id) if !id.is_empty() => match ha_core::agent_loader::load_agent(id) {
            Ok(_) => id.to_string(),
            Err(e) => {
                app_warn!(
                    "skill",
                    "agent",
                    "Skill '{}' declares agent '{}' which is not loadable ({}); falling back to parent agent",
                    skill.name,
                    id,
                    e
                );
                parent_agent_id.to_string()
            }
        },
        _ => parent_agent_id.to_string(),
    };

    let params = SpawnParams {
        task,
        agent_id: resolved_agent,
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: parent_agent_id.to_string(),
        depth: 1,
        timeout_secs: Some(600),
        model_override: None,
        label: Some(format!("Skill: {}", skill.name)),
        isolate_worktree: false,
        attachments: Vec::new(),
        plan_agent_mode: None,
        plan_mode_allow_paths: Vec::new(),
        lock_plan_agent_mode: false,
        skip_parent_injection,
        run_instruction_context: None,
        run_data_context: None,
        skill_allowed_tools: skill.tool_ceiling().execution_filter(),
        reasoning_effort: skill.effort.clone(),
        skill_name: Some(skill.name.clone()),
        origin_source: None,
        origin_channel_kb_context: None,
        // `context: fork` skill subagent (skip_parent_injection) — never grouped (R5).
        group_id: None,
        owner_kind: ha_core::subagent::SubagentOwnerKind::Internal,
        owner_id: format!("skill:{}:{}", parent_session_id, skill.name),
        delivery_kind: ha_core::subagent::SubagentDeliveryKind::None,
    };

    subagent::spawn_subagent(params, session_db, cancel_registry)
        .await
        .map_err(|e| anyhow!("Failed to fork skill: {}", e))
}

/// Poll the session DB until the given run reaches a terminal state, then
/// format the result as a string suitable for a tool_result payload.
///
/// Timeouts/Errors/Killed all return a short marker rather than an `Err` so
/// the LLM sees a deterministic answer and can recover.
pub async fn extract_fork_result(run_id: &str, skill_name: &str) -> Result<String> {
    let session_db: Arc<ha_core::session::SessionDB> = ha_core::globals::get_session_db()
        .ok_or_else(|| anyhow!("Session DB not initialized"))?
        .clone();

    // Cap overall wait at 15 min; this helper explicitly gives skill forks a
    // 600s SpawnParams.timeout_secs. This is a ceiling for the polling loop in
    // case something gets wedged.
    let hard_deadline = Instant::now() + Duration::from_secs(900);

    loop {
        let run = {
            let db = session_db.clone();
            let run_id = run_id.to_string();
            db.run(move |db| db.get_subagent_run(&run_id)).await?
        }
        .ok_or_else(|| anyhow!("Sub-agent run '{}' not found", run_id))?;

        if run.status.is_terminal() {
            // Prevent EventBus injection delivering a duplicate result to the
            // parent conversation. The skill tool path already feeds this
            // string back as a tool_result — no other path should re-inject.
            {
                let db = session_db.clone();
                let run_id = run_id.to_string();
                db.run(move |db| {
                    db.suppress_subagent_result_delivery(&run_id, "explicitly_consumed")
                })
                .await?;
            }
            subagent::mark_run_fetched_in_memory(run_id);

            let body = match run.status {
                SubagentStatus::Completed => run
                    .result
                    .unwrap_or_else(|| "[Skill completed with empty output]".to_string()),
                SubagentStatus::Error => {
                    let reason = run.error.as_deref().unwrap_or("unknown error");
                    format!("[Skill failed: {}]", reason)
                }
                SubagentStatus::Timeout => "[Skill timed out]".to_string(),
                SubagentStatus::Killed => "[Skill cancelled]".to_string(),
                SubagentStatus::Interrupted => "[Skill interrupted by app restart]".to_string(),
                // is_terminal() guards against non-terminal variants
                _ => unreachable!(),
            };

            let truncated = ha_core::truncate_utf8(&body, MAX_RESULT_CHARS);
            return Ok(format!(
                "Skill '{}' completed.\n\nResult:\n{}",
                skill_name, truncated
            ));
        }

        if Instant::now() >= hard_deadline {
            return Ok(format!(
                "Skill '{}' did not complete within 15 minutes. Result will be injected when available.",
                skill_name
            ));
        }

        // Matches `tools::subagent::action_spawn_and_wait` cadence — fork
        // skills aren't time-sensitive enough to warrant more frequent DB hits.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
