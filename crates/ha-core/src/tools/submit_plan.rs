use crate::plan::{self, PlanModeState, TransitionOpts, TransitionOutcome};
use serde_json::Value;

/// Execute the submit_plan tool.
/// LLM calls this to submit the final plan after interactive Q&A.
pub(crate) async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    // Route to parent session if this is a plan sub-agent
    let effective_sid = plan::get_plan_owner_session_id(sid)
        .await
        .unwrap_or_else(|| sid.to_string());

    let title = match args.get("title").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return "Error: title parameter is required".to_string(),
    };

    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return "Error: content parameter is required (markdown plan)".to_string(),
    };

    // Save plan file under the effective (parent) session
    match plan::save_plan_file(&effective_sid, &content) {
        Ok(file_path) => {
            app_info!(
                "plan",
                "submit_plan",
                "Plan saved: '{}' → {}",
                title,
                file_path
            );
        }
        Err(e) => {
            return format!("Error: failed to save plan file: {}", e);
        }
    }

    // Transition to Review state (centralized side effects: DB persist +
    // plan_mode_changed emit). submit_plan additionally emits `plan_submitted`
    // below with the content payload so the frontend can skip a follow-up RPC.
    match plan::transition_state(
        &effective_sid,
        PlanModeState::Review,
        TransitionOpts::new("plan_submitted"),
    )
    .await
    {
        Ok(TransitionOutcome::Applied) => {}
        Ok(TransitionOutcome::Rejected) => {
            return "Error: invalid plan state transition to review".to_string();
        }
        Err(e) => {
            return format!("Error: failed to persist plan state: {}", e);
        }
    }

    // Set title on the meta entry (post-transition so PlanMeta exists).
    {
        let store_ref = plan::store();
        let mut map = store_ref.write().await;
        if let Some(meta) = map.get_mut(&*effective_sid) {
            meta.title = Some(title.clone());
        }
    }

    // submit_plan-specific event (carries plan title + content so the frontend
    // doesn't need a follow-up `get_plan_content` RPC).
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "plan_submitted",
            serde_json::json!({
                "sessionId": effective_sid,
                "title": title,
                "content": content,
            }),
        );
    }

    format!(
        "Plan '{}' submitted successfully. The plan is now in Review mode. \
         The user can see the plan in the chat and the Plan panel on the right side. \
         They can approve and start execution when ready.",
        title
    )
}
