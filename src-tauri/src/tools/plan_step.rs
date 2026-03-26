use serde_json::Value;
use crate::plan::{self, PlanStepStatus};

/// Execute the update_plan_step tool.
/// Parameters: step_index (number), status ("in_progress"|"completed"|"skipped"|"failed")
pub(crate) async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let step_index = match args.get("step_index").and_then(|v| v.as_u64()) {
        Some(i) => i as usize,
        None => return "Error: step_index parameter is required (number)".to_string(),
    };

    let status_str = match args.get("status").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return "Error: status parameter is required".to_string(),
    };

    let status = PlanStepStatus::from_str(status_str);
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    plan::update_step_status(sid, step_index, status, None).await;

    // Emit Tauri global event for frontend real-time update
    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app_handle.emit("plan_step_updated", serde_json::json!({
            "sessionId": sid,
            "stepIndex": step_index,
            "status": status_str,
        }));
    }

    // Check if all steps completed → auto-transition
    if let Some(meta) = plan::get_plan_meta(sid).await {
        if meta.all_terminal() && meta.state == plan::PlanModeState::Executing {
            plan::set_plan_state(sid, plan::PlanModeState::Off).await;
            // Update DB
            if let Some(session_db) = crate::get_session_db() {
                let _ = session_db.update_session_plan_mode(sid, "off");
            }
            if let Some(app_handle) = crate::get_app_handle() {
                use tauri::Emitter;
                let _ = app_handle.emit("plan_mode_changed", serde_json::json!({
                    "sessionId": sid,
                    "state": "off",
                    "reason": "all_steps_completed",
                }));
            }
            return format!("Step {} marked as {}. All plan steps completed! Plan execution finished.", step_index, status_str);
        }
    }

    format!("Step {} marked as {}.", step_index, status_str)
}
