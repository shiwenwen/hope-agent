//! Fold lower-level alerts using immutable host identities, never the latest
//! turn or a regular-session-wide guess.
use super::*;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationOwnership {
    #[default]
    Ordinary,
    Assistant,
    Unknown,
}
impl NotificationOwnership {
    pub fn is_ordinary(&self) -> bool {
        *self == Self::Ordinary
    }
}
impl SessionDB {
    pub fn assistant_notification_execution_ownership(
        &self,
        execution: &str,
    ) -> Result<NotificationOwnership> {
        let conn = self.read_conn()?;
        let owned:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_execution_receipts WHERE execution_id=?1 OR root_turn_id=?1) OR EXISTS(SELECT 1 FROM assistant_research_runs WHERE execution_id=?1) OR EXISTS(SELECT 1 FROM assistant_attention_private_executions WHERE execution_id=?1)",[execution],|row|row.get(0))?;
        if owned {
            return Ok(NotificationOwnership::Assistant);
        }
        let session:Option<String>=conn.query_row("SELECT session_id FROM chat_stream_runs WHERE run_id=?1 UNION ALL SELECT session_id FROM chat_turns WHERE id=?1 LIMIT 1",[execution],|row|row.get(0)).optional()?;
        match session {
            Some(session) => Ok(if private_source_with_conn(&conn, &session)? {
                NotificationOwnership::Assistant
            } else {
                NotificationOwnership::Ordinary
            }),
            None => Ok(NotificationOwnership::Unknown),
        }
    }
    fn notification_job_ownership(&self, job: &str) -> Result<NotificationOwnership> {
        let mut current = job.to_owned();
        let mut visited = std::collections::HashSet::new();
        for _ in 0..16 {
            if !visited.insert(current.clone()) {
                return Ok(NotificationOwnership::Unknown);
            }
            let owned:bool=self.read_conn()?.query_row("SELECT EXISTS(SELECT 1 FROM assistant_source_receipts WHERE source_kind='job' AND source_id=?1)",[&current],|row|row.get(0))?;
            if owned {
                return Ok(NotificationOwnership::Assistant);
            }
            // Group children retain a host-created parent identity. Missing
            // records are unknown; deletion cannot turn a private child alert
            // into an ordinary desktop push.
            let Some(record) = crate::async_jobs::JobManager::get(&current)? else {
                return Ok(NotificationOwnership::Unknown);
            };
            if let Some(session) = record.session_id.as_deref() {
                if private_source_with_conn(&*self.read_conn()?, session)? {
                    return Ok(NotificationOwnership::Assistant);
                }
            }
            let Some(parent) = record.group_id else {
                return Ok(NotificationOwnership::Ordinary);
            };
            current = parent;
        }
        Ok(NotificationOwnership::Unknown)
    }
    pub fn register_assistant_approval_handoff(
        &self,
        id: &str,
        execution: &str,
        deadline: Option<i64>,
        now: i64,
        admitted_epochs: Option<(u64, u64)>,
    ) -> Result<bool> {
        self.register_assistant_handoff(id, execution, "approval", deadline, now, admitted_epochs)
    }
    pub(crate) fn register_assistant_question_handoff(
        &self,
        id: &str,
        execution: &str,
        deadline: Option<i64>,
        now: i64,
        admitted_epochs: Option<(u64, u64)>,
    ) -> Result<bool> {
        self.register_assistant_handoff(id, execution, "question", deadline, now, admitted_epochs)
    }
    fn register_assistant_handoff(
        &self,
        id: &str,
        execution: &str,
        kind: &str,
        deadline: Option<i64>,
        now: i64,
        admitted_epochs: Option<(u64, u64)>,
    ) -> Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant handoff unavailable"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let item:Option<(String,i64)>=tx.query_row("SELECT i.id,i.revision FROM assistant_execution_contexts c JOIN assistant_items i ON i.id=c.item_id JOIN chat_stream_runs s ON s.run_id=c.execution_id
            WHERE c.execution_id=?1 AND c.item_revision=i.revision AND i.state IN('open','waiting') AND s.status='running'",[execution],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let registered = if let Some((item, revision)) = item.filter(|_| kind == "approval") {
            tx.execute("INSERT INTO assistant_attention_handoffs(id,execution_id,item_id,item_revision,active,instance_id,created_at,expires_at)
                VALUES(?1,?2,?3,?4,1,?5,?6,?7) ON CONFLICT(id) DO NOTHING",params![id,execution,item,revision,claims::instance_id(),now,deadline.unwrap_or(now+86_400).min(now+86_400)])?;
            true
        } else if let Some((home, global, epoch)) =
            direct_home_context(&tx, execution, admitted_epochs)?
        {
            if kind=="question" && !tx.query_row("SELECT EXISTS(SELECT 1 FROM ask_user_questions WHERE request_id=?1 AND session_id=?2 AND status='pending')",params![id,home],|row|row.get::<_,bool>(0))? {return Ok(false)}
            tx.execute("INSERT INTO assistant_attention_home_handoffs(id,kind,execution_id,home_session_id,global_stop_epoch,home_stop_epoch,active,instance_id,created_at,expires_at)
                VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,?9) ON CONFLICT(id) DO NOTHING",params![id,kind,execution,home,global,epoch,claims::instance_id(),now,deadline.unwrap_or(now+86_400).min(now+86_400)])?;
            true
        } else {
            false
        };
        tx.commit()?;
        drop(conn);
        changed();
        Ok(registered)
    }
    pub fn close_assistant_attention_handoff(&self, id: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant handoff unavailable"))?;
        let tx = conn.transaction()?;

        tx.execute(
            "UPDATE assistant_attention_handoffs SET active=0 WHERE id=?1",
            [id],
        )?;
        tx.execute(
            "UPDATE assistant_attention_home_handoffs SET active=0 WHERE id=?1",
            [id],
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(())
    }
}

