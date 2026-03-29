use serde_json::Value;
use crate::plan::{self, PlanQuestion, PlanQuestionOption, PlanQuestionGroup, PlanQuestionAnswer};
use crate::process_registry::create_session_id;

/// Execute the plan_question tool.
/// Sends structured questions to the user and blocks until they respond.
pub(crate) async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    // Parse questions array
    let questions_val = match args.get("questions").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return "Error: questions parameter is required (array)".to_string(),
    };

    let context = args.get("context").and_then(|v| v.as_str()).map(|s| s.to_string());

    let mut questions = Vec::new();
    for (i, q) in questions_val.iter().enumerate() {
        let text = match q.get("text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return format!("Error: questions[{}].text is required", i),
        };

        let options = q.get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|opt| {
                    let value = opt.get("value").and_then(|v| v.as_str())?.to_string();
                    let label = opt.get("label").and_then(|v| v.as_str())?.to_string();
                    let description = opt.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let recommended = opt.get("recommended").and_then(|v| v.as_bool()).unwrap_or(false);
                    Some(PlanQuestionOption { value, label, description, recommended })
                }).collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let allow_custom = q.get("allow_custom").and_then(|v| v.as_bool()).unwrap_or(true);
        let multi_select = q.get("multi_select").and_then(|v| v.as_bool()).unwrap_or(false);

        let question_id = q.get("question_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("q_{}", i));

        let template = q.get("template").and_then(|v| v.as_str()).map(|s| s.to_string());

        questions.push(PlanQuestion {
            question_id,
            text,
            options,
            allow_custom,
            multi_select,
            template,
        });
    }

    if questions.is_empty() {
        return "Error: at least one question is required".to_string();
    }

    let request_id = create_session_id();

    // Route to parent session if this is a plan sub-agent
    let effective_sid = plan::get_plan_owner_session_id(sid).await
        .unwrap_or_else(|| sid.to_string());

    let group = PlanQuestionGroup {
        request_id: request_id.clone(),
        session_id: effective_sid,
        questions: questions.clone(),
        context: context.clone(),
    };

    // Create oneshot channel
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Register pending question
    plan::register_plan_question(request_id.clone(), tx).await;

    // Emit event to frontend
    if let Some(app_handle) = crate::get_app_handle() {
        use tauri::Emitter;
        match serde_json::to_string(&group) {
            Ok(event_data) => {
                let _ = app_handle.emit("plan_question_request", event_data);
                app_info!("plan", "plan_question",
                    "Plan question sent to frontend (id: {}, {} questions)",
                    request_id, questions.len()
                );
            }
            Err(e) => {
                plan::cancel_pending_plan_question(&request_id).await;
                return format!("Error: failed to serialize question: {}", e);
            }
        }
    } else {
        plan::cancel_pending_plan_question(&request_id).await;
        return "Error: AppHandle not available for plan question events".to_string();
    }

    // Wait for response with timeout (10 minutes — user may need time to think)
    match tokio::time::timeout(std::time::Duration::from_secs(600), rx).await {
        Ok(Ok(answers)) => {
            // Format answers as readable text for the LLM
            format_answers_for_llm(&questions, &answers, context.as_deref())
        }
        Ok(Err(_)) => {
            app_warn!("plan", "plan_question", "Plan question cancelled (id: {})", request_id);
            "The user cancelled the questions without answering.".to_string()
        }
        Err(_) => {
            plan::cancel_pending_plan_question(&request_id).await;
            app_warn!("plan", "plan_question", "Plan question timed out (id: {})", request_id);
            "The questions timed out after 10 minutes without a response.".to_string()
        }
    }
}

/// Format user answers as JSON for both LLM consumption and frontend rendering.
fn format_answers_for_llm(
    questions: &[PlanQuestion],
    answers: &[PlanQuestionAnswer],
    _context: Option<&str>,
) -> String {
    let mut items = Vec::new();
    for question in questions {
        let mut selected_labels = Vec::new();
        let mut custom_input: Option<String> = None;

        if let Some(answer) = answers.iter().find(|a| a.question_id == question.question_id) {
            for sel in &answer.selected {
                let label = question.options.iter()
                    .find(|o| o.value == *sel)
                    .map(|o| o.label.clone())
                    .unwrap_or_else(|| sel.clone());
                selected_labels.push(label);
            }
            if let Some(c) = &answer.custom_input {
                if !c.is_empty() {
                    custom_input = Some(c.clone());
                }
            }
        }

        items.push(serde_json::json!({
            "question": question.text,
            "selected": selected_labels,
            "customInput": custom_input,
        }));
    }

    serde_json::json!({ "answers": items }).to_string()
}
