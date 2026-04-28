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

    // Parse steps from markdown content
    let steps = plan::parse_plan_steps(&content);
    if steps.is_empty() {
        return "Error: plan content must contain at least one step in checklist format (- [ ] step)".to_string();
    }

    // Save plan file under the effective (parent) session
    match plan::save_plan_file(&effective_sid, &content) {
        Ok(file_path) => {
            app_info!(
                "plan",
                "submit_plan",
                "Plan saved: '{}' ({} steps) → {}",
                title,
                steps.len(),
                file_path
            );
        }
        Err(e) => {
            return format!("Error: failed to save plan file: {}", e);
        }
    }

    // Update plan meta: set title, steps, and transition to Review state
    {
        let store = plan::store().write().await;
        // We need to drop the store lock first, then use set_plan_state
        drop(store);
    }

    // First ensure meta exists, then update title and steps
    plan::set_plan_state(&effective_sid, PlanModeState::Review).await;
    plan::update_plan_steps(&effective_sid, steps.clone()).await;
    {
        let store_ref = plan::store();
        let mut map = store_ref.write().await;
        if let Some(meta) = map.get_mut(&*effective_sid) {
            meta.title = Some(title.clone());
            meta.steps = steps.clone();
        }
    }

    // Persist to DB
    if let Some(session_db) = crate::get_session_db() {
        let _ = session_db.update_session_plan_mode(&effective_sid, "review");
    }

    // Emit event to frontend
    if let Some(bus) = crate::globals::get_event_bus() {
        // Emit plan_submitted event with summary info
        let phase_count = {
            let mut phases = std::collections::HashSet::new();
            for step in &steps {
                if !step.phase.is_empty() {
                    phases.insert(step.phase.clone());
                }
            }
            phases.len()
        };

        bus.emit(
            "plan_submitted",
            serde_json::json!({
                "sessionId": effective_sid,
                "title": title,
                "stepCount": steps.len(),
                "phaseCount": phase_count,
                "steps": steps,
            }),
        );

        // Also emit plan_mode_changed
        bus.emit(
            "plan_mode_changed",
            serde_json::json!({
                "sessionId": effective_sid,
                "state": "review",
                "reason": "plan_submitted",
            }),
        );
    }

    let phase_count = {
        let mut phases = std::collections::HashSet::new();
        for step in &steps {
            if !step.phase.is_empty() {
                phases.insert(step.phase.clone());
            }
        }
        phases.len()
    };

    format!(
        "Plan '{}' submitted successfully ({} phases, {} steps). The plan is now in Review mode. \
         The user can see the plan card in the chat and the Plan panel on the right side. \
         They can approve and start execution when ready.",
        title,
        phase_count,
        steps.len()
    )
}
