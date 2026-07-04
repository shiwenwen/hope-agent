use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;

use super::types::{
    AskUserQuestion, AskUserQuestionAnswer, AskUserQuestionGroup, AskUserTimedOutPayload,
    CreateOwnerAskUserQuestionInput,
};

// ── EventBus event names ─────────────────────────────────────────

/// Canonical event name for an interactive user-question request.
pub const EVENT_ASK_USER_REQUEST: &str = "ask_user_request";
/// Canonical event name for a live ask_user_question request timing out.
pub const EVENT_ASK_USER_TIMED_OUT: &str = "ask_user_timed_out";

pub fn emit_ask_user_timed_out(
    request_id: &str,
    session_id: &str,
    timeout_secs: u64,
    used_default_values: bool,
    question_preview: Option<String>,
) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let payload = AskUserTimedOutPayload {
        request_id: request_id.to_string(),
        session_id: session_id.to_string(),
        timeout_secs,
        used_default_values,
        question_preview,
    };
    match serde_json::to_value(payload) {
        Ok(value) => bus.emit(EVENT_ASK_USER_TIMED_OUT, value),
        Err(e) => app_warn!(
            "ask_user",
            "timeout",
            "Failed to serialize ask_user timeout event {}: {}",
            request_id,
            e
        ),
    }
}

// ── Pending Ask-User Questions Registry (oneshot pattern) ────────

static PENDING_ASK_USER_QUESTIONS: OnceLock<
    TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>>>,
> = OnceLock::new();