/// A direct Home gate requires the exact live interactive stream and its user
/// turn. The shared source proof enforces Desktop/HTTP, privacy and Stop.
fn direct_home_context(
    conn: &Connection,
    execution: &str,
    admitted_epochs: Option<(u64, u64)>,
) -> Result<Option<(String, i64, i64)>> {
    let Some((admitted_global, admitted_home)) = admitted_epochs else {
        return Ok(None);
    };
    let source:Option<(String,String)>=conn.query_row("SELECT stream.session_id,stream.turn_id FROM chat_stream_runs stream JOIN chat_turns turn ON turn.id=stream.turn_id AND turn.session_id=stream.session_id JOIN sessions home ON home.id=stream.session_id
        WHERE stream.run_id=?1 AND stream.status='running' AND turn.status='running' AND turn.source IN('desktop','http') AND home.kind='personal_assistant' AND home.incognito=0 AND home.archived_at IS NULL AND home.parent_session_id IS NULL",[execution],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((home, turn)) = source else {
        return Ok(None);
    };
    let global: i64 = conn.query_row(
        "SELECT epoch FROM runtime_control_epochs WHERE key='global_stop'",
        [],
        |row| row.get(0),
    )?;
    let epoch: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=?1",
        [&home],
        |row| row.get(0),
    )?;
    if u64::try_from(global)? != admitted_global || u64::try_from(epoch)? != admitted_home {
        return Ok(None);
    }
    match super::super::owner_source_turn(conn, &turn, admitted_global) {
        Ok((owner, _)) if owner == home => {}
        Ok(_) => return Ok(None),
        Err(error) if error.is::<super::super::AssistantError>() => return Ok(None),
        Err(error) => return Err(error),
    }
    Ok(Some((home, global, epoch)))
}

pub(super) fn home_handoff_current(
    conn: &Connection,
    candidate: &Candidate,
    now: i64,
) -> Result<bool> {
    if !handoff_pending(&candidate.subject) {
        return Ok(false);
    }
    let kind = match candidate.kind.as_str() {
        "home_approval" => "approval",
        "home_question" => "question",
        _ => return Ok(false),
    };
    let row:Option<(String,String,i64,i64)>=conn.query_row("SELECT execution_id,home_session_id,global_stop_epoch,home_stop_epoch FROM assistant_attention_home_handoffs WHERE id=?1 AND kind=?2 AND active=1 AND instance_id=?3 AND expires_at>?4",params![candidate.subject,kind,claims::instance_id(),now],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    let Some((execution, home, global, epoch)) = row else {
        return Ok(false);
    };
    if (home.as_str(), global, epoch)
        != (
            candidate.home.as_str(),
            candidate.global_epoch,
            candidate.home_epoch,
        )
        || direct_home_context(
            conn,
            &execution,
            Some((u64::try_from(global)?, u64::try_from(epoch)?)),
        )? != Some((home, global, epoch))
    {
        return Ok(false);
    }
    if kind == "question" {
        return Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM ask_user_questions WHERE request_id=?1 AND session_id=?2 AND status='pending')",params![candidate.subject,candidate.home],|row|row.get(0))?);
    }
    Ok(true)
}

