//! `agent` hook handler — spawns a sub-agent with the hook prompt as its task
//! (design §7.6).
//!
//! `async: true` → fire-and-forget: spawn and immediately return the run id.
//! Otherwise the handler waits for the run to reach a terminal state (bounded
//! by the deadline) and returns the sub-agent's final output as stdout. The
//! sub-agent never injects back into the parent conversation
//! (`skip_parent_injection`) — the hook owns its result.
//!
//! NOTE: this is the heaviest handler; prefer `async: true` off hot paths.

use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::super::config::AgentHookConfig;
use super::super::env::HookEnv;
use super::super::types::HookInput;
use super::{HookHandler, RawHookResult};
use crate::subagent::{spawn_subagent, SpawnParams, HOOK_SPAWN_LABEL};

/// Default `agent` hook timeout when waiting for a synchronous run (official 60s).
const DEFAULT_AGENT_TIMEOUT_SECS: u64 = 60;
/// Poll interval while waiting for a synchronous sub-agent run.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub struct AgentHandler {
    config: AgentHookConfig,
}

impl AgentHandler {
    pub fn new(config: AgentHookConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl HookHandler for AgentHandler {
    fn identity(&self) -> String {
        format!(
            "{}|{}",
            self.config.agent.as_deref().unwrap_or(""),
            self.config.prompt
        )
    }

    fn handler_type(&self) -> &'static str {
        "agent"
    }

    fn default_timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout.unwrap_or(DEFAULT_AGENT_TIMEOUT_SECS))
    }

    async fn run(&self, input: &HookInput, _env: &HookEnv, deadline: Instant) -> RawHookResult {
        let start = Instant::now();
        let Some(session_db) = crate::get_session_db() else {
            return RawHookResult::non_blocking_error("agent hook: session DB unavailable");
        };
        let Some(cancel_registry) = crate::get_subagent_cancels().cloned() else {
            return RawHookResult::non_blocking_error("agent hook: cancel registry unavailable");
        };

        let common = input.common();
        let parent_agent_id = common
            .agent_id
            .clone()
            .unwrap_or_else(|| crate::agent_loader::DEFAULT_AGENT_ID.to_string());
        let agent_id = self
            .config
            .agent
            .clone()
            .unwrap_or_else(|| parent_agent_id.clone());

        let params = SpawnParams {
            task: self.config.prompt.clone(),
            agent_id,
            parent_session_id: common.session_id.clone(),
            parent_agent_id,
            depth: 0,
            timeout_secs: self.config.timeout,
            model_override: None,
            // Tag the spawn so `spawn_subagent` knows to suppress
            // SubagentStart / SubagentStop hooks for this child — otherwise
            // a Subagent{Start,Stop} agent hook would respawn forever.
            label: Some(HOOK_SPAWN_LABEL.to_string()),
            isolate_worktree: false,
            attachments: Vec::new(),
            plan_agent_mode: None,
            plan_mode_allow_paths: Vec::new(),
            lock_plan_agent_mode: false,
            // The hook captures the result itself; don't echo into the parent.
            skip_parent_injection: true,
            extra_system_context: None,
            skill_allowed_tools: self.config.allowed_tools.clone(),
            reasoning_effort: None,
            skill_name: None,
            origin_source: None,
            origin_channel_kb_context: None,
            // Internal hook-spawned subagent (skip_parent_injection) — never grouped (R5).
            group_id: None,
            owner_kind: crate::subagent::SubagentOwnerKind::Internal,
            owner_id: format!("hook:{}", common.session_id),
            delivery_kind: crate::subagent::SubagentDeliveryKind::None,
        };

        let run_id = match spawn_subagent(params, session_db.clone(), cancel_registry.clone()).await
        {
            Ok(id) => id,
            Err(e) => {
                return RawHookResult::non_blocking_error(format!("agent hook spawn failed: {e}"))
            }
        };

        // Fire-and-forget: return the run id and let it run in the background.
        if self.config.async_run.unwrap_or(false) {
            return RawHookResult {
                exit_code: Some(0),
                stdout: format!("spawned sub-agent run {run_id}"),
                stderr: String::new(),
                duration: start.elapsed(),
                timed_out: false,
            };
        }

        // Synchronous: poll the run record until it reaches a terminal state or
        // the deadline elapses.
        loop {
            if Instant::now() >= deadline {
                // Deadline hit. Signal the subagent's cancel flag so it stops
                // calling the model / running tools — otherwise the run
                // continues in the background indefinitely (until its own
                // `timeout_secs`, typically much longer than the hook
                // deadline), silently burning tokens and concurrency slots
                // on every PreToolUse / PostToolUse fire. Adversarial-review
                // MEDIUM. `cancel(...)` returns false when the run already
                // terminated between the last poll and now — that's fine,
                // it's a no-op in that case.
                let cancelled = cancel_registry.cancel(&run_id);
                crate::app_warn!(
                    "hooks",
                    "agent",
                    "agent hook deadline exceeded; cancel signaled run_id={} cancelled={}",
                    run_id,
                    cancelled
                );
                return RawHookResult {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!(
                        "agent hook run {run_id} did not finish before deadline (cancel signaled)"
                    ),
                    duration: start.elapsed(),
                    timed_out: true,
                };
            }
            match session_db.get_subagent_run(&run_id) {
                Ok(Some(run)) if run.status.is_terminal() => {
                    let body = run.result.or(run.error).unwrap_or_default();
                    return RawHookResult {
                        exit_code: Some(0),
                        stdout: body,
                        stderr: String::new(),
                        duration: start.elapsed(),
                        timed_out: false,
                    };
                }
                Ok(_) => {}
                Err(e) => {
                    return RawHookResult::non_blocking_error(format!(
                        "agent hook: failed to read run {run_id}: {e}"
                    ))
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}
