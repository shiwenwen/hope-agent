//! Execution backend for the `ask_user_question` tool.
//!
//! Types and the pending-question registry live in [`crate::ask_user`]; this
//! module only handles tool-call execution: parsing args, persisting the
//! group, awaiting the answer (with timeout / default fallback), and
//! formatting the result for the LLM.

use crate::ask_user::{
    self, AskUserDirectionCard, AskUserFileConstraints, AskUserI18nText, AskUserQuestion,
    AskUserQuestionAnswer, AskUserQuestionGroup, AskUserQuestionOption, AskUserText,
};
use crate::process_registry::create_session_id;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Execute the ask_user_question tool.
/// Sends structured questions to the user and blocks until they respond or time out.
///
/// pub：结构化问答唯一入口（AGENTS.md 红线）——特征 crate 的工具 adapter
/// （ha-updater `app_update` install/rollback 确认）从 crate 外复用，不 fork。
pub async fn execute(args: &Value, session_id: Option<&str>) -> String {
    let sid = match session_id {
        Some(s) => s,
        None => return "Error: no session context available".to_string(),
    };

    let questions_val = match args.get("questions").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return "Error: questions parameter is required (array)".to_string(),
    };

    let context = parse_optional_text_field(args, "context");

    let mut questions = Vec::new();
    for (i, q) in questions_val.iter().enumerate() {
        let text = match parse_optional_text_field(q, "text") {
            Some(t) => t,
            None => {
                return format!(
                    "Error: questions[{}].text is required (string or {{key, params, fallback}})",
                    i
                )
            }
        };

        let options = q
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|opt| {
                        let value = opt.get("value").and_then(|v| v.as_str())?.to_string();
                        let label = parse_optional_text_field(opt, "label")?;
                        let description = opt.get("description").and_then(parse_text_value);
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
                        let card = opt.get("card").and_then(parse_direction_card);
                        Some(AskUserQuestionOption {
                            value,
                            label,
                            description,
                            recommended,
                            preview,
                            preview_kind,
                            card,
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
        // Primary input shape (frontend rendering hint). Unknown / garbage
        // values fall back to None = legacy single/multi so a drifting model
        // can never produce an unrenderable question.
        let input_kind = q
            .get("input_kind")
            .or_else(|| q.get("inputKind"))
            .and_then(|v| v.as_str())
            .and_then(normalize_input_kind);

        let question_id = q
            .get("question_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("q_{}", i));

        let template = q
            .get("template")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let header = q.get("header").and_then(parse_text_value);
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

        let file_constraints = if input_kind.as_deref() == Some("file") {
            Some(parse_file_constraints(q))
        } else {
            None
        };

        questions.push(AskUserQuestion {
            question_id,
            text,
            options,
            input_kind,
            file_constraints,
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
    if questions
        .iter()
        .any(|q| q.input_kind.as_deref() == Some("file"))
        && (questions.len() != 1 || questions[0].input_kind.as_deref() != Some("file"))
    {
        return "Error: the file input PoC must be the only question in its request".to_string();
    }

    let request_id = create_session_id();

    // Route to the visible parent session when the question is raised inside
    // a sub-agent. Child sessions are intentionally hidden from the main chat
    // UI, so emitting the question against the child would leave the user with
    // no confirmation card to answer.
    let plan_owner = crate::plan::get_plan_owner_session_id(sid).await;
    let subagent_owner = if plan_owner.is_none() {
        crate::globals::get_session_db()
            .and_then(|db| db.get_session(sid).ok().flatten())
            .and_then(|meta| meta.parent_session_id)
    } else {
        None
    };
    let effective_sid = plan_owner
        .clone()
        .or_else(|| subagent_owner.clone())
        .unwrap_or_else(|| sid.to_string());
    if questions
        .iter()
        .any(|question| question.input_kind.as_deref() == Some("file"))
    {
        let attached_to_channel = crate::globals::get_channel_db()
            .and_then(|db| {
                db.get_conversation_by_session(&effective_sid)
                    .ok()
                    .flatten()
            })
            .is_some();
        if !attached_to_channel {
            return "Error: file input is currently available only for an attached IM conversation"
                .to_string();
        }
    }
    let source = Some(
        if plan_owner.is_some() {
            "plan"
        } else if subagent_owner.is_some() {
            "subagent"
        } else {
            "normal"
        }
        .to_string(),
    );

    // Resolve effective group timeout. The global switch defaults off; when
    // disabled, even model-provided per-question timeout hints are ignored.
    let cfg = crate::config::cached_config();
    let effective_timeout_secs = if cfg.ask_user_question_timeout_enabled {
        let per_q_max = questions
            .iter()
            .filter_map(|q| q.timeout_secs)
            .max()
            .unwrap_or(0);
        if per_q_max > 0 {
            per_q_max
        } else {
            cfg.ask_user_question_timeout_secs
        }
    } else {
        0
    };
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let timeout_at = if effective_timeout_secs > 0 {
        Some(now_secs.saturating_add(effective_timeout_secs))
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
        timeout_secs: (effective_timeout_secs > 0).then_some(effective_timeout_secs),
        server_now: Some(now_secs),
        owner_response: None,
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
    ask_user::register_ask_user_question(request_id.clone(), effective_sid.clone(), tx).await;

    // Emit event.
    if let Some(bus) = crate::globals::get_event_bus() {
        match serde_json::to_value(&group) {
            Ok(event_data) => {
                bus.emit(ask_user::EVENT_ASK_USER_REQUEST, event_data);
                // Elicitation hook (observation): a question prompt was raised.
                crate::hooks::fire_elicitation(&effective_sid, &request_id, questions.len());
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
                ask_user::cancel_pending_ask_user_question_with_source(&request_id, "error").await;
                let _ = ask_user::mark_group_answered(&request_id);
                return format!("Error: failed to serialize question: {}", e);
            }
        }
    } else {
        ask_user::cancel_pending_ask_user_question_with_source(&request_id, "error").await;
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
                ask_user::cancel_pending_ask_user_question_with_source(&request_id, "timeout")
                    .await;
                Outcome::TimedOut
            }
        }
    };

    // Final cleanup: mark persisted row answered and drop any IM-side pending
    // state so stale entries don't accumulate in the button/text maps.
    let _ = ask_user::mark_group_answered(&request_id);
    crate::channel_hooks::drop_ask_user_by_request_id(&request_id).await;

    // ElicitationResult hook (observation): the question group reached a
    // terminal state.
    let result_status = match &result {
        Outcome::Answered(_) => "answered",
        Outcome::Cancelled => "cancelled",
        Outcome::TimedOut => "timeout",
    };
    crate::hooks::fire_elicitation_result(&effective_sid, &request_id, result_status);

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
            ask_user::emit_ask_user_timed_out(
                &request_id,
                &effective_sid,
                effective_timeout_secs,
                !synth.is_empty(),
                first_question_preview(&questions),
            );
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

fn first_question_preview(questions: &[AskUserQuestion]) -> Option<String> {
    questions.first().and_then(|q| {
        let preview = crate::truncate_utf8(q.text.fallback_text(), 160)
            .trim()
            .to_string();
        (!preview.is_empty()).then_some(preview)
    })
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
            files: Vec::new(),
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
        let mut selected_values = Vec::new();
        let mut selected_labels = Vec::new();
        let mut custom_input: Option<String> = None;
        let mut files = Vec::new();

        if let Some(answer) = answers
            .iter()
            .find(|a| a.question_id == question.question_id)
        {
            for sel in &answer.selected {
                selected_values.push(sel.clone());
                let label = question
                    .options
                    .iter()
                    .find(|o| o.value == *sel)
                    .map(|o| o.label.fallback_text().to_string())
                    .unwrap_or_else(|| sel.clone());
                selected_labels.push(label);
            }
            if let Some(c) = &answer.custom_input {
                if !c.is_empty() {
                    custom_input = Some(c.clone());
                }
            }
            files = answer.files.clone();
        }

        items.push(serde_json::json!({
            "questionId": question.question_id,
            "question": question.text.fallback_text(),
            "selected": selected_labels,
            "selectedValues": selected_values,
            "customInput": custom_input,
            "files": files,
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

/// Whitelist the model-provided `input_kind`; anything outside the known set
/// (including empty/garbage) collapses to `None` = legacy single/multi so a
/// drifting model can never produce an unrenderable question.
fn normalize_input_kind(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    matches!(
        s.as_str(),
        "single" | "multi" | "text" | "textarea" | "direction-cards" | "file"
    )
    .then_some(s)
}

const ASK_USER_FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;

fn normalize_file_type(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pdf" | ".pdf" | "application/pdf" => Some("application/pdf"),
        "txt" | ".txt" | "text/plain" => Some("text/plain"),
        "md" | ".md" | "markdown" | "text/markdown" => Some("text/markdown"),
        _ => None,
    }
}

fn parse_file_constraints(question: &Value) -> AskUserFileConstraints {
    let raw = question
        .get("file_constraints")
        .or_else(|| question.get("fileConstraints"));
    let mut types = raw
        .and_then(|value| value.get("types"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(normalize_file_type)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    types.sort();
    types.dedup();
    if types.is_empty() {
        types = vec![
            "application/pdf".into(),
            "text/plain".into(),
            "text/markdown".into(),
        ];
    }
    let max_bytes = raw
        .and_then(|value| value.get("max_bytes").or_else(|| value.get("maxBytes")))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(ASK_USER_FILE_MAX_BYTES)
        .min(ASK_USER_FILE_MAX_BYTES);
    AskUserFileConstraints {
        types,
        max_bytes,
        // The first shared contract is single-file only. Ignore a model's
        // attempt to widen this value instead of creating provider-specific
        // multi-file semantics accidentally.
        count: 1,
    }
}

/// Parse a `direction-cards` option's `card` payload. Any malformed shape
/// yields `None` (the option then renders as a plain radio row) rather than
/// failing the whole question — presentation must never gate the answer.
fn parse_direction_card(value: &Value) -> Option<AskUserDirectionCard> {
    let obj = value.as_object()?;
    let str_vec = |key: &str, cap: usize| -> Vec<String> {
        obj.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .filter(|s| !s.trim().is_empty())
                    .take(cap)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let str_field = |key: &str| -> Option<String> {
        obj.get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    let card = AskUserDirectionCard {
        palette: str_vec("palette", 6),
        display_font: str_field("displayFont").or_else(|| str_field("display_font")),
        body_font: str_field("bodyFont").or_else(|| str_field("body_font")),
        mood: obj.get("mood").and_then(parse_text_value),
        references: str_vec("references", 4),
    };
    // Drop an entirely empty card so an option with `card: {}` stays a plain row.
    let empty = card.palette.is_empty()
        && card.display_font.is_none()
        && card.body_font.is_none()
        && card.mood.is_none()
        && card.references.is_empty();
    (!empty).then_some(card)
}

fn parse_optional_text_field(value: &Value, field: &str) -> Option<AskUserText> {
    value.get(field).and_then(parse_text_value)
}

fn parse_text_value(value: &Value) -> Option<AskUserText> {
    match value {
        Value::String(s) => Some(AskUserText::plain(s.clone())),
        Value::Object(obj) => {
            let key = obj.get("key")?.as_str()?.to_string();
            let fallback = obj
                .get("fallback")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let params = obj
                .get("params")
                .and_then(|v| v.as_object())
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            Some(AskUserText::I18n(AskUserI18nText {
                key,
                params,
                fallback,
            }))
        }
        _ => None,
    }
}

pub fn i18n_text(key: &str, params: Value, fallback: impl Into<String>) -> Value {
    let params = params.as_object().cloned().unwrap_or_default();
    json!({
        "key": key,
        "params": params,
        "fallback": fallback.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_kind_whitelist_filters_garbage() {
        for good in [
            "single",
            "multi",
            "text",
            "textarea",
            "direction-cards",
            "file",
        ] {
            assert_eq!(normalize_input_kind(good).as_deref(), Some(good));
        }
        // Case / whitespace tolerant.
        assert_eq!(
            normalize_input_kind("  Direction-Cards  ").as_deref(),
            Some("direction-cards")
        );
        // Unknown / empty collapse to None (legacy single/multi).
        for bad in ["", "color", "number", "range", "radio", "🙂"] {
            assert_eq!(normalize_input_kind(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn direction_card_parses_rich_payload() {
        let card = parse_direction_card(&json!({
            "palette": ["#111", "#fff", "", "#888", "#000", "#0af", "#extra7"],
            "displayFont": "Playfair Display, serif",
            "bodyFont": "Inter, sans-serif",
            "mood": "Editorial and confident.",
            "references": ["Monocle", "FT", "Kinfolk", "Cereal", "Extra5"]
        }))
        .expect("valid card");
        // Blanks dropped; palette capped at 6, references at 4.
        assert_eq!(
            card.palette,
            ["#111", "#fff", "#888", "#000", "#0af", "#extra7"]
        );
        assert_eq!(card.references.len(), 4);
        assert_eq!(
            card.display_font.as_deref(),
            Some("Playfair Display, serif")
        );
        assert_eq!(
            card.mood.as_ref().map(|m| m.fallback_text()),
            Some("Editorial and confident.")
        );
    }

    #[test]
    fn direction_card_snake_case_font_aliases() {
        let card = parse_direction_card(&json!({
            "display_font": "Georgia",
            "body_font": "Verdana"
        }))
        .expect("aliases accepted");
        assert_eq!(card.display_font.as_deref(), Some("Georgia"));
        assert_eq!(card.body_font.as_deref(), Some("Verdana"));
    }

    #[test]
    fn empty_card_is_none_so_option_stays_plain() {
        assert!(parse_direction_card(&json!({})).is_none());
        assert!(parse_direction_card(&json!({ "palette": [], "references": [] })).is_none());
        assert!(parse_direction_card(&json!("not an object")).is_none());
    }

    #[test]
    fn formatted_answers_preserve_question_and_option_identity() {
        let questions: Vec<AskUserQuestion> = serde_json::from_value(json!([{
            "questionId": "q-treatment",
            "text": "How should this be handled?",
            "options": [
                { "value": "steps", "label": "Same label" },
                { "value": "implement", "label": "Same label" }
            ]
        }]))
        .expect("valid questions");
        let answers = vec![AskUserQuestionAnswer {
            question_id: "q-treatment".into(),
            selected: vec!["implement".into()],
            custom_input: Some("Keep the explanation concise".into()),
            files: Vec::new(),
        }];

        let formatted = format_answers_for_llm(&questions, &answers, false);
        let value: Value = serde_json::from_str(&formatted).expect("valid result JSON");
        let answer = &value["answers"][0];

        assert_eq!(answer["questionId"], "q-treatment");
        assert_eq!(answer["selected"], json!(["Same label"]));
        assert_eq!(answer["selectedValues"], json!(["implement"]));
        assert_eq!(answer["customInput"], "Keep the explanation concise");
    }
}
