use crate::ask_user::{self, AskUserQuestion, AskUserQuestionGroup, AskUserQuestionOption};
use crate::plan::{self, PlanModeState, TransitionOutcome};
use crate::process_registry::create_session_id;
use serde_json::Value;

/// Execute the `enter_plan_mode` tool.
///
/// This is a **suggestion** path — the model proposes entering Plan Mode for a
/// non-trivial task, but the user has the final say. The tool surfaces a
/// Yes/No prompt via the standard `ask_user_question` infrastructure, and only
/// transitions the session to Planning if the user accepts. The user can
/// always enter Plan Mode directly without going through this tool (UI button
/// or `/plan enter`); this tool exists so the model can raise the suggestion
/// when it sees something that benefits from up-front planning.
pub(crate) async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    // If we're already in Plan Mode the suggestion is a no-op.
    let current = plan::get_plan_state(sid).await;
    if current != PlanModeState::Off {
        return format!(
            "Plan Mode is already active (state: {}). Continue with the in-mode workflow \
             (read / search / submit_plan) instead of calling enter_plan_mode again.",
            current.as_str()
        );
    }

    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let context_text = match &reason {
        Some(r) => format!(
            "The model suggests entering Plan Mode before doing this task. Reason: {}",
            r
        ),
        None => "The model suggests entering Plan Mode before doing this task.".to_string(),
    };

    let request_id = create_session_id();
    let group = AskUserQuestionGroup {
        request_id: request_id.clone(),
        session_id: sid.to_string(),
        questions: vec![AskUserQuestion {
            question_id: "enter_plan_mode".to_string(),
            text: "Enter Plan Mode? The model will explore, ask clarifying questions, and \
                   draft a written plan for your review before doing the work."
                .to_string(),
            options: vec![
                AskUserQuestionOption {
                    value: "yes".to_string(),
                    label: "Enter Plan Mode".to_string(),
                    description: Some(
                        "Switch to Plan Mode now; the model will start drafting the plan."
                            .to_string(),
                    ),
                    recommended: true,
                    preview: None,
                    preview_kind: None,
                },
                AskUserQuestionOption {
                    value: "no".to_string(),
                    label: "Skip planning".to_string(),
                    description: Some(
                        "Stay in normal mode; the model will continue the task directly."
                            .to_string(),
                    ),
                    recommended: false,
                    preview: None,
                    preview_kind: None,
                },
            ],
            allow_custom: false,
            multi_select: false,
            template: None,
            header: Some("Plan Mode".to_string()),
            timeout_secs: None,
            default_values: Vec::new(),
        }],
        context: Some(context_text),
        source: Some("plan".to_string()),
        timeout_at: None,
    };

    if let Err(e) = ask_user::persist_pending_group(&group) {
        app_warn!(
            "plan",
            "enter_plan_mode",
            "Failed to persist pending group {}: {}",
            request_id,
            e
        );
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    ask_user::register_ask_user_question(request_id.clone(), tx).await;

    if let Some(bus) = crate::globals::get_event_bus() {
        match serde_json::to_value(&group) {
            Ok(event_data) => {
                bus.emit(ask_user::EVENT_ASK_USER_REQUEST, event_data);
            }
            Err(e) => {
                ask_user::cancel_pending_ask_user_question(&request_id).await;
                let _ = ask_user::mark_group_answered(&request_id);
                return format!("Error: failed to serialize plan-mode prompt: {}", e);
            }
        }
    } else {
        ask_user::cancel_pending_ask_user_question(&request_id).await;
        let _ = ask_user::mark_group_answered(&request_id);
        return "Error: EventBus not available".to_string();
    }

    let answers = match rx.await {
        Ok(answers) => answers,
        Err(_) => {
            let _ = ask_user::mark_group_answered(&request_id);
            crate::channel::worker::ask_user::drop_pending_by_request_id(&request_id).await;
            return "Plan Mode prompt was cancelled. Continue the task directly.".to_string();
        }
    };
    let _ = ask_user::mark_group_answered(&request_id);
    crate::channel::worker::ask_user::drop_pending_by_request_id(&request_id).await;

    let accepted = answers
        .iter()
        .find(|a| a.question_id == "enter_plan_mode")
        .map(|a| a.selected.iter().any(|s| s == "yes"))
        .unwrap_or(false);

    if !accepted {
        return "User declined Plan Mode. Continue the task directly without drafting a plan."
            .to_string();
    }

    match plan::transition_state(sid, PlanModeState::Planning, "tool_enter_plan_mode").await {
        Ok(TransitionOutcome::Applied) => {}
        Ok(TransitionOutcome::Rejected) => {
            return format!(
                "Error: cannot transition from {} to Planning. Exit plan mode first if needed.",
                current.as_str()
            );
        }
        Err(e) => {
            return format!("Error: failed to persist plan state: {}", e);
        }
    }

    app_info!(
        "plan",
        "enter_plan_mode",
        "User accepted plan mode for session {}",
        sid
    );

    "Plan Mode entered (Planning). The user accepted your suggestion. You're now in a \
     read-only exploration and drafting phase: explore the codebase / sources, ask the user \
     for clarification via ask_user_question if needed, then call submit_plan with a \
     Context / Approach / Files / Reuse / Verification structure when the plan is ready. \
     The plan file is the only file you may edit during this phase. \
     IMPORTANT: your tool schema for the rest of THIS turn was built before Plan Mode \
     activated; write / edit / apply_patch / canvas calls will be denied at execution. \
     Stick to read-only tools (read / grep / glob / find / ls / web_search / web_fetch / \
     ask_user_question / submit_plan) until the next user message rebuilds the agent."
        .to_string()
}
