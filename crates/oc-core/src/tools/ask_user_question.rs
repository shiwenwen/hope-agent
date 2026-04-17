//! Execution backend for the `ask_user_question` tool.
//!
//! Types and the pending-question registry live in [`crate::ask_user`]; this
//! module only handles tool-call execution: parsing args, persisting the
//! group, awaiting the answer (with timeout / default fallback), and
//! formatting the result for the LLM.

use crate::ask_user::{
    self, AskUserQuestion, AskUserQuestionAnswer, AskUserQuestionGroup, AskUserQuestionOption,
};
use crate::process_registry::create_session_id;
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Execute the ask_user_question tool.
/// Sends structured questions to the user and blocks until they respond or time out.
pub(crate) async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    let questions_val = match args.get("questions").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return "Error: questions parameter is required (array)".to_string(),
    };

    let context = args
        .get("context")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut questions = Vec::new();
    for (i, q) in questions_val.iter().enumerate() {
        let text = match q.get("text").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return format!("Error: questions[{}].text is required", i),
        };

        let options = q
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|opt| {
                        let value = opt.get("value").and_then(|v| v.as_str())?.to_string();
                        let label = opt.get("label").and_then(|v| v.as_str())?.to_string();
                        let description = opt
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let recommended = opt
                            .get("recommended")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let preview = opt
                            .get("preview")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let preview_kind = opt
                            .get("previewKind")
                            .or_else(|| opt.get("preview_kind"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        Some(AskUserQuestionOption {
                            value,
                            label,
                            description,
                            recommended,
                            preview,
                            preview_kind,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // 模型可传 `allow_custom` 参数，但当前强制覆盖为 true：
        // 模型给的选项常常覆盖不到用户真实意图，强制留一个自由文本入口
        // 避免用户被迫二选一。字段和 schema 都保留着，等未来模型提问质量
        // 更稳定后可以摘掉这段覆盖恢复模型自主控制。
        let _model_allow_custom = q.get("allow_custom").and_then(|v| v.as_bool());
        let allow_custom = true;
        let multi_select = q
            .get("multi_select")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let question_id = q
            .get("question_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("q_{}", i));

        let template = q
            .get("template")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let header = q
            .get("header")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let timeout_secs = q
            .get("timeout_secs")
            .or_else(|| q.get("timeoutSecs"))
            .and_then(|v| v.as_u64())
            .filter(|n| *n > 0);
        let default_values = q
            .get("default_values")
            .or_else(|| q.get("defaultValues"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        questions.push(AskUserQuestion {
            question_id,
            text,
            options,
            allow_custom,
            multi_select,
            template,
            header,
            timeout_secs,
            default_values,
        });
    }

    if questions.is_empty() {
        return "Error: at least one question is required".to_string();
    }

    let request_id = create_session_id();

    // Route to parent session if this is a plan sub-agent. Cache the lookup
    // so the `source` tag can reuse it without a second DB round-trip.
    let plan_owner = crate::plan::get_plan_owner_session_id(sid).await;
    let effective_sid = plan_owner.clone().unwrap_or_else(|| sid.to_string());
    let source = Some(if plan_owner.is_some() { "plan" } else { "normal" }.to_string());

    // Resolve effective group timeout: max(per-question timeouts, global default).
    let global_default = crate::config::cached_config().ask_user_question_timeout_secs;
    let per_q_max = questions
        .iter()
        .filter_map(|q| q.timeout_secs)
        .max()
        .unwrap_or(0);
    let effective_timeout_secs = if per_q_max > 0 {
        per_q_max
    } else {
        global_default
    };
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let timeout_at = if effective_timeout_secs > 0 {
        Some(now_secs + effective_timeout_secs)
    } else {
        None
    };

    let group = AskUserQuestionGroup {
        request_id: request_id.clone(),
        session_id: effective_sid.clone(),
        questions: questions.clone(),
        context: context.clone(),
        source,
        timeout_at,
    };

    // Persist the pending group before emitting so restarts can resume it.
    if let Err(e) = ask_user::persist_pending_group(&group) {
        app_warn!(
            "ask_user",
            "persist",
            "Failed to persist pending ask_user group {}: {}",
            request_id,
            e
        );
    }

    // Create oneshot channel + register pending.
    let (tx, rx) = tokio::sync::oneshot::channel();
    ask_user::register_ask_user_question(request_id.clone(), tx).await;

    // Emit event.
    if let Some(bus) = crate::globals::get_event_bus() {
        match serde_json::to_value(&group) {
            Ok(event_data) => {
                bus.emit(ask_user::EVENT_ASK_USER_REQUEST, event_data);
                app_info!(
                    "ask_user",
                    "emit",
                    "ask_user question emitted (id: {}, {} questions, timeout: {}s)",
                    request_id,
                    questions.len(),
                    effective_timeout_secs
                );
            }
            Err(e) => {
                ask_user::cancel_pending_ask_user_question(&request_id).await;
                let _ = ask_user::mark_group_answered(&request_id);
                return format!("Error: failed to serialize question: {}", e);
            }
        }
    } else {
        ask_user::cancel_pending_ask_user_question(&request_id).await;
        let _ = ask_user::mark_group_answered(&request_id);
        return "Error: EventBus not available for ask_user events".to_string();
    }

    // Wait for response with optional timeout.
    let result = if effective_timeout_secs == 0 {
        match rx.await {
            Ok(answers) => Outcome::Answered(answers),
            Err(_) => Outcome::Cancelled,
        }
    } else {
        match tokio::time::timeout(Duration::from_secs(effective_timeout_secs), rx).await {
            Ok(Ok(answers)) => Outcome::Answered(answers),
            Ok(Err(_)) => Outcome::Cancelled,
            Err(_) => {
                ask_user::cancel_pending_ask_user_question(&request_id).await;
                Outcome::TimedOut
            }
        }
    };

    // Final cleanup: mark persisted row answered and drop any IM-side pending
    // state so stale entries don't accumulate in the button/text maps.
    let _ = ask_user::mark_group_answered(&request_id);
    crate::channel::worker::ask_user::drop_pending_by_request_id(&request_id).await;

    match result {
        Outcome::Answered(answers) => {
            format_answers_for_llm(&questions, &answers, /* timed_out */ false)
        }
        Outcome::Cancelled => {
            app_warn!(
                "ask_user",
                "cancel",
                "ask_user question cancelled (id: {})",
                request_id
            );
            "The user cancelled the questions without answering.".to_string()
        }
        Outcome::TimedOut => {
            app_warn!(
                "ask_user",
                "timeout",
                "ask_user question timed out after {}s (id: {})",
                effective_timeout_secs,
                request_id
            );
            let synth = synthesize_default_answers(&questions);
            if synth.is_empty() {
                format!(
                    "The questions timed out after {} seconds without a response and no default values were provided.",
                    effective_timeout_secs
                )
            } else {
                format_answers_for_llm(&questions, &synth, /* timed_out */ true)
            }
        }
    }
}

enum Outcome {
    Answered(Vec<AskUserQuestionAnswer>),
    Cancelled,
    TimedOut,
}

/// Construct synthetic answers from each question's `default_values` after a timeout.
fn synthesize_default_answers(questions: &[AskUserQuestion]) -> Vec<AskUserQuestionAnswer> {
    let mut out = Vec::new();
    for q in questions {
        if q.default_values.is_empty() {
            continue;
        }
        let mut selected = Vec::new();
        let mut custom: Option<String> = None;
        for v in &q.default_values {
            if q.options.iter().any(|o| &o.value == v) {
                selected.push(v.clone());
            } else {
                custom = Some(match custom {
                    Some(prev) => format!("{prev}, {v}"),
                    None => v.clone(),
                });
            }
        }
        out.push(AskUserQuestionAnswer {
            question_id: q.question_id.clone(),
            selected,
            custom_input: custom,
        });
    }
    out
}

/// Format user answers as JSON for both LLM consumption and frontend rendering.
fn format_answers_for_llm(
    questions: &[AskUserQuestion],
    answers: &[AskUserQuestionAnswer],
    timed_out: bool,
) -> String {
    let mut items = Vec::new();
    for question in questions {
        let mut selected_labels = Vec::new();
        let mut custom_input: Option<String> = None;

        if let Some(answer) = answers
            .iter()
            .find(|a| a.question_id == question.question_id)
        {
            for sel in &answer.selected {
                let label = question
                    .options
                    .iter()
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

    let mut root = serde_json::Map::new();
    root.insert("answers".into(), serde_json::Value::Array(items));
    if timed_out {
        root.insert("timedOut".into(), serde_json::Value::Bool(true));
        root.insert(
            "note".into(),
            serde_json::Value::String(
                "Some or all questions timed out; default values were automatically applied."
                    .into(),
            ),
        );
    }
    serde_json::Value::Object(root).to_string()
}
