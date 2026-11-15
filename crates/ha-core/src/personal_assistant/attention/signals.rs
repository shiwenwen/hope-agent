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
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant handoff unavailable"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let item:Option<(String,i64)>=tx.query_row("SELECT i.id,i.revision FROM assistant_execution_contexts c JOIN assistant_items i ON i.id=c.item_id JOIN chat_stream_runs s ON s.run_id=c.execution_id
            WHERE c.execution_id=?1 AND c.item_revision=i.revision AND i.state IN('open','waiting') AND s.status='running'",[execution],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((item, revision)) = item {
            tx.execute("INSERT INTO assistant_attention_handoffs(id,execution_id,item_id,item_revision,active,instance_id,created_at,expires_at)
                VALUES(?1,?2,?3,?4,1,?5,?6,?7) ON CONFLICT(id) DO NOTHING",params![id,execution,item,revision,claims::instance_id(),now,deadline.unwrap_or(now+86_400).min(now+86_400)])?;
        }
        tx.commit()?;
        drop(conn);
        changed();
        Ok(())
    }
    pub fn close_assistant_approval_handoff(&self, id: &str) -> Result<()> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("Assistant handoff unavailable"))?
            .execute(
                "UPDATE assistant_attention_handoffs SET active=0 WHERE id=?1",
                [id],
            )?;
        changed();
        Ok(())
    }
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
    let Some(db) = ctx
        .session_db
        .as_ref()
        .map(|value| value.0.clone())
        .or_else(|| crate::get_session_db().cloned())
    else {
        return Ok(());
    };
    let Some(execution) = ctx.persistence_run_id.clone() else {
        return Ok(());
    };
    let request = id.to_owned();
    db.run(move |db| {
        db.register_assistant_approval_handoff(
            &request,
            &execution,
            deadline,
            chrono::Utc::now().timestamp(),
        )
    })
    .await?;
    live_handoffs()
        .lock()
        .map_err(|_| anyhow!("Assistant handoff registry unavailable"))?
        .insert(id.into());
    handoff_databases()
        .lock()
        .map_err(|_| anyhow!("Assistant handoff registry unavailable"))?
        .insert(id.into(), Arc::downgrade(&db));
    Ok(())
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
            .run(move |db| db.close_assistant_approval_handoff(&request))
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
