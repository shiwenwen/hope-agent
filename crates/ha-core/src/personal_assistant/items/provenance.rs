use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{execution_item, validate_item, AssistantError};
use crate::session::{CreateStreamRun, SessionDB, StreamRunRegistration};

/// Only the host creates this proof after resolving immutable source bindings.
/// It has no deserializer or public constructor.
#[derive(Clone)]
pub(crate) struct AssistantInjectionProof {
    item: String,
    root: String,
    revision: i64,
    session: String,
    sources: Vec<(String, String)>,
    subtask: bool,
    entry_subagent: bool,
    completed: bool,
    origin: crate::knowledge::KbAccessSource,
}

impl AssistantInjectionProof {
    pub(crate) fn origin(&self) -> crate::knowledge::KbAccessSource {
        self.origin
    }
    pub(crate) fn completed(&self) -> bool {
        self.completed
    }
    pub(crate) fn matches_source(&self, source: crate::chat_engine::ChatSource) -> bool {
        if self.entry_subagent {
            source == crate::chat_engine::ChatSource::Subagent
        } else {
            source == crate::chat_engine::ChatSource::ParentInjection
        }
    }
}

impl SessionDB {
    pub fn bind_assistant_work_source(
        &self,
        kind: &str,
        id: &str,
        parent_session: &str,
        execution: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        bind_source_with_conn(&tx, kind, id, parent_session, execution)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn assistant_injection_proof(
        &self,
        session: &str,
        sources: &[(String, String)],
    ) -> Result<Option<AssistantInjectionProof>> {
        let conn = self.read_conn()?;
        let mut proof: Option<AssistantInjectionProof> = None;
        let mut ordinary = 0;
        for (kind, id) in sources {
            match kind.as_str() {
                "job" => {
                    let job = crate::async_jobs::JobManager::get(id)?
                        .ok_or(AssistantError::Unavailable)?;
                    if job.session_id.as_deref() != Some(session) || !job.status.is_terminal() {
                        bail!(AssistantError::Unavailable)
                    }
                }
                "subagent" => {
                    let run = self
                        .get_subagent_run(id)?
                        .ok_or(AssistantError::Unavailable)?;
                    if run.parent_session_id != session
                        || !run.status.is_terminal()
                        || run.delivery_kind != crate::subagent::SubagentDeliveryKind::Parent
                        || run.owner_kind != crate::subagent::SubagentOwnerKind::ParentSession
                    {
                        bail!(AssistantError::Unavailable)
                    }
                }
                _ => bail!(AssistantError::InvalidInput),
            }
            let bound:Option<(String,String,i64,String,Option<String>,bool)>=conn.query_row(
                "SELECT item_id,root_turn_id,item_revision,parent_session_id,consumed_execution_id,parent_is_subtask
                 FROM assistant_owned_sources WHERE source_kind=?1 AND source_id=?2",
                params![kind,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
            if let Some((item, root, revision, parent, previous, subtask)) = bound {
                if parent != session {
                    bail!(AssistantError::Unavailable)
                }
                validate_item(&conn, &item)?;
                let waiting: bool = conn.query_row(
                    "SELECT reason='needs_input' FROM assistant_items WHERE id=?1",
                    [&item],
                    |r| r.get::<_, Option<bool>>(0).map(|v| v.unwrap_or(false)),
                )?;
                if (waiting || super::needs_owner_input(&conn, &item)?) && !subtask {
                    bail!(AssistantError::Busy)
                }
                let completed = previous
                    .as_deref()
                    .map(|p| source_attempt_state(&conn, p))
                    .transpose()?
                    .unwrap_or(false);
                if let Some(previous) = &proof {
                    if previous.item != item
                        || previous.revision != revision
                        || previous.subtask != subtask
                    {
                        bail!("Cannot merge different assistant continuation owners")
                    }
                    if previous.completed != completed {
                        bail!(AssistantError::Conflict)
                    }
                } else {
                    proof = Some(AssistantInjectionProof {
                        origin: item_origin(&conn, &item)?,
                        item,
                        root,
                        revision,
                        session: session.to_owned(),
                        sources: sources.to_vec(),
                        subtask,
                        entry_subagent: false,
                        completed,
                    });
                }
            } else {
                super::receipts::deny_removed_source(&conn, kind, id)?;
                ordinary += 1;
            }
        }
        if proof.is_some() && ordinary > 0 {
            bail!("Cannot merge assistant and ordinary completions")
        }
        Ok(proof)
    }

    pub(crate) fn assistant_subagent_proof(
        &self,
        session: &str,
    ) -> Result<Option<AssistantInjectionProof>> {
        let conn = self.read_conn()?;
        let bound: Option<(String, String, i64, String)> = conn
            .query_row(
                "SELECT b.item_id,b.root_turn_id,b.item_revision,b.source_id
             FROM assistant_owned_sources b JOIN subagent_runs s ON s.run_id=b.source_id
             WHERE b.source_kind='subagent' AND s.child_session_id=?1 AND s.status='running'
             ORDER BY s.started_at DESC LIMIT 1",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((item, root, revision, id)) = bound {
            validate_item(&conn, &item)?;
            return Ok(Some(AssistantInjectionProof {
                origin: item_origin(&conn, &item)?,
                item,
                root,
                revision,
                session: session.to_owned(),
                sources: vec![("subagent".to_owned(), id)],
                subtask: true,
                entry_subagent: true,
                completed: false,
            }));
        }
        let inherited: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_execution_receipts WHERE session_id=?1)
                OR EXISTS(SELECT 1 FROM assistant_source_receipts WHERE child_session_id=?1)",
            [session],
            |r| r.get(0),
        )?;
        if inherited {
            bail!("Assistant child resume lacks current execution provenance")
        }
        Ok(None)
    }

    pub fn assistant_source_owner(&self, kind: &str, id: &str) -> Result<Option<String>> {
        Ok(self.read_conn()?.query_row("SELECT item_id FROM assistant_source_receipts WHERE source_kind=?1 AND source_id=?2",params![kind,id],|r|r.get(0)).optional()?)
    }

    pub fn settle_assistant_inline_source(&self, id: &str, execution: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let Some((item, root, revision)) = execution_item(&conn, execution)? else {
            return Ok(());
        };
        conn.execute(
            "UPDATE assistant_owned_sources SET consumed_execution_id=?2,inline_settled=1
            WHERE source_kind='job' AND source_id=?1 AND item_id=?3 AND root_turn_id=?4
                AND item_revision=?5 AND consumed_execution_id IS NULL",
            params![id, execution, item, root, revision],
        )?;
        Ok(())
    }

    pub(crate) fn create_assistant_injection_stream(
        &self,
        input: &CreateStreamRun,
        proof: &AssistantInjectionProof,
    ) -> Result<StreamRunRegistration> {
        let result = self.create_assistant_injection_stream_inner(input, proof);
        if result.as_ref().is_err_and(|e| {
            e.downcast_ref::<AssistantError>() == Some(&AssistantError::LimitReached)
        }) {
            self.park_assistant_budget_exhausted(&proof.item)?;
        }
        result
    }

    fn create_assistant_injection_stream_inner(
        &self,
        input: &CreateStreamRun,
        proof: &AssistantInjectionProof,
    ) -> Result<StreamRunRegistration> {
        if input.session_id != proof.session || input.turn_id.is_some() || proof.completed {
            bail!(AssistantError::Unavailable)
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_item(&tx, &proof.item)?;
        let registration = Self::create_stream_run_in_transaction(&tx, input, None)?;
        bind_assistant_injection_with_conn(&tx, input, proof)?;
        tx.commit()?;
        Ok(registration)
    }

    pub(crate) fn create_assistant_work_stream(
        &self,
        input: &CreateStreamRun,
    ) -> Result<StreamRunRegistration> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (item, session): (String, String) = tx.query_row(
            "SELECT ar.item_id,t.session_id
            FROM assistant_item_runs ar JOIN chat_turns t ON t.id=ar.turn_id
            WHERE ar.turn_id=?1 AND t.source='session_tool' AND t.status='running'",
            [input.turn_id.as_deref()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if session != input.session_id {
            bail!(AssistantError::Unavailable)
        }
        validate_item(&tx, &item)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts
            WHERE root_turn_id=?1)",
            [input.turn_id.as_deref()],
            |r| r.get(0),
        )?;
        if exists {
            bail!(AssistantError::Conflict)
        }
        let registration = Self::create_stream_run_in_transaction(&tx, input, None)?;
        tx.commit()?;
        Ok(registration)
    }
}

pub(crate) fn bind_assistant_injection_with_conn(
    conn: &Connection,
    input: &CreateStreamRun,
    proof: &AssistantInjectionProof,
) -> Result<()> {
    validate_item(conn, &proof.item)?;
    let waiting: bool = conn.query_row(
        "SELECT reason='needs_input' FROM assistant_items WHERE id=?1",
        [&proof.item],
        |r| r.get::<_, Option<bool>>(0).map(|v| v.unwrap_or(false)),
    )?;
    if (waiting || super::needs_owner_input(conn, &proof.item)?) && !proof.subtask {
        bail!(AssistantError::Busy)
    }
    let now = chrono::Utc::now().to_rfc3339();
    let (owner_turn_limit, _, _) =
        super::owner_limits_with_conn(conn, chrono::Utc::now().timestamp())?;
    if super::owner_turns_with_conn(conn, chrono::Utc::now().timestamp())? >= owner_turn_limit {
        bail!(AssistantError::LimitReached)
    }
    let won = conn.execute(
        "UPDATE assistant_items SET turns_used=turns_used+1,updated_at=?3
        WHERE id=?1 AND revision=?2 AND state IN('open','waiting') AND turns_used<turn_limit",
        params![proof.item, proof.revision, now],
    )?;
    if won != 1 {
        let exhausted: bool = conn.query_row(
            "SELECT revision=?2 AND turns_used>=turn_limit FROM assistant_items WHERE id=?1",
            params![proof.item, proof.revision],
            |r| r.get(0),
        )?;
        if exhausted {
            bail!(AssistantError::LimitReached)
        }
        bail!(AssistantError::Stopped)
    }
    if !proof.subtask {
        conn.execute(
            "UPDATE assistant_items SET state='open',reason=NULL WHERE id=?1",
            [&proof.item],
        )?;
    }
    let mut safe_previous = Vec::<String>::new();
    for (kind, id) in &proof.sources {
        if proof.entry_subagent {
            let valid: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_owned_sources
                WHERE source_kind=?1 AND source_id=?2 AND item_id=?3 AND root_turn_id=?4
                AND item_revision=?5)",
                params![kind, id, proof.item, proof.root, proof.revision],
                |r| r.get(0),
            )?;
            if !valid {
                bail!(AssistantError::Unavailable)
            }
            continue;
        }
        let previous: Option<String> = conn.query_row(
            "SELECT consumed_execution_id FROM assistant_owned_sources
            WHERE source_kind=?1 AND source_id=?2",
            params![kind, id],
            |r| r.get(0),
        )?;
        if let Some(previous) = previous.as_deref() {
            if source_attempt_state(conn, previous)? {
                bail!(AssistantError::Conflict)
            }
            safe_previous.push(previous.to_owned());
        }
        let won = conn.execute(
            "UPDATE assistant_owned_sources SET consumed_execution_id=?1
            WHERE source_kind=?2 AND source_id=?3 AND item_id=?4 AND item_revision=?5
                AND parent_session_id=?6 AND consumed_execution_id IS ?7",
            params![
                input.run_id,
                kind,
                id,
                proof.item,
                proof.revision,
                proof.session,
                previous
            ],
        )?;
        if won != 1 {
            bail!(AssistantError::Conflict)
        }
    }
    if !proof.subtask {
        let previous = serde_json::to_string(&safe_previous)?;
        // Only this exact source's verified-safe previous consumer may return
        // its events to pending. An unrelated failed consumer keeps its claim.
        conn.execute(
            "UPDATE assistant_followups SET state='pending',consumer_execution_id=NULL
            WHERE item_id=?1 AND item_revision=?2 AND state='claimed'
                AND consumer_execution_id IN(SELECT value FROM json_each(?3))",
            params![proof.item, proof.revision, previous],
        )?;
        conn.execute(
            "UPDATE assistant_followups SET state='claimed',consumer_execution_id=?1
            WHERE id IN(SELECT id FROM assistant_followups WHERE item_id=?2 AND item_revision=?3
                AND state='pending' AND (id LIKE 'answer:%' OR id LIKE 'owner-followup:%')
                ORDER BY CASE WHEN id LIKE 'answer:%' THEN 0 ELSE 1 END,due_at,id LIMIT 8)",
            params![input.run_id, proof.item, proof.revision],
        )?;
    }
    conn.execute("INSERT INTO assistant_execution_contexts(execution_id,root_turn_id,item_id,item_revision,is_subtask)
        VALUES(?1,?2,?3,?4,?5)",params![input.run_id,proof.root,proof.item,proof.revision,proof.subtask])?;
    conn.execute(
        "INSERT INTO assistant_owner_turns(execution_id,started_at) VALUES(?1,?2)",
        params![input.run_id, chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

pub(crate) fn bind_source_with_conn(
    conn: &Connection,
    kind: &str,
    id: &str,
    parent_session: &str,
    execution: Option<&str>,
) -> Result<()> {
    let binding = execution
        .map(|e| execution_item(conn, e))
        .transpose()?
        .flatten();
    if let Some((item, root, revision)) = binding {
        validate_item(conn, &item)?;
        let live: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts c
                JOIN chat_stream_runs s ON s.run_id=c.execution_id
                WHERE c.execution_id=?1 AND s.session_id=?2 AND s.status='running')",
            params![execution, parent_session],
            |r| r.get(0),
        )?;
        if !live {
            bail!(AssistantError::Unavailable)
        }
        let parent_is_subtask: bool = conn.query_row(
            "SELECT is_subtask FROM assistant_execution_contexts
            WHERE execution_id=?1",
            [execution],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO assistant_owned_sources(source_kind,source_id,parent_session_id,
                root_turn_id,item_id,item_revision,parent_is_subtask) VALUES(?1,?2,?3,?4,?5,?6,?7)
                ON CONFLICT(source_kind,source_id) DO NOTHING",
            params![
                kind,
                id,
                parent_session,
                root,
                item,
                revision,
                parent_is_subtask
            ],
        )?;
        let matches: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_owned_sources
                WHERE source_kind=?1 AND source_id=?2 AND parent_session_id=?3
                    AND root_turn_id=?4 AND item_id=?5 AND item_revision=?6)",
            params![kind, id, parent_session, root, item, revision],
            |r| r.get(0),
        )?;
        if !matches {
            bail!(AssistantError::Conflict)
        }
    } else {
        // Missing provenance in an assistant worker is not ordinary work.
        // This is a deny predicate, never a way of guessing an item.
        let owned: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_execution_receipts c
                JOIN chat_stream_runs s ON s.run_id=c.execution_id
                WHERE s.session_id=?1 AND s.status='running')",
            [parent_session],
            |r| r.get(0),
        )?;
        if owned {
            bail!("Assistant child work requires its exact admitted execution identity")
        }
    }
    Ok(())
}

