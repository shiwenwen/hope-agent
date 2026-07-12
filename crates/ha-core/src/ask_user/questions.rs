use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
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
/// Canonical terminal lifecycle event shared by desktop, web, and IM surfaces.
pub const EVENT_ASK_USER_RESOLVED: &str = "ask_user:resolved";

pub fn emit_ask_user_resolved(request_id: &str, session_id: &str, status: &str, source: &str) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            EVENT_ASK_USER_RESOLVED,
            json!({
                "requestId": request_id,
                "sessionId": session_id,
                "status": status,
                "source": source,
            }),
        );
    }
}

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

struct PendingAskUserQuestion {
    sender: tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>,
    session_id: String,
}

static PENDING_ASK_USER_QUESTIONS: OnceLock<TokioMutex<HashMap<String, PendingAskUserQuestion>>> =
    OnceLock::new();

/// Durable owner-plane questions do not have a live oneshot receiver, so their
/// timeout tasks are tracked separately and can be re-armed after restart.
struct OwnerTimeoutTask {
    abort_handle: tokio::task::AbortHandle,
    session_id: String,
}

static OWNER_ASK_USER_TIMEOUT_TASKS: OnceLock<Mutex<HashMap<String, OwnerTimeoutTask>>> =
    OnceLock::new();
/// Serializes every owner response, including the default no-timeout case, and
/// shares the same gate with a timeout task when one exists.
static OWNER_ASK_USER_TERMINAL_GATES: OnceLock<Mutex<HashMap<String, Arc<TokioMutex<()>>>>> =
    OnceLock::new();

fn get_pending_questions() -> &'static TokioMutex<HashMap<String, PendingAskUserQuestion>> {
    PENDING_ASK_USER_QUESTIONS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Register a pending question and return the receiver.
pub async fn register_ask_user_question(
    request_id: String,
    session_id: String,
    sender: tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>,
) {
    let mut pending = get_pending_questions().lock().await;
    pending.insert(request_id, PendingAskUserQuestion { sender, session_id });
}

async fn mark_ask_user_rows_answered(request_ids: Vec<String>) {
    if request_ids.is_empty() {
        return;
    }
    let result = crate::blocking::run_blocking(move || -> Result<()> {
        let Some(db) = crate::get_session_db() else {
            return Ok(());
        };
        for request_id in request_ids {
            db.mark_ask_user_answered(&request_id)?;
        }
        Ok(())
    })
    .await;
    if let Err(e) = result {
        app_warn!(
            "ask_user",
            "terminal_cleanup",
            "Failed to persist ask_user terminal cleanup: {}",
            e
        );
    }
}

/// Submit answers from the frontend (called by Tauri command).
pub async fn submit_ask_user_question_response(
    request_id: &str,
    answers: Vec<AskUserQuestionAnswer>,
) -> Result<()> {
    let mut pending = get_pending_questions().lock().await;
    if let Some(entry) = pending.remove(request_id) {
        let session_id = entry.session_id;
        let _ = entry.sender.send(answers);
        drop(pending);
        mark_ask_user_rows_answered(vec![request_id.to_string()]).await;
        emit_ask_user_resolved(request_id, &session_id, "answered", "response");
        crate::tools::approval::emit_pending_interactions_changed(Some(&session_id));
        Ok(())
    } else {
        drop(pending);
        submit_owner_question_response(request_id, answers).await
    }
}

/// Cancel a pending question (e.g., on timeout or user cancel).
pub async fn cancel_pending_ask_user_question(request_id: &str) {
    cancel_pending_ask_user_question_with_source(request_id, "cancelled").await;
}

pub async fn cancel_pending_ask_user_question_with_source(request_id: &str, source: &str) {
    let mut pending = get_pending_questions().lock().await;
    let removed = pending.remove(request_id);
    drop(pending);
    if let Some(entry) = removed {
        let session_id = entry.session_id;
        drop(entry.sender);
        mark_ask_user_rows_answered(vec![request_id.to_string()]).await;
        let status = if source == "timeout" {
            "timed_out"
        } else {
            "cancelled"
        };
        emit_ask_user_resolved(request_id, &session_id, status, source);
        crate::tools::approval::emit_pending_interactions_changed(Some(&session_id));
    }
}

