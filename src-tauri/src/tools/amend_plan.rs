use crate::plan::{self, PlanModeState, PlanStep, PlanStepStatus};
use serde_json::Value;

/// Execute the amend_plan tool.
/// Allows modifying the plan during execution (insert/delete/update steps).
pub(crate) async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    // Verify we're in Executing or Paused state
    let state = plan::get_plan_state(sid).await;
    if state != PlanModeState::Executing && state != PlanModeState::Paused {
        return "Error: amend_plan can only be used during Executing or Paused state".to_string();
    }

    let action = match args.get("action").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => {
            return "Error: 'action' parameter is required (insert, delete, update)".to_string()
        }
    };

    match action {
        "insert" => action_insert(args, sid).await,
        "delete" => action_delete(args, sid).await,
        "update" => action_update(args, sid).await,
        _ => format!(
            "Error: unknown action '{}'. Use 'insert', 'delete', or 'update'.",
            action
        ),
    }
}

async fn action_insert(args: &Value, session_id: &str) -> String {
    let after_index = args
        .get("after_index")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let title = match args.get("title").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return "Error: 'title' is required for insert action".to_string(),
    };

    let phase = args
        .get("phase")
        .and_then(|v| v.as_str())
        .unwrap_or("Amended")
        .to_string();

    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let meta = match plan::get_plan_meta(session_id).await {
        Some(m) => m,
        None => return "Error: no plan found for this session".to_string(),
    };

    let mut steps = meta.steps;
    let insert_at = match after_index {
        Some(idx) => (idx + 1).min(steps.len()),
        None => steps.len(), // Append to end
    };

    let new_step = PlanStep {
        index: insert_at,
        phase,
        title: title.clone(),
        description,
        status: PlanStepStatus::Pending,
        duration_ms: None,
    };

    steps.insert(insert_at, new_step);

    // Re-index all steps
    for (i, step) in steps.iter_mut().enumerate() {
        step.index = i;
    }

    plan::update_plan_steps(session_id, steps.clone()).await;

    // Update plan file to reflect changes
    update_plan_file(session_id, &steps).await;

    // Emit event
    emit_plan_amended(session_id, &steps);

    format!(
        "Step '{}' inserted at index {}. Plan now has {} steps.",
        title,
        insert_at,
        steps.len()
    )
}

async fn action_delete(args: &Value, session_id: &str) -> String {
    let step_index = match args.get("step_index").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => return "Error: 'step_index' is required for delete action".to_string(),
    };

    let meta = match plan::get_plan_meta(session_id).await {
        Some(m) => m,
        None => return "Error: no plan found for this session".to_string(),
    };

    let mut steps = meta.steps;

    if step_index >= steps.len() {
        return format!(
            "Error: step_index {} is out of range (total: {})",
            step_index,
            steps.len()
        );
    }

    // Don't allow deleting completed steps
    if steps[step_index].status == PlanStepStatus::Completed {
        return "Error: cannot delete a completed step".to_string();
    }

    let removed_title = steps[step_index].title.clone();
    steps.remove(step_index);

    // Re-index
    for (i, step) in steps.iter_mut().enumerate() {
        step.index = i;
    }

    plan::update_plan_steps(session_id, steps.clone()).await;
    update_plan_file(session_id, &steps).await;
    emit_plan_amended(session_id, &steps);

    format!(
        "Step '{}' (index {}) deleted. Plan now has {} steps.",
        removed_title,
        step_index,
        steps.len()
    )
}

async fn action_update(args: &Value, session_id: &str) -> String {
    let step_index = match args.get("step_index").and_then(|v| v.as_u64()) {
        Some(idx) => idx as usize,
        None => return "Error: 'step_index' is required for update action".to_string(),
    };

    let meta = match plan::get_plan_meta(session_id).await {
        Some(m) => m,
        None => return "Error: no plan found for this session".to_string(),
    };

    let mut steps = meta.steps;

    if step_index >= steps.len() {
        return format!(
            "Error: step_index {} is out of range (total: {})",
            step_index,
            steps.len()
        );
    }

    // Only allow updating pending/in_progress steps
    if steps[step_index].status.is_terminal() {
        return "Error: cannot update a completed/skipped/failed step".to_string();
    }

    if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
        steps[step_index].title = title.to_string();
    }
    if let Some(description) = args.get("description").and_then(|v| v.as_str()) {
        steps[step_index].description = description.to_string();
    }
    if let Some(phase) = args.get("phase").and_then(|v| v.as_str()) {
        steps[step_index].phase = phase.to_string();
    }

    plan::update_plan_steps(session_id, steps.clone()).await;
    update_plan_file(session_id, &steps).await;
    emit_plan_amended(session_id, &steps);

    format!("Step {} updated: '{}'", step_index, steps[step_index].title)
}

/// Regenerate the plan markdown file from the current steps.
/// Preserves step descriptions as indented text below each checklist item.
async fn update_plan_file(session_id: &str, steps: &[PlanStep]) {
    // Build markdown from steps
    let mut md = String::new();
    let mut current_phase = String::new();

    for step in steps {
        if step.phase != current_phase {
            current_phase = step.phase.clone();
            md.push_str(&format!("\n### {}\n", current_phase));
        }
        let checkbox = if step.status == PlanStepStatus::Completed {
            "x"
        } else {
            " "
        };
        md.push_str(&format!("- [{}] {}\n", checkbox, step.title));
        if !step.description.is_empty() {
            // Indent description as continuation of the list item
            for line in step.description.lines() {
                md.push_str(&format!("  {}\n", line));
            }
        }
    }

    let _ = plan::save_plan_file(session_id, md.trim());
}

/// Emit a plan_amended event to update the frontend.
fn emit_plan_amended(session_id: &str, steps: &[PlanStep]) {
    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app_handle.emit(
            "plan_amended",
            serde_json::json!({
                "sessionId": session_id,
                "steps": steps,
                "stepCount": steps.len(),
            }),
        );
    }
}