pub(crate) fn consume_source_with_conn(
    conn: &Connection,
    kind: &str,
    id: &str,
    execution: Option<&str>,
) -> Result<()> {
    let bound: Option<(String, i64, String, bool)> = conn
        .query_row(
            "SELECT item_id,item_revision,parent_session_id,parent_is_subtask
        FROM assistant_owned_sources WHERE source_kind=?1 AND source_id=?2",
            params![kind, id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((item, revision, parent, role)) = bound else {
        super::receipts::deny_removed_source(conn, kind, id)?;
        return Ok(());
    };
    validate_item(conn, &item)?;
    let execution = execution.ok_or(AssistantError::Unavailable)?;
    let live:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts c
        JOIN chat_stream_runs s ON s.run_id=c.execution_id WHERE c.execution_id=?1 AND c.item_id=?2
        AND c.item_revision=?3 AND c.is_subtask=?4 AND s.session_id=?5 AND s.status='running' AND c.closed_for_question=0)",
        params![execution,item,revision,role,parent],|r|r.get(0))?;
    if !live {
        bail!(AssistantError::Unavailable)
    }
    conn.execute(
        "UPDATE assistant_owned_sources SET consumed_execution_id=?1,inline_settled=1
        WHERE source_kind=?2 AND source_id=?3",
        params![execution, kind, id],
    )?;
    Ok(())
}