/// Cancel every live tool-created ask_user wait for one session. Owner-plane
/// questions are durable workflow state and are intentionally unaffected by a
/// foreground chat Stop.
pub async fn cancel_pending_ask_user_questions_for_session(
    session_id: &str,
    source: &str,
) -> usize {
    let drained: Vec<(String, PendingAskUserQuestion)> = {
        let mut pending = get_pending_questions().lock().await;
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, entry)| entry.session_id == session_id)
            .map(|(request_id, _)| request_id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|request_id| pending.remove(&request_id).map(|entry| (request_id, entry)))
            .collect()
    };
    let count = drained.len();
    let request_ids = drained
        .iter()
        .map(|(request_id, _)| request_id.clone())
        .collect();
    for (request_id, entry) in drained {
        drop(entry.sender);
        emit_ask_user_resolved(&request_id, &entry.session_id, "cancelled", source);
    }
    mark_ask_user_rows_answered(request_ids).await;
    if count > 0 {
        crate::tools::approval::emit_pending_interactions_changed(Some(session_id));
    }
    count
}

pub async fn cancel_all_pending_ask_user_questions(source: &str) -> usize {
    let drained: Vec<(String, PendingAskUserQuestion)> = {
        let mut pending = get_pending_questions().lock().await;
        pending.drain().collect()
    };
    let count = drained.len();
    let request_ids = drained
        .iter()
        .map(|(request_id, _)| request_id.clone())
        .collect();
    for (request_id, entry) in drained {
        drop(entry.sender);
        emit_ask_user_resolved(&request_id, &entry.session_id, "cancelled", source);
        crate::tools::approval::emit_pending_interactions_changed(Some(&entry.session_id));
    }
    mark_ask_user_rows_answered(request_ids).await;
    count
}

fn owner_timeout_tasks() -> &'static Mutex<HashMap<String, OwnerTimeoutTask>> {
    OWNER_ASK_USER_TIMEOUT_TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn owner_terminal_gates() -> &'static Mutex<HashMap<String, Arc<TokioMutex<()>>>> {
    OWNER_ASK_USER_TERMINAL_GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn owner_terminal_gate(request_id: &str) -> Arc<TokioMutex<()>> {
    let mut gates = owner_terminal_gates()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(
        gates
            .entry(request_id.to_string())
            .or_insert_with(|| Arc::new(TokioMutex::new(()))),
    )
}

fn forget_owner_terminal_gate(request_id: &str) {
    if let Ok(mut gates) = owner_terminal_gates().lock() {
        gates.remove(request_id);
    }
}

fn forget_owner_terminal_gate_if_unused(request_id: &str, gate: &Arc<TokioMutex<()>>) {
    let has_timeout_task = owner_timeout_tasks()
        .lock()
        .map(|tasks| tasks.contains_key(request_id))
        .unwrap_or(true);
    if has_timeout_task {
        return;
    }
    if let Ok(mut gates) = owner_terminal_gates().lock() {
        let is_same_gate = gates
            .get(request_id)
            .is_some_and(|registered| Arc::ptr_eq(registered, gate));
        if is_same_gate {
            gates.remove(request_id);
        }
    }
}

fn forget_owner_timeout_task(request_id: &str) {
    if let Ok(mut tasks) = owner_timeout_tasks().lock() {
        tasks.remove(request_id);
    }
}

fn cancel_owner_timeout_task(request_id: &str) {
    if let Ok(mut tasks) = owner_timeout_tasks().lock() {
        if let Some(task) = tasks.remove(request_id) {
            task.abort_handle.abort();
        }
    }
}

