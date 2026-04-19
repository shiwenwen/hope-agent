use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use super::constants::{PLAN_MODE_SYSTEM_PROMPT, PLAN_SUBAGENT_CONTEXT_NOTICE};
use super::types::PlanAgentConfig;

// ── Plan Sub-Agent Session Registry ─────────────────────────────
// Maps child_session_id → parent info, so plan tools (ask_user_question, submit_plan)
// can route events to the parent session instead of the sub-agent session.

struct PlanSubagentInfo {
    parent_session_id: String,
    run_id: String,
}

static PLAN_SUBAGENT_SESSIONS: OnceLock<Arc<RwLock<HashMap<String, PlanSubagentInfo>>>> =
    OnceLock::new();

fn plan_subagent_store() -> &'static Arc<RwLock<HashMap<String, PlanSubagentInfo>>> {
    PLAN_SUBAGENT_SESSIONS.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

/// Register a plan sub-agent session mapping.
pub async fn register_plan_subagent(child_sid: &str, parent_sid: &str, run_id: &str) {
    let mut map = plan_subagent_store().write().await;
    map.insert(
        child_sid.to_string(),
        PlanSubagentInfo {
            parent_session_id: parent_sid.to_string(),
            run_id: run_id.to_string(),
        },
    );
    app_info!(
        "plan",
        "subagent",
        "Registered plan sub-agent: child={} parent={} run={}",
        child_sid,
        parent_sid,
        run_id
    );
}

/// Unregister a plan sub-agent session mapping.
#[allow(dead_code)]
pub async fn unregister_plan_subagent(child_sid: &str) {
    let mut map = plan_subagent_store().write().await;
    if map.remove(child_sid).is_some() {
        app_info!(
            "plan",
            "subagent",
            "Unregistered plan sub-agent: child={}",
            child_sid
        );
    }
}

/// Synchronous version for use in non-async contexts (e.g., spawn completion callback).
/// Spawns a blocking task to do the cleanup.
pub fn try_unregister_plan_subagent_sync(child_sid: &str) {
    let sid = child_sid.to_string();
    let store = plan_subagent_store().clone();
    tokio::spawn(async move {
        let mut map = store.write().await;
        if map.remove(&sid).is_some() {
            app_info!(
                "plan",
                "subagent",
                "Unregistered plan sub-agent (sync): child={}",
                sid
            );
        }
    });
}

/// If this session_id belongs to a plan sub-agent, return the parent session_id.
pub async fn get_plan_owner_session_id(session_id: &str) -> Option<String> {
    let map = plan_subagent_store().read().await;
    map.get(session_id)
        .map(|info| info.parent_session_id.clone())
}

/// Get the active plan sub-agent run_id for a parent session, if any.
pub async fn get_active_plan_run_id(parent_session_id: &str) -> Option<String> {
    let map = plan_subagent_store().read().await;
    map.values()
        .find(|info| info.parent_session_id == parent_session_id)
        .map(|info| info.run_id.clone())
}

/// Spawn a dedicated plan sub-agent for the Planning phase.
/// Returns the run_id. The sub-agent runs with PlanAgent mode and PLAN_MODE_SYSTEM_PROMPT.
pub async fn spawn_plan_subagent(
    parent_session_id: &str,
    parent_agent_id: &str,
    user_message: &str,
    recent_context_summary: &str,
    session_db: std::sync::Arc<crate::session::SessionDB>,
    cancel_registry: std::sync::Arc<crate::subagent::SubagentCancelRegistry>,
) -> Result<String> {
    let config = PlanAgentConfig::default_config();

    let task = if recent_context_summary.is_empty() {
        user_message.to_string()
    } else {
        format!(
            "## User Request\n{}\n\n## Conversation Context\n{}",
            user_message, recent_context_summary
        )
    };

    let params = crate::subagent::SpawnParams {
        task,
        agent_id: parent_agent_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: parent_agent_id.to_string(),
        depth: 1,
        timeout_secs: Some(3600), // 1 hour — ask_user_question can wait 10 min each
        model_override: None,
        label: Some("Plan Creation".to_string()),
        attachments: Vec::new(),
        plan_agent_mode: Some(crate::agent::PlanAgentMode::PlanAgent {
            allowed_tools: config.allowed_tools,
            ask_tools: config.ask_tools,
        }),
        plan_mode_allow_paths: config.plan_mode_allow_paths,
        skip_parent_injection: true,
        extra_system_context: Some(format!(
            "{}\n\n{}",
            PLAN_MODE_SYSTEM_PROMPT, PLAN_SUBAGENT_CONTEXT_NOTICE
        )),
        skill_allowed_tools: Vec::new(),
        reasoning_effort: None,
        skill_name: None,
    };

    let run_id =
        crate::subagent::spawn_subagent(params, session_db.clone(), cancel_registry).await?;

    // Get child_session_id from the run record
    if let Ok(Some(run)) = session_db.get_subagent_run(&run_id) {
        register_plan_subagent(&run.child_session_id, parent_session_id, &run_id).await;
    }

    app_info!(
        "plan",
        "subagent",
        "Spawned plan sub-agent: run_id={} parent_session={}",
        run_id,
        parent_session_id
    );

    // Emit status event to frontend
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "plan_subagent_status",
            serde_json::json!({
                "sessionId": parent_session_id,
                "status": "running",
                "runId": run_id,
            }),
        );
    }

    Ok(run_id)
}