/// A claim may move only from a known terminal attempt. Unknown dispatch or
/// interrupted tool work remains inspectable and requires owner verification.
pub(super) fn source_attempt_state(conn: &Connection, execution: &str) -> Result<bool> {
    let status: String = conn.query_row(
        "SELECT status FROM chat_stream_runs WHERE run_id=?1",
        [execution],
        |r| r.get(0),
    )?;
    if status == "committed" {
        return Ok(true);
    }
    if status == "running" {
        bail!(AssistantError::Busy)
    }
    let uncertain:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_budget_reservations
        WHERE execution_id=?1 AND (settled_at IS NULL OR (kind='tool' AND replay_safe=0))) OR EXISTS(
        SELECT 1 FROM request_projection_plans WHERE run_id=?1 AND state='send_unknown')",
        [execution],|r|r.get(0))?;
    if uncertain {
        bail!("Interrupted assistant work requires verification before retry")
    }
    Ok(false)
}

#[cfg(test)]
pub(super) fn test_proof(item: String) -> AssistantInjectionProof {
    AssistantInjectionProof {
        origin: crate::knowledge::KbAccessSource::Gui,
        item,
        root: "worker-turn".into(),
        revision: 1,
        session: "work".into(),
        sources: vec![("job".into(), "job-id".into())],
        subtask: false,
        entry_subagent: false,
        completed: false,
    }
}

fn item_origin(conn: &Connection, item: &str) -> Result<crate::knowledge::KbAccessSource> {
    let im:bool=conn.query_row("SELECT s.kind='personal_assistant_im' FROM assistant_items i
        JOIN personal_assistant_requests r ON r.id=i.request_id JOIN chat_turns t ON t.id=r.source_turn_id
        JOIN sessions s ON s.id=t.session_id WHERE i.id=?1",[item],|r|r.get(0))?;
    Ok(if im {
        crate::knowledge::KbAccessSource::Im
    } else {
        crate::knowledge::KbAccessSource::Gui
    })
}