/// Abort durable owner timeout tasks when their session is deleted/purged.
/// The DB rows are removed by FK cascade, so no terminal event should fire.
pub fn cancel_owner_question_timeouts_for_session(session_id: &str) -> usize {
    let Ok(mut tasks) = owner_timeout_tasks().lock() else {
        return 0;
    };
    let request_ids: Vec<String> = tasks
        .iter()
        .filter(|(_, task)| task.session_id == session_id)
        .map(|(request_id, _)| request_id.clone())
        .collect();
    for request_id in &request_ids {
        if let Some(task) = tasks.remove(request_id) {
            task.abort_handle.abort();
        }
        forget_owner_terminal_gate(request_id);
    }
    request_ids.len()
}

/// Arm one durable owner-plane timeout. Duplicate calls are idempotent, which
/// makes this safe at both creation time and startup recovery.
pub fn schedule_owner_question_timeout(group: AskUserQuestionGroup) {
    let Some(timeout_at) = group.timeout_at.filter(|value| *value > 0) else {
        return;
    };
    if group.owner_response.is_none() {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        app_warn!(
            "ask_user",
            "owner_timeout",
            "Cannot arm owner ask_user timeout {} without a Tokio runtime",
            group.request_id
        );
        return;
    };

    let request_id = group.request_id.clone();
    let mut tasks = match owner_timeout_tasks().lock() {
        Ok(tasks) => tasks,
        Err(e) => {
            app_warn!(
                "ask_user",
                "owner_timeout",
                "Failed to lock owner ask_user timeout registry: {}",
                e
            );
            return;
        }
    };
    if tasks.contains_key(&request_id) {
        return;
    }

    let task_request_id = request_id.clone();
    let task_session_id = group.session_id.clone();
    let gate = owner_terminal_gate(&request_id);
    let task_gate = Arc::clone(&gate);
    let join = runtime.spawn(async move {
        loop {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            if now >= timeout_at {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(timeout_at - now)).await;
        }

        let _terminal_guard = task_gate.lock().await;
        let mut retry_delay = std::time::Duration::from_secs(1);
        let did_expire = loop {
            let expire_request_id = task_request_id.clone();
            let result = crate::blocking::run_blocking(move || -> Result<bool> {
                let Some(db) = crate::get_session_db() else {
                    bail!("session DB is unavailable");
                };
                db.mark_ask_user_timed_out(&expire_request_id)
            })
            .await;
            match result {
                Ok(changed) => break changed,
                Err(e) => {
                    app_warn!(
                        "ask_user",
                        "owner_timeout",
                        "Failed to expire owner ask_user question {}; retrying in {}s: {}",
                        task_request_id,
                        retry_delay.as_secs(),
                        e
                    );
                    // Keep the registry entry and terminal gate while retrying.
                    // Session deletion aborts this sleep through the stored
                    // AbortHandle, so an unavailable DB cannot leak the task.
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(30));
                }
            }
        };
        if did_expire {
            crate::channel::worker::ask_user::drop_pending_by_request_id(&task_request_id).await;
            crate::tools::approval::emit_pending_interactions_changed(Some(&group.session_id));
            crate::hooks::fire_elicitation_result(&group.session_id, &task_request_id, "timeout");
            emit_ask_user_resolved(&task_request_id, &group.session_id, "timed_out", "timeout");
            let timeout_secs = group.timeout_secs.unwrap_or_else(|| {
                group
                    .questions
                    .iter()
                    .filter_map(|question| question.timeout_secs)
                    .max()
                    .unwrap_or(0)
            });
            emit_ask_user_timed_out(
                &task_request_id,
                &group.session_id,
                timeout_secs,
                false,
                group.questions.first().map(|question| {
                    crate::truncate_utf8(question.text.fallback_text(), 160).to_string()
                }),
            );
        }
        forget_owner_timeout_task(&task_request_id);
        forget_owner_terminal_gate(&task_request_id);
    });
    tasks.insert(
        request_id,
        OwnerTimeoutTask {
            abort_handle: join.abort_handle(),
            session_id: task_session_id,
        },
    );
}

