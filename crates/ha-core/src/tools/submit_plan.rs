use crate::plan::{self, PlanModeState};
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

    // Transition to Review state
    if !plan::set_plan_state(&effective_sid, PlanModeState::Review).await {
        return "Error: invalid plan state transition to review".to_string();
    }

    // Set title on the meta entry
    {
        let store_ref = plan::store();
        let mut map = store_ref.write().await;
        if let Some(meta) = map.get_mut(&*effective_sid) {
            meta.title = Some(title.clone());
        }
    }

    // Persist to DB
    if let Some(session_db) = crate::get_session_db() {
        let _ =
            session_db.update_session_plan_mode(&effective_sid, crate::plan::PlanModeState::Review);
    }

    // Emit events to frontend. Include `content` in the payload so the frontend
    // doesn't need a follow-up `get_plan_content` RPC after the event fires.
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "plan_submitted",
            serde_json::json!({
                "sessionId": effective_sid,
                "title": title,
                "content": content,
            }),
        );
        bus.emit(
            "plan_mode_changed",
            serde_json::json!({
                "sessionId": effective_sid,
                "state": PlanModeState::Review.as_str(),
                "reason": "plan_submitted",
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