pub(crate) async fn classify_context(
    ctx: &crate::tool_defs::ToolExecContext,
) -> NotificationOwnership {
    let db = ctx
        .session_db
        .as_ref()
        .map(|value| value.0.clone())
        .or_else(|| crate::get_session_db().cloned());
    let Some(db) = db else {
        return NotificationOwnership::Unknown;
    };
    let Some(execution) = ctx
        .persistence_run_id
        .clone()
        .or_else(|| ctx.turn_id.clone())
    else {
        return NotificationOwnership::Unknown;
    };
    match db
        .run(move |db| db.assistant_notification_execution_ownership(&execution))
        .await
    {
        Ok(ownership) => ownership,
        Err(_) => NotificationOwnership::Unknown,
    }
}

pub(crate) fn emit_job_completed(mut payload: serde_json::Value) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let db = crate::get_session_db().cloned();
    let job = payload
        .get("job_id")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    if let (Ok(runtime), Some(db), Some(job)) = (tokio::runtime::Handle::try_current(), db, job) {
        runtime.spawn(async move {
            let ownership = db
                .run(move |db| db.notification_job_ownership(&job))
                .await
                .unwrap_or(NotificationOwnership::Unknown);
            payload["assistantAttention"] = serde_json::to_value(ownership)
                .unwrap_or(serde_json::Value::String("unknown".into()));
            bus.emit("job:completed", payload);
        });
    } else {
        payload["assistantAttention"] = serde_json::Value::String("unknown".into());
        bus.emit("job:completed", payload);
    }
}

fn handoff_databases() -> &'static Mutex<std::collections::HashMap<String, Weak<SessionDB>>> {
    static DATABASES: OnceLock<Mutex<std::collections::HashMap<String, Weak<SessionDB>>>> =
        OnceLock::new();
    DATABASES.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}
pub(crate) async fn register_handoff(
    ctx: &crate::tool_defs::ToolExecContext,
    id: &str,
    deadline: Option<i64>,
) -> Result<()> {
    register_live_handoff(ctx, id, deadline, None)
        .await
        .map(|_| ())
}
pub(crate) async fn register_question_handoff(
    ctx: &crate::tool_defs::ToolExecContext,
    id: &str,
    deadline: Option<i64>,
    target: Arc<crate::ask_user::ToolQuestionPersistenceTarget>,
) -> Result<bool> {
    register_live_handoff(ctx, id, deadline, Some(target)).await
}
async fn register_live_handoff(
    ctx: &crate::tool_defs::ToolExecContext,
    id: &str,
    deadline: Option<i64>,
    target: Option<Arc<crate::ask_user::ToolQuestionPersistenceTarget>>,
) -> Result<bool> {
    let question = target.is_some();
    let Some(db) = ctx
        .session_db
        .as_ref()
        .map(|value| value.0.clone())
        .or_else(|| crate::get_session_db().cloned())
    else {
        return Ok(false);
    };
    let Some(execution) = ctx.persistence_run_id.clone() else {
        return Ok(false);
    };
    if target
        .as_ref()
        .is_some_and(|target| !target.is_for_request(&db, id))
    {
        return Ok(false);
    }
    let request = id.to_owned();
    let admitted_epochs = ctx
        .turn_admitted_global_stop_epoch
        .zip(ctx.turn_admitted_stop_epoch);
    let registered = db
        .run(move |db| {
            if question {
                db.register_assistant_question_handoff(
                    &request,
                    &execution,
                    deadline,
                    chrono::Utc::now().timestamp(),
                    admitted_epochs,
                )
            } else {
                db.register_assistant_approval_handoff(
                    &request,
                    &execution,
                    deadline,
                    chrono::Utc::now().timestamp(),
                    admitted_epochs,
                )
            }
        })
        .await?;
    if !registered {
        return Ok(false);
    }
    activate_live_handoff(&db, id, target.as_ref())
}