/// Re-arm every durable owner-plane timeout left pending across a restart.
pub fn restore_owner_question_timeouts() -> Result<usize> {
    let Some(db) = crate::get_session_db() else {
        return Ok(0);
    };
    let groups = db.list_pending_owner_ask_user_groups()?;
    let count = groups.len();
    for group in groups {
        schedule_owner_question_timeout(group);
    }
    Ok(count)
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
    // Arm before broadcasting so an immediate response/delete cannot race past
    // task registration and leave an answered request sleeping until deadline.
    schedule_owner_question_timeout(group.clone());
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
            Err(e) => {
                cancel_owner_timeout_task(&group.request_id);
                forget_owner_terminal_gate(&group.request_id);
                let _ = mark_group_answered(&group.request_id);
                bail!("failed to serialize owner ask_user question: {e}")
            }
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
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let timeout_at = if timeout_secs > 0 {
        Some(now_secs.saturating_add(timeout_secs))
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
        timeout_secs: (timeout_secs > 0).then_some(timeout_secs),
        server_now: Some(now_secs),
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

async fn submit_owner_question_response(
    request_id: &str,
    answers: Vec<AskUserQuestionAnswer>,
) -> Result<()> {
    let terminal_gate = owner_terminal_gate(request_id);
    let terminal_guard = Arc::clone(&terminal_gate).lock_owned().await;
    // The owner-response path issues several synchronous SessionDB writes
    // (lookup + record_domain_evidence + mark_answered). Route them through the
    // blocking pool so they never pin the async worker (see `crate::blocking`).
    let request_id_owned = request_id.to_string();
    let response_result = crate::blocking::run_blocking(move || -> Result<String> {
        let Some(db) = crate::get_session_db() else {
            bail!("No pending ask_user_question request: {request_id_owned}");
        };
        let Some(group) = db.get_pending_ask_user_group_by_request_id(&request_id_owned)? else {
            bail!("No pending ask_user_question request: {request_id_owned}");
        };
        let Some(owner) = group.owner_response.clone() else {
            bail!("No pending ask_user_question request: {request_id_owned}");
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
                db.record_owner_ask_user_evidence_and_answer(&request_id_owned, input)?;
            }
            other => bail!("unsupported owner ask_user response action: {other}"),
        }
        Ok(group.session_id)
    })
    .await;
    let session_id = match response_result {
        Ok(session_id) => session_id,
        Err(error) => {
            drop(terminal_guard);
            // Invalid/already-terminal no-timeout requests must not leave a
            // permanent registry entry. A live timeout task retains its gate.
            forget_owner_terminal_gate_if_unused(request_id, &terminal_gate);
            return Err(error);
        }
    };
    cancel_owner_timeout_task(request_id);
    forget_owner_terminal_gate(request_id);
    crate::channel::worker::ask_user::drop_pending_by_request_id(request_id).await;
    crate::hooks::fire_elicitation_result(&session_id, request_id, "answered");
    emit_ask_user_resolved(request_id, &session_id, "answered", "response");
    crate::tools::approval::emit_pending_interactions_changed(Some(&session_id));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_cancel_drains_only_matching_live_questions() {
        let request_a = format!("ask-test-a-{}", uuid::Uuid::new_v4());
        let request_b = format!("ask-test-b-{}", uuid::Uuid::new_v4());
        let session_a = format!("session-a-{}", uuid::Uuid::new_v4());
        let session_b = format!("session-b-{}", uuid::Uuid::new_v4());
        let (sender_a, receiver_a) = tokio::sync::oneshot::channel();
        let (sender_b, receiver_b) = tokio::sync::oneshot::channel();
        register_ask_user_question(request_a.clone(), session_a.clone(), sender_a).await;
        register_ask_user_question(request_b.clone(), session_b, sender_b).await;

        assert_eq!(
            cancel_pending_ask_user_questions_for_session(&session_a, "user_stop").await,
            1
        );
        assert!(receiver_a.await.is_err(), "Stop must wake the blocked tool");
        assert!(is_ask_user_question_live(&request_b).await);

        cancel_pending_ask_user_question_with_source(&request_b, "test_cleanup").await;
        assert!(receiver_b.await.is_err());
    }
}
