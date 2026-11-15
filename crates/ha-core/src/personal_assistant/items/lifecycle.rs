use std::sync::Arc;

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};

use super::{execution_item, validate_item};
use crate::session::SessionDB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantPendingWork {
    Ready,
    Running,
    Unknown,
}

struct Cancellation {
    requests: Vec<String>,
    streams: Vec<(String, String)>,
    sources: Vec<(String, String, String)>,
}

impl SessionDB {
    /// Resolve immutable item-owned identities before consulting either
    /// execution ledger. Session-wide UI snapshots are never authority here.
    pub fn assistant_pending_work(&self, execution: &str) -> Result<AssistantPendingWork> {
        let conn = self.read_conn()?;
        let Some((item, _, revision)) = execution_item(&conn, execution)? else {
            return Ok(AssistantPendingWork::Unknown);
        };
        let sources = {
            let mut stmt = conn.prepare(
                "SELECT source_kind,source_id,consumed_execution_id,inline_settled
                FROM assistant_owned_sources WHERE item_id=?1 AND item_revision=?2",
            )?;
            let rows = stmt
                .query_map(params![item, revision], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut unknown = false;
        for (kind, id, consumed, inline) in sources {
            if let Some(consumer) = consumed {
                let status: Option<String> = conn
                    .query_row(
                        "SELECT status FROM chat_stream_runs
                    WHERE run_id=?1",
                        [&consumer],
                        |r| r.get(0),
                    )
                    .optional()?;
                if status.as_deref() == Some("running") {
                    return Ok(AssistantPendingWork::Running);
                }
                if inline || status.as_deref() == Some("committed") {
                    continue;
                }
                match super::provenance::source_attempt_state(&conn, &consumer) {
                    Ok(false) => return Ok(AssistantPendingWork::Running),
                    Ok(true) => {}
                    Err(_) => unknown = true,
                }
                continue;
            }
            if inline {
                continue;
            }

            match kind.as_str() {
                "job" => match crate::async_jobs::JobManager::get(&id)? {
                    Some(job) if !job.status.is_terminal() || !job.injected => {
                        return Ok(AssistantPendingWork::Running)
                    }
                    _ => unknown = true,
                },
                "subagent" => match self.get_subagent_run(&id)? {
                    Some(run) if !run.status.is_terminal() => {
                        return Ok(AssistantPendingWork::Running)
                    }
                    // A Group alone owns parent continuation for these children.
                    Some(run)
                        if run.delivery_kind == crate::subagent::SubagentDeliveryKind::Group => {}
                    Some(_) => {
                        let state: Option<String> = conn
                            .query_row(
                                "SELECT state FROM subagent_result_deliveries
                            WHERE run_id=?1",
                                [id],
                                |r| r.get(0),
                            )
                            .optional()?;
                        if state
                            .as_deref()
                            .is_some_and(|s| s == "pending" || s.starts_with("injecting"))
                        {
                            return Ok(AssistantPendingWork::Running);
                        }
                        unknown = true;
                    }
                    None => unknown = true,
                },
                _ => unknown = true,
            }
        }
        Ok(if unknown {
            AssistantPendingWork::Unknown
        } else {
            AssistantPendingWork::Ready
        })
    }

    fn assistant_cancellation(&self, id: &str) -> Result<Cancellation> {
        let conn = self.read_conn()?;
        let requests = {
            let mut s = conn.prepare(
                "SELECT r.id FROM personal_assistant_requests r WHERE
                (r.id=(SELECT request_id FROM assistant_items WHERE id=?1)
                 OR r.id IN(SELECT request_id FROM assistant_followups WHERE item_id=?1))
                AND r.status IN('queued','running','cancellation_requested')",
            )?;
            let rows = s
                .query_map([id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let streams = {
            let mut s = conn.prepare(
                "SELECT s.run_id,s.session_id FROM chat_stream_runs s
                JOIN assistant_execution_contexts c ON c.execution_id=s.run_id
                WHERE c.item_id=?1 AND s.status='running'",
            )?;
            let rows = s
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let sources = {
            let mut s = conn.prepare(
                "SELECT source_kind,source_id,parent_session_id
                FROM assistant_owned_sources WHERE item_id=?1",
            )?;
            let rows = s
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        Ok(Cancellation {
            requests,
            streams,
            sources,
        })
    }

    /// Also runs in the Primary: a Secondary's persistent Stop must converge
    /// even when its process has none of the executing process's tokens.
    pub fn assistant_stopped_work(&self) -> Result<Vec<String>> {
        let items = {
            let conn = self.read_conn()?;
            let mut s = conn.prepare("SELECT id,state FROM assistant_items WHERE state<>'done'")?;
            let rows = s
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .filter(|(id, state)| state == "stopped" || validate_item(&conn, id).is_err())
                .collect::<Vec<_>>()
        };
        for (id, state) in &items {
            if state == "stopped" {
                continue;
            }
            self.stop_assistant_item(id)?;
            let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            conn.execute(
                "UPDATE assistant_items SET reason='scope_or_stop_changed' WHERE id=?1",
                [id],
            )?;
        }
        let items = items.into_iter().map(|(id, _)| id).collect();
        Ok(items)
    }
}

pub async fn stop_item(db: Arc<SessionDB>, id: String) -> Result<()> {
    let stopped = id.clone();
    db.run(move |db| db.stop_assistant_item(&stopped)).await?;
    cancel_item_work(db, id).await
}

pub async fn cancel_item_work(db: Arc<SessionDB>, id: String) -> Result<()> {
    let plan = db.run(move |db| db.assistant_cancellation(&id)).await?;
    let mut failure = None;
    for request in plan.requests {
        if let Err(error) = crate::tools::cancel_dispatch_request_only(db.clone(), request).await {
            failure = Some(error)
        }
    }
    for (run, session) in plan.streams {
        crate::chat_engine::durability::request_stop_for_exact_run(&run, &session);
    }
    for (kind, id, session) in plan.sources {
        crate::subagent::request_cancel_parent_injection(&session, &id);
        let task_kind = match kind.as_str() {
            "job" => crate::runtime_tasks::RuntimeTaskKind::AsyncJob,
            "subagent" => crate::runtime_tasks::RuntimeTaskKind::Subagent,
            _ => continue,
        };
        let source = id.clone();
        let source_kind = kind.clone();
        let active = db
            .run(move |db| -> Result<bool> {
                Ok(if source_kind == "job" {
                    crate::async_jobs::JobManager::get(&source)?
                        .is_some_and(|j| !j.status.is_terminal())
                } else {
                    db.get_subagent_run(&source)?
                        .is_some_and(|r| !r.status.is_terminal())
                })
            })
            .await;
        match active {
            Ok(true) => {
                if let Err(error) = crate::runtime_tasks::cancel_runtime_task(task_kind, &id).await
                {
                    failure = Some(error)
                }
            }
            Err(error) => failure = Some(error),
            _ => {}
        }
    }
    if let Some(error) = failure {
        Err(error)
    } else {
        Ok(())
    }
}