pub(crate) fn activate_live_handoff(
    db: &Arc<SessionDB>,
    id: &str,
    target: Option<&Arc<crate::ask_user::ToolQuestionPersistenceTarget>>,
) -> Result<bool> {
    {
        let mut live = live_handoffs()
            .lock()
            .map_err(|_| anyhow!("Assistant handoff registry unavailable"))?;
        // close_live stores closed before taking this same mutex to remove.
        // A delayed DB registration therefore cannot resurrect a completed gate.
        if target.is_some_and(|target| target.is_closed() || !target.is_for_request(db, id)) {
            return Ok(false);
        }
        live.insert(id.into());
    }
    handoff_databases()
        .lock()
        .map_err(|_| anyhow!("Assistant handoff registry unavailable"))?
        .insert(id.into(), Arc::downgrade(db));
    Ok(true)
}

pub(crate) async fn home_context_current(ctx: &crate::tool_defs::ToolExecContext) -> Result<bool> {
    let Some(db) = ctx
        .session_db
        .as_ref()
        .map(|db| db.0.clone())
        .or_else(|| crate::get_session_db().cloned())
    else {
        return Ok(false);
    };
    let Some(execution) = ctx.persistence_run_id.clone() else {
        return Ok(false);
    };
    let epochs = ctx
        .turn_admitted_global_stop_epoch
        .zip(ctx.turn_admitted_stop_epoch);
    db.run(move |db| {
        let conn = db.read_conn()?;
        Ok(direct_home_context(&conn, &execution, epochs)?.is_some())
    })
    .await
}

pub(crate) async fn close_handoff(id: &str) {
    let db = handoff_databases()
        .lock()
        .ok()
        .and_then(|mut entries| entries.remove(id))
        .and_then(|weak| weak.upgrade());
    if let Some(db) = db {
        let request = id.to_owned();
        let _ = db
            .run(move |db| db.close_assistant_attention_handoff(&request))
            .await;
    }
}

pub(crate) fn private_source_with_conn(conn: &Connection, session: &str) -> Result<bool> {
    // A work conversation may contain both assistant-owned and ordinary owner
    // turns. Its broad export/privacy marker is never an execution identity.
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_attention_private_origins WHERE session_id=?1)",
        [session],
        |row| row.get(0),
    )?)
}
pub(crate) async fn requires_managed_home_work(
    ctx: &crate::tool_defs::ToolExecContext,
) -> Result<bool> {
    let Some(db) = ctx
        .session_db
        .as_ref()
        .map(|db| db.0.clone())
        .or_else(|| crate::get_session_db().cloned())
    else {
        return Ok(false);
    };
    let Some(session) = ctx.session_id.clone() else {
        return Ok(false);
    };
    let execution = ctx
        .persistence_run_id
        .clone()
        .or_else(|| ctx.turn_id.clone());
    db.run(move |db| {
        if let Some(execution) = execution {
            if db.assistant_execution_role(&execution)?.is_some() {
                return Ok(false);
            }
        }
        private_source_with_conn(&*db.read_conn()?, &session)
    })
    .await
}

pub(crate) async fn classify_private_session(session: Option<&str>) -> NotificationOwnership {
    let Some(session) = session.map(str::to_owned) else {
        return NotificationOwnership::Ordinary;
    };
    let Some(db) = crate::get_session_db().cloned() else {
        return NotificationOwnership::Unknown;
    };
    match db
        .run(move |db| private_source_with_conn(&*db.read_conn()?, &session))
        .await
    {
        Ok(true) => NotificationOwnership::Assistant,
        Ok(false) => NotificationOwnership::Ordinary,
        Err(_) => NotificationOwnership::Unknown,
    }
}

fn live_handoffs() -> &'static Mutex<std::collections::HashSet<String>> {
    static LIVE: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}
pub(crate) fn handoff_pending(id: &str) -> bool {
    live_handoffs().lock().is_ok_and(|live| live.contains(id))
}
pub(crate) fn close_handoff_later(id: &str) {
    if let Ok(mut live) = live_handoffs().lock() {
        live.remove(id);
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let id = id.to_owned();
        runtime.spawn(async move {
            close_handoff(&id).await;
        });
    }
}
