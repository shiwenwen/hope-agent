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
    crate::personal_assistant::attention::signals::close_handoff_later(request_id);
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
    ownership: crate::personal_assistant::attention::NotificationOwnership,
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
        Ok(mut value) => {
            value["assistantAttention"] =
                serde_json::to_value(ownership).unwrap_or(serde_json::json!("unknown"));
            bus.emit(EVENT_ASK_USER_TIMED_OUT, value)
        }
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

/// Persistence belongs to the live tool request, including its first save.
/// The pending registry retains this target before waking a response receiver.
pub(crate) struct ToolQuestionPersistenceTarget {
    request_id: String,
    session_id: String,
    db: Option<Arc<crate::session::SessionDB>>,
    cancelled: std::sync::atomic::AtomicBool,
}
impl ToolQuestionPersistenceTarget {
    pub(crate) fn is_closed(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn is_for_request(&self, db: &Arc<crate::session::SessionDB>, id: &str) -> bool {
        self.request_id == id && self.db.as_ref().is_some_and(|own| Arc::ptr_eq(own, db))
    }
    fn close_live(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        crate::personal_assistant::attention::signals::close_handoff_later(&self.request_id);
    }
    async fn finish(&self) -> Result<()> {
        self.close_live();
        if let Some(db) = &self.db {
            let request = self.request_id.clone();
            let session = self.session_id.clone();
            db.run(move |db| db.close_live_ask_user_group(&request, &session))
                .await?;
        }
        Ok(())
    }
}
pub(crate) struct ToolQuestionPersistenceGuard {
    target: Arc<ToolQuestionPersistenceTarget>,
}
impl ToolQuestionPersistenceGuard {
    pub(crate) fn new(
        request_id: String,
        session_id: String,
        db: Option<Arc<crate::session::SessionDB>>,
    ) -> Self {
        Self {
            target: Arc::new(ToolQuestionPersistenceTarget {
                request_id,
                session_id,
                db,
                cancelled: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }
    pub(crate) fn target(&self) -> Arc<ToolQuestionPersistenceTarget> {
        self.target.clone()
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.target.is_closed()
    }
    pub(crate) async fn persist(&self, group: &AskUserQuestionGroup) -> Result<()> {
        if let Some(db) = &self.target.db {
            let target = self.target.clone();
            let group = group.clone();
            db.run(move |db| db.save_live_ask_user_group(&group, &target.cancelled))
                .await?;
        }
        Ok(())
    }
    pub(crate) async fn finish(&self) -> Result<()> {
        self.target.finish().await
    }
}
impl Drop for ToolQuestionPersistenceGuard {
    fn drop(&mut self) {
        self.target.close_live();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let target = self.target.clone();
            runtime.spawn(async move {
                let removed = {
                    let mut pending = get_pending_questions().lock().await;
                    if pending
                        .get(&target.request_id)
                        .and_then(|entry| entry.persistence.as_ref())
                        .is_some_and(|existing| Arc::ptr_eq(existing, &target))
                    {
                        pending.remove(&target.request_id)
                    } else {
                        None
                    }
                };
                if let Some(entry) = removed {
                    drop(entry.sender);
                    emit_ask_user_resolved(
                        &target.request_id,
                        &target.session_id,
                        "cancelled",
                        "execution_cancelled",
                    );
                }
                let _ = target.finish().await;
            });
        }
    }
}

struct PendingAskUserQuestion {
    sender: tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>,
    session_id: String,
    persistence: Option<Arc<ToolQuestionPersistenceTarget>>,
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
    pending.insert(
        request_id,
        PendingAskUserQuestion {
            sender,
            session_id,
            persistence: None,
        },
    );
}

pub(crate) async fn register_tool_ask_user_question(
    request_id: String,
    session_id: String,
    sender: tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>,
    persistence: Arc<ToolQuestionPersistenceTarget>,
) {
    let mut pending = get_pending_questions().lock().await;
    pending.insert(
        request_id,
        PendingAskUserQuestion {
            sender,
            session_id,
            persistence: Some(persistence),
        },
    );
}
/// Serialize initial UI publication with removal of the live sender. A
/// response that wins this mutex cannot be followed by a stale request event.
pub(crate) async fn emit_live_tool_question(
    request_id: &str,
    target: &Arc<ToolQuestionPersistenceTarget>,
    emit: impl FnOnce(),
) -> bool {
    let pending = get_pending_questions().lock().await;
    let live = pending.get(request_id).is_some_and(|entry| {
        !entry.sender.is_closed()
            && entry
                .persistence
                .as_ref()
                .is_some_and(|saved| Arc::ptr_eq(saved, target))
    });
    if !live || target.is_closed() {
        return false;
    }
    emit();
    true
}
async fn mark_live_question_answered(
    request_id: String,
    target: Option<Arc<ToolQuestionPersistenceTarget>>,
    legacy_db: Option<Arc<crate::session::SessionDB>>,
) {
    let result = if let Some(target) = target {
        target.finish().await
    } else if let Some(db) = legacy_db {
        db.run(move |db| db.mark_ask_user_answered(&request_id))
            .await
    } else {
        Ok(())
    };
    if let Err(error) = result {
        app_warn!(
            "ask_user",
            "terminal_cleanup",
            "Failed to persist question terminal cleanup: {}",
            error
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
        crate::personal_assistant::attention::signals::close_handoff_later(request_id);
        let session_id = entry.session_id;
        let persistence = entry.persistence;
        if let Some(target) = &persistence {
            target.close_live();
        }
        let legacy_db = if persistence.is_none() {
            crate::get_session_db().cloned()
        } else {
            None
        };
        let _ = entry.sender.send(answers);
        drop(pending);
        mark_live_question_answered(request_id.to_owned(), persistence, legacy_db).await;
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
    crate::personal_assistant::attention::signals::close_handoff_later(request_id);
    if let Some(entry) = removed {
        let session_id = entry.session_id;
        let persistence = entry.persistence;
        if let Some(target) = &persistence {
            target.close_live();
        }
        let legacy_db = if persistence.is_none() {
            crate::get_session_db().cloned()
        } else {
            None
        };
        drop(entry.sender);
        mark_live_question_answered(request_id.to_owned(), persistence, legacy_db).await;
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
        .map(|(request_id, entry)| (request_id.clone(), entry.persistence.clone()))
        .collect::<Vec<_>>();
    for (request_id, entry) in drained {
        if let Some(target) = &entry.persistence {
            target.close_live();
        }
        drop(entry.sender);
        emit_ask_user_resolved(&request_id, &entry.session_id, "cancelled", source);
    }
    for (request_id, target) in request_ids {
        let legacy_db = if target.is_none() {
            crate::get_session_db().cloned()
        } else {
            None
        };
        mark_live_question_answered(request_id, target, legacy_db).await;
    }
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
        .map(|(request_id, entry)| (request_id.clone(), entry.persistence.clone()))
        .collect::<Vec<_>>();
    for (request_id, entry) in drained {
        if let Some(target) = &entry.persistence {
            target.close_live();
        }
        drop(entry.sender);
        emit_ask_user_resolved(&request_id, &entry.session_id, "cancelled", source);
        crate::tools::approval::emit_pending_interactions_changed(Some(&entry.session_id));
    }
    for (request_id, target) in request_ids {
        let legacy_db = if target.is_none() {
            crate::get_session_db().cloned()
        } else {
            None
        };
        mark_live_question_answered(request_id, target, legacy_db).await;
    }
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
            crate::channel_hooks::drop_ask_user_by_request_id(&task_request_id).await;
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
                if group.source.as_deref() == Some("personal_assistant") {
                    crate::personal_assistant::attention::NotificationOwnership::Assistant
                } else {
                    crate::personal_assistant::attention::NotificationOwnership::Ordinary
                },
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
            "assistant_item_answer" => {
                let target = owner
                    .assistant_item
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Missing assistant answer target"))?;
                db.answer_assistant_question(&request_id_owned, target, &answers)?;
            }
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
    crate::channel_hooks::drop_ask_user_by_request_id(request_id).await;
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
        "assistant_item_answer" => {
            bail!("Assistant questions must be created by their atomic item service")
        }
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

/// Called only after a durable assistant answer and its continuation commit.
/// Notification cleanup must not re-answer the question or run custom hooks.
pub(crate) fn publish_committed_assistant_answer(request: &str, session: &str) {
    cancel_owner_timeout_task(request);
    forget_owner_terminal_gate(request);
    emit_ask_user_resolved(request, session, "answered", "response");
    crate::tools::approval::emit_pending_interactions_changed(Some(session));
    crate::personal_assistant::emit_inbox_changed();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let request = request.to_owned();
        runtime.spawn(async move {
            crate::channel_hooks::drop_ask_user_by_request_id(&request).await;
        });
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

    fn question_persistence_isolated(
        test: impl FnOnce(Arc<crate::session::SessionDB>, &std::path::Path),
    ) {
        let directory = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", directory.path())], || {
            let db = Arc::new(
                crate::session::SessionDB::open(&directory.path().join("questions.db")).unwrap(),
            );
            test(db, directory.path());
        });
    }
    fn question_persistence_group(request: &str, session: &str) -> AskUserQuestionGroup {
        serde_json::from_value(json!({"requestId":request,"sessionId":session,"source":"normal","questions":[{"questionId":"decision","text":"Choose one","options":[]}]})).unwrap()
    }
    fn question_persistence_pending(db: &crate::session::SessionDB, id: &str) -> bool {
        db.read_conn().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM ask_user_questions WHERE request_id=?1 AND status='pending')",[id],|row|row.get(0)).unwrap()
    }
    fn question_persistence_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    async fn question_persistence_wait_closed(db: &crate::session::SessionDB, id: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while question_persistence_pending(db, id) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the exact question database reaches a terminal state");
    }
    #[test]
    fn question_persistence_uses_one_exact_database_even_with_a_copied_request_id() {
        question_persistence_isolated(|db, root| {
            question_persistence_runtime().block_on(async {
                let other =
                    Arc::new(crate::session::SessionDB::open(&root.join("other-root.db")).unwrap());
                let request = uuid::Uuid::new_v4().to_string();
                let session = db.create_session("ha-main").unwrap();
                let foreign_session = other.create_session("ha-main").unwrap();
                let group = question_persistence_group(&request, &session.id);
                other
                    .save_ask_user_group(&question_persistence_group(&request, &foreign_session.id))
                    .unwrap();
                let guard = ToolQuestionPersistenceGuard::new(
                    request.clone(),
                    session.id,
                    Some(db.clone()),
                );
                guard.persist(&group).await.unwrap();
                mark_live_question_answered(
                    request.clone(),
                    Some(guard.target()),
                    Some(other.clone()),
                )
                .await;
                assert!(!question_persistence_pending(&db, &request));
                assert!(
                    question_persistence_pending(&other, &request),
                    "an available legacy root with the same ID must remain untouched"
                );
            })
        });
    }
    #[test]
    fn question_persistence_cleans_a_saved_question_when_handoff_admission_fails() {
        question_persistence_isolated(|db, _| {
            question_persistence_runtime().block_on(async {
                let ctx = crate::test_support::home_tool_context(&db);
                let request = uuid::Uuid::new_v4().to_string();
                let group = question_persistence_group(&request, ctx.session_id.as_ref().unwrap());
                let guard = ToolQuestionPersistenceGuard::new(
                    request.clone(),
                    group.session_id.clone(),
                    Some(db.clone()),
                );
                guard.persist(&group).await.unwrap();
                db.conn
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE runtime_control_epochs SET epoch=epoch+1 WHERE key='global_stop'",
                        [],
                    )
                    .unwrap();
                assert!(
                    !crate::personal_assistant::attention::signals::register_question_handoff(
                        &ctx,
                        &request,
                        None,
                        guard.target()
                    )
                    .await
                    .unwrap()
                );
                drop(guard);
                question_persistence_wait_closed(&db, &request).await;
                assert!(!crate::personal_assistant::attention::signals::handoff_pending(&request));
            })
        });
    }
    #[test]
    fn question_persistence_drop_during_queued_save_cannot_recreate_pending_after_cleanup() {
        question_persistence_isolated(|db, _| {
            question_persistence_runtime().block_on(async {
                let session = db.create_session("ha-main").unwrap();
                let request = uuid::Uuid::new_v4().to_string();
                let group = question_persistence_group(&request, &session.id);
                let guard = ToolQuestionPersistenceGuard::new(
                    request.clone(),
                    session.id,
                    Some(db.clone()),
                );
                let target = guard.target();
                let writer = db.clone();
                let held = db.conn.lock().unwrap();
                let (started, waiting) = tokio::sync::oneshot::channel();
                let save = tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    writer.save_live_ask_user_group(&group, &target.cancelled)
                });
                waiting.await.unwrap();
                drop(guard);
                drop(held);
                assert!(
                    save.await.unwrap().is_err(),
                    "cancel is checked under the same DB lock as save"
                );
                assert!(!question_persistence_pending(&db, &request));
            })
        });
    }
    #[test]
    fn question_persistence_drop_while_registering_leaves_no_pending_or_late_notice() {
        question_persistence_isolated(|db, _| {
            question_persistence_runtime().block_on(async{
            let ctx=crate::test_support::home_tool_context(&db);
            let request=uuid::Uuid::new_v4().to_string();
            let group=question_persistence_group(&request,ctx.session_id.as_ref().unwrap());
            let (saved,ready)=tokio::sync::oneshot::channel();
            let (proceed,go)=tokio::sync::oneshot::channel();
            let (enter,entered)=tokio::sync::oneshot::channel();
            let worker=db.clone();let id=request.clone();
            let task=tokio::spawn(async move{
                let guard=ToolQuestionPersistenceGuard::new(id.clone(),group.session_id.clone(),Some(worker));
                guard.persist(&group).await.unwrap();
                let (sender,_receiver)=tokio::sync::oneshot::channel();
                register_tool_ask_user_question(id.clone(),group.session_id.clone(),sender,guard.target()).await;
                let _=saved.send(());go.await.unwrap();let _=enter.send(());
                let _=crate::personal_assistant::attention::signals::register_question_handoff(&ctx,&id,None,guard.target()).await;
                std::future::pending::<()>().await;
            });
            ready.await.unwrap();
            let held=db.conn.lock().unwrap();
            let _=proceed.send(());entered.await.unwrap();
            task.abort();assert!(task.await.unwrap_err().is_cancelled());
            drop(held);
            question_persistence_wait_closed(&db,&request).await;
            assert!(!crate::personal_assistant::attention::signals::handoff_pending(&request));
            let active=db.read_conn().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM assistant_attention_home_handoffs WHERE id=?1 AND active=1)",[&request],|row|row.get::<_,bool>(0)).unwrap();
            assert!(!active,"late admission cannot outlive the exact question cleanup");
            assert!(!get_pending_questions().lock().await.contains_key(&request));
        })
        });
    }
}