fn get_pending_questions(
) -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>>>
{
    PENDING_ASK_USER_QUESTIONS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Register a pending question and return the receiver.
pub async fn register_ask_user_question(
    request_id: String,
    sender: tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>,
) {
    let mut pending = get_pending_questions().lock().await;
    pending.insert(request_id, sender);
}

/// Submit answers from the frontend (called by Tauri command).
pub async fn submit_ask_user_question_response(
    request_id: &str,
    answers: Vec<AskUserQuestionAnswer>,
) -> Result<()> {
    let mut pending = get_pending_questions().lock().await;
    if let Some(sender) = pending.remove(request_id) {
        let _ = sender.send(answers);
        drop(pending);
        crate::tools::approval::emit_pending_interactions_changed(None);
        Ok(())
    } else {
        drop(pending);
        submit_owner_question_response(request_id, answers)
    }
}

/// Cancel a pending question (e.g., on timeout or user cancel).
pub async fn cancel_pending_ask_user_question(request_id: &str) {
    let mut pending = get_pending_questions().lock().await;
    pending.remove(request_id);
    drop(pending);
    crate::tools::approval::emit_pending_interactions_changed(None);
}

/// Check whether a request_id is currently awaited by a live tool call
/// (in-memory oneshot registered). Used to filter out zombie rows left over
/// from a previous process that can no longer receive answers.
pub async fn is_ask_user_question_live(request_id: &str) -> bool {
    get_pending_questions()
        .lock()
        .await
        .contains_key(request_id)
}

/// Return the most recent still-pending question group for the given session.
/// Tool-created groups must still have a live in-memory oneshot. Owner-created
/// groups carry a durable response handler and can be answered without one.
pub async fn find_live_pending_group_for_session(
    session_id: &str,
) -> anyhow::Result<Option<AskUserQuestionGroup>> {
    let Some(db) = crate::get_session_db() else {
        return Ok(None);
    };
    let groups = db.list_pending_ask_user_groups_for_session(session_id)?;
    for group in groups.into_iter().rev() {
        if group.owner_response.is_some() || is_ask_user_question_live(&group.request_id).await {
            return Ok(Some(group));
        }
    }
    Ok(None)
}

// ── SQLite Persistence ──────────────────────────────────────────

/// Persist a pending question group so a restart can resume it.
/// No-op when the session DB isn't initialised (e.g. during tests).
pub fn persist_pending_group(group: &AskUserQuestionGroup) -> Result<()> {
    let Some(db) = crate::get_session_db() else {
        return Ok(());
    };
    db.save_ask_user_group(group)
}

/// Persist and emit an owner-plane question. Unlike tool-created ask_user
/// requests, there is no live oneshot; submitting the answer records durable
/// evidence described by `group.owner_response`.
pub fn persist_owner_question(group: &AskUserQuestionGroup) -> Result<()> {
    if group.owner_response.is_none() {
        bail!("owner ask_user question requires owner_response");
    }
    persist_pending_group(group)?;
    if let Some(bus) = crate::globals::get_event_bus() {
        match serde_json::to_value(group) {
            Ok(event_data) => {
                bus.emit(EVENT_ASK_USER_REQUEST, event_data);
                crate::hooks::fire_elicitation(
                    &group.session_id,
                    &group.request_id,
                    group.questions.len(),
                );
            }
            Err(e) => bail!("failed to serialize owner ask_user question: {e}"),
        }
    }
    crate::tools::approval::emit_pending_interactions_changed(Some(&group.session_id));
    Ok(())
}

pub fn create_owner_ask_user_question(
    input: CreateOwnerAskUserQuestionInput,
) -> Result<AskUserQuestionGroup> {
    let mut owner_response = input.owner_response;
    let session_id = input.session_id.trim();
    if session_id.is_empty() {
        bail!("owner ask_user question requires session_id");
    }
    if input.questions.is_empty() {
        bail!("owner ask_user question requires at least one question");
    }
    if input.questions.len() > 4 {
        bail!("owner ask_user question supports at most 4 questions");
    }
    let Some(db) = crate::get_session_db() else {
        bail!("session DB is not initialized");
    };
    let session = db
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
    if session.incognito {
        bail!("owner ask_user question is disabled for incognito sessions");
    }
    validate_owner_response_session(&mut owner_response, session_id)?;
    let timeout_secs = input.timeout_secs.unwrap_or(0);
    let timeout_at = if timeout_secs > 0 {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(now_secs + timeout_secs)
    } else {
        None
    };
    let group = AskUserQuestionGroup {
        request_id: format!("auq_{}", uuid::Uuid::new_v4().simple()),
        session_id: session_id.to_string(),
        questions: input.questions,
        context: input.context,
        source: input
            .source
            .and_then(|value| non_empty(&value).map(str::to_string))
            .or_else(|| Some("owner".to_string())),
        timeout_at,
        owner_response: Some(owner_response),
    };
    persist_owner_question(&group)?;
    Ok(group)
}

/// Mark a persisted question group as answered so it won't be replayed on
/// next startup. No-op when the session DB isn't initialised.
pub fn mark_group_answered(request_id: &str) -> Result<()> {
    let Some(db) = crate::get_session_db() else {
        return Ok(());
    };
    db.mark_ask_user_answered(request_id)
}

fn submit_owner_question_response(
    request_id: &str,
    answers: Vec<AskUserQuestionAnswer>,
) -> Result<()> {
    let Some(db) = crate::get_session_db() else {
        bail!("No pending ask_user_question request: {request_id}");
    };
    let Some(group) = db.get_pending_ask_user_group_by_request_id(request_id)? else {
        bail!("No pending ask_user_question request: {request_id}");
    };
    let Some(owner) = group.owner_response.clone() else {
        bail!("No pending ask_user_question request: {request_id}");
    };
    match owner.action.as_str() {
        "record_domain_evidence" => {
            let Some(mut input) = owner.domain_evidence else {
                bail!("owner ask_user response missing domain evidence target");
            };
            ensure_domain_evidence_session(&mut input, &group.session_id)?;
            let formatted = format_answers_for_evidence(&group.questions, &answers);
            input.summary = Some(owner_answer_summary(input.summary.as_deref(), &formatted));
            input.source_metadata =
                merge_owner_answer_metadata(input.source_metadata, &group, &formatted);
            db.record_domain_evidence(input)?;
        }
        other => bail!("unsupported owner ask_user response action: {other}"),
    }
    db.mark_ask_user_answered(request_id)?;
    crate::hooks::fire_elicitation_result(&group.session_id, request_id, "answered");
    crate::tools::approval::emit_pending_interactions_changed(Some(&group.session_id));
    Ok(())
}

fn validate_owner_response_session(
    owner: &mut super::types::AskUserOwnerResponse,
    session_id: &str,
) -> Result<()> {
    match owner.action.as_str() {
        "record_domain_evidence" => {
            let Some(input) = owner.domain_evidence.as_mut() else {
                bail!("owner ask_user response missing domain evidence target");
            };
            ensure_domain_evidence_session(input, session_id)
        }
        other => bail!("unsupported owner ask_user response action: {other}"),
    }
}

fn ensure_domain_evidence_session(
    input: &mut crate::domain_workflow::RecordDomainEvidenceInput,
    session_id: &str,
) -> Result<()> {
    match input.session_id.as_deref().and_then(non_empty) {
        Some(existing) if existing != session_id => {
            bail!("owner ask_user evidence session mismatch: {existing} != {session_id}")
        }
        Some(_) => Ok(()),
        None => {
            input.session_id = Some(session_id.to_string());
            Ok(())
        }
    }
}

fn format_answers_for_evidence(
    questions: &[AskUserQuestion],
    answers: &[AskUserQuestionAnswer],
) -> Vec<Value> {
    questions
        .iter()
        .map(|question| {
            let answer = answers
                .iter()
                .find(|answer| answer.question_id == question.question_id);
            let selected_values = answer
                .map(|answer| answer.selected.clone())
                .unwrap_or_default();
            let selected_labels: Vec<String> = selected_values
                .iter()
                .map(|value| {
                    question
                        .options
                        .iter()
                        .find(|option| option.value == *value)
                        .map(|option| option.label.fallback_text().to_string())
                        .unwrap_or_else(|| value.clone())
                })
                .collect();
            json!({
                "questionId": question.question_id,
                "question": question.text.fallback_text(),
                "selected": selected_values,
                "selectedLabels": selected_labels,
                "customInput": answer.and_then(|answer| answer.custom_input.clone()),
            })
        })
        .collect()
}

fn owner_answer_summary(base: Option<&str>, formatted: &[Value]) -> String {
    let mut lines = Vec::new();
    if let Some(base) = base.and_then(non_empty) {
        lines.push(base.to_string());
    }
    for item in formatted {
        let question = item
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("Question");
        let selected = item
            .get("selectedLabels")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty());
        let custom = item
            .get("customInput")
            .and_then(Value::as_str)
            .and_then(non_empty);
        let answer = match (selected, custom) {
            (Some(selected), Some(custom)) => format!("{selected}; {custom}"),
            (Some(selected), None) => selected,
            (None, Some(custom)) => custom.to_string(),
            (None, None) => "未选择".to_string(),
        };
        lines.push(format!("{question}: {answer}"));
    }
    lines.join("\n")
}

fn merge_owner_answer_metadata(
    existing: Value,
    group: &AskUserQuestionGroup,
    formatted: &[Value],
) -> Value {
    let mut object = match existing {
        Value::Object(object) => object,
        other if other.is_null() => Map::new(),
        other => {
            let mut object = Map::new();
            object.insert("original".to_string(), other);
            object
        }
    };
    object.insert("ownerAction".to_string(), json!("ask_user"));
    object.insert("requestId".to_string(), json!(group.request_id));
    object.insert("source".to_string(), json!(group.source.as_deref()));
    object.insert("answers".to_string(), Value::Array(formatted.to_vec()));
    Value::Object(object)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
