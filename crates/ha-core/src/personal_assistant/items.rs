//! Lightweight owner work. Request/turn/job ledgers remain execution truth.
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{check_stop, emit_inbox_changed, stop_epoch, validate_target, AssistantError};
use crate::session::SessionDB;
mod budget;
mod lifecycle;
mod provenance;
mod questions;
mod receipts;
pub use budget::{begin_model_budget, begin_tool_budget, check_execution, check_tool_capability};
pub(crate) use budget::{bounded_auxiliary_call, custom_hooks_blocked, execution_scope};
pub use lifecycle::{cancel_item_work, stop_item, AssistantPendingWork};
pub(crate) use provenance::{
    bind_source_with_conn, consume_source_with_conn, AssistantInjectionProof,
};
pub use questions::AssistantAnswerTarget;

pub const ITEM_TURNS: i64 = 8;
pub const ITEM_TOOLS: i64 = 64;
pub const ITEM_ACTIVE_SECONDS: i64 = 1_800;
pub const OWNER_TURNS: i64 = 16;
pub const OWNER_TOOLS: i64 = 128;
pub const OWNER_ACTIVE_SECONDS: i64 = 3_600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Open,
    Waiting,
    Done,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantItem {
    pub id: String,
    pub request_id: String,
    pub target_session_id: String,
    pub instruction: String,
    pub state: ItemState,
    pub reason: Option<String>,
    pub revision: i64,
    pub turns_used: i64,
    pub tools_used: i64,
    pub active_seconds: i64,
    pub turn_limit: i64,
    pub tool_limit: i64,
    pub active_seconds_limit: i64,
    pub updated_at: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantUpdate {
    pub id: String,
    pub item_id: String,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub summary_code: Option<String>,
    pub result_message_id: Option<i64>,
    pub created_at: String,
    pub viewed_at: Option<String>,
}

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assistant_items(
            id TEXT PRIMARY KEY,
            request_id TEXT NOT NULL UNIQUE REFERENCES personal_assistant_requests(id) ON DELETE CASCADE,
            state TEXT NOT NULL DEFAULT 'open' CHECK(state IN('open','waiting','done','stopped')),
            reason TEXT,
            revision INTEGER NOT NULL DEFAULT 1,
            stop_epoch INTEGER NOT NULL DEFAULT 0,
            turns_used INTEGER NOT NULL DEFAULT 0,
            tools_used INTEGER NOT NULL DEFAULT 0,
            active_seconds INTEGER NOT NULL DEFAULT 0,
            turn_limit INTEGER NOT NULL DEFAULT 8,
            tool_limit INTEGER NOT NULL DEFAULT 64,
            active_seconds_limit INTEGER NOT NULL DEFAULT 1800,
            expires_at INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS assistant_item_runs(
            turn_id TEXT PRIMARY KEY REFERENCES chat_turns(id) ON DELETE CASCADE,
            item_id TEXT NOT NULL REFERENCES assistant_items(id) ON DELETE CASCADE,
            request_id TEXT NOT NULL UNIQUE REFERENCES personal_assistant_requests(id) ON DELETE CASCADE,
            continuation_owner TEXT NOT NULL CHECK(continuation_owner IN('worker','assistant')),
            started_at INTEGER NOT NULL,
            settled_at INTEGER,
            tools_used INTEGER NOT NULL DEFAULT 0,
            active_seconds INTEGER NOT NULL DEFAULT 0,
            tool_inflight INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS assistant_runs_item ON assistant_item_runs(item_id);
         CREATE INDEX IF NOT EXISTS assistant_runs_time ON assistant_item_runs(started_at);
         CREATE TABLE IF NOT EXISTS assistant_execution_contexts(
            execution_id TEXT PRIMARY KEY REFERENCES chat_stream_runs(run_id) ON DELETE CASCADE,
            root_turn_id TEXT NOT NULL REFERENCES assistant_item_runs(turn_id) ON DELETE CASCADE,
            item_id TEXT NOT NULL REFERENCES assistant_items(id) ON DELETE CASCADE,
            item_revision INTEGER NOT NULL,
            is_subtask INTEGER NOT NULL DEFAULT 0,
            closed_for_question INTEGER NOT NULL DEFAULT 0,
            settled INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS assistant_owned_sources(
            source_kind TEXT NOT NULL,
            source_id TEXT NOT NULL,
            parent_session_id TEXT NOT NULL,
            root_turn_id TEXT NOT NULL REFERENCES assistant_item_runs(turn_id) ON DELETE CASCADE,
            item_id TEXT NOT NULL REFERENCES assistant_items(id) ON DELETE CASCADE,
            item_revision INTEGER NOT NULL,
            parent_is_subtask INTEGER NOT NULL,
            consumed_execution_id TEXT,
            inline_settled INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(source_kind,source_id)
         );
         CREATE TABLE IF NOT EXISTS assistant_owner_turns(
            execution_id TEXT PRIMARY KEY,
            started_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS assistant_budget_grants(
            id TEXT PRIMARY KEY,
            item_id TEXT NOT NULL,
            issued_at INTEGER NOT NULL,
            turns INTEGER NOT NULL,
            tools INTEGER NOT NULL,
            seconds INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS assistant_budget_reservations(
            token TEXT PRIMARY KEY,
            item_id TEXT NOT NULL,
            root_turn_id TEXT NOT NULL,
            execution_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            replay_safe INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER NOT NULL,
            reserved_seconds INTEGER NOT NULL,
            charged_seconds INTEGER,
            settled_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS assistant_item_reports(
            execution_id TEXT PRIMARY KEY REFERENCES assistant_execution_contexts(execution_id) ON DELETE CASCADE,
            item_revision INTEGER NOT NULL,
            outcome TEXT NOT NULL CHECK(outcome IN('done','waiting','continue','failed')),
            summary TEXT NOT NULL,
            next_instruction TEXT,
            question_id TEXT,
            created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS assistant_updates(
            id TEXT PRIMARY KEY,
            item_id TEXT NOT NULL REFERENCES assistant_items(id) ON DELETE CASCADE,
            event_key TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            summary TEXT NOT NULL,
            summary_code TEXT,
            result_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
            source_revision INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            viewed_at TEXT
         );
         CREATE INDEX IF NOT EXISTS assistant_updates_time ON assistant_updates(created_at,id);
         CREATE TABLE IF NOT EXISTS assistant_followups(
            id TEXT PRIMARY KEY,
            item_id TEXT NOT NULL REFERENCES assistant_items(id) ON DELETE CASCADE,
            item_revision INTEGER NOT NULL,
            source_turn_id TEXT NOT NULL,
            instruction TEXT NOT NULL,
            due_at INTEGER NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN('pending','queued','claimed','cancelled')),
            request_id TEXT UNIQUE,
            consumer_execution_id TEXT
         );
         CREATE TABLE IF NOT EXISTS assistant_item_questions(
            question_id TEXT PRIMARY KEY REFERENCES ask_user_questions(request_id) ON DELETE CASCADE,
            item_id TEXT NOT NULL REFERENCES assistant_items(id) ON DELETE CASCADE,
            item_revision INTEGER NOT NULL,
            execution_id TEXT NOT NULL REFERENCES assistant_execution_contexts(execution_id) ON DELETE CASCADE,
            answer_json TEXT,
            UNIQUE(execution_id)
         );",
    )?;
    receipts::migrate(conn)
}

pub(super) fn create_for_request(conn: &Connection, request_id: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO assistant_items(id,request_id,created_at,updated_at,expires_at)
         VALUES(?1,?1,?2,?2,?3) ON CONFLICT(request_id) DO NOTHING",
        params![request_id, now, chrono::Utc::now().timestamp() + 7 * 86_400],
    )?;
    Ok(())
}

fn validate_item(conn: &Connection, id: &str) -> Result<()> {
    let (home, target, epoch, source_stop, target_stop, state): (
        String,
        String,
        i64,
        i64,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT r.home_session_id,r.target_session_id,r.global_stop_epoch,
                r.source_stop_epoch,r.target_stop_epoch,i.state
         FROM assistant_items i JOIN personal_assistant_requests r ON r.id=i.request_id
         WHERE i.id=?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or(AssistantError::Unavailable)?;
    if state == "stopped" {
        bail!(AssistantError::Stopped);
    }
    let expires: i64 = conn.query_row(
        "SELECT expires_at FROM assistant_items WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    if chrono::Utc::now().timestamp() >= expires {
        bail!("Assistant delegation expired")
    }
    validate_target(conn, &target)?;
    check_stop(conn, &home, epoch.max(0) as u64)?;
    check_stop(conn, &target, epoch.max(0) as u64)?;
    if stop_epoch(conn, &home)? != source_stop || stop_epoch(conn, &target)? != target_stop {
        bail!(AssistantError::Stopped);
    }
    let source: String = conn.query_row(
        "SELECT source_turn_id FROM personal_assistant_requests WHERE id=(
            SELECT request_id FROM assistant_items WHERE id=?1)",
        [id],
        |r| r.get(0),
    )?;
    let im: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_turns t JOIN sessions s ON s.id=t.session_id
         WHERE t.id=?1 AND s.kind='personal_assistant_im')",
        [&source],
        |r| r.get(0),
    )?;
    if im {
        crate::personal_assistant_im::revalidate_assistant_im_dispatch(
            conn,
            &source,
            &target,
            chrono::Utc::now().timestamp(),
        )?;
    }
    Ok(())
}

pub(super) fn admit_with_conn(conn: &Connection, request_id: &str, _turn: &str) -> Result<()> {
    let item: Option<String> = conn
        .query_row(
            "SELECT id FROM assistant_items WHERE request_id=?1
         UNION ALL SELECT item_id FROM assistant_followups WHERE request_id=?1 LIMIT 1",
            [request_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = item else { return Ok(()) };
    validate_item(conn, &id)?;
    if needs_owner_input(conn, &id)? {
        bail!(AssistantError::Busy)
    }
    let now = chrono::Utc::now().timestamp();
    let turns = owner_turns_with_conn(conn, now)?;
    let (tools, seconds): (i64, i64) = conn.query_row(
        "SELECT
        COALESCE(SUM(CASE WHEN kind='tool' THEN 1 ELSE 0 END),0),
        COALESCE(SUM(COALESCE(charged_seconds,reserved_seconds)),0)
        FROM assistant_budget_reservations WHERE started_at>=?1",
        [now - 86_400],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (owner_turns, owner_tools, owner_seconds) = owner_limits_with_conn(conn, now)?;
    let accepted = conn.execute(
        "UPDATE assistant_items SET turns_used=turns_used+1
         WHERE id=?1 AND state IN('open','waiting') AND turns_used<turn_limit
           AND tools_used<tool_limit AND active_seconds<active_seconds_limit
           AND ?2<?3 AND ?4<?5 AND ?6<?7",
        params![
            id,
            turns,
            owner_turns,
            tools,
            owner_tools,
            seconds,
            owner_seconds
        ],
    )?;
    if accepted == 0 {
        bail!(AssistantError::LimitReached);
    }
    conn.execute(
        "INSERT INTO assistant_owner_turns(execution_id,started_at) VALUES(?1,?2)",
        params![_turn, now],
    )?;
    Ok(())
}

pub(crate) fn owner_turns_with_conn(conn: &Connection, now: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM assistant_owner_turns WHERE started_at>=?1",
        [now - 86_400],
        |r| r.get(0),
    )?)
}

fn owner_limits_with_conn(conn: &Connection, now: i64) -> Result<(i64, i64, i64)> {
    let (turns,tools,seconds):(i64,i64,i64)=conn.query_row("SELECT COALESCE(SUM(turns),0),
        COALESCE(SUM(tools),0),COALESCE(SUM(seconds),0) FROM assistant_budget_grants WHERE issued_at>=?1",
        [now-86_400],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    Ok((
        OWNER_TURNS + turns,
        OWNER_TOOLS + tools,
        OWNER_ACTIVE_SECONDS + seconds,
    ))
}

/// Called after the visible turn INSERT, in the same admission transaction.
pub(crate) fn link_admitted_turn(conn: &Connection, turn: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO assistant_item_runs(turn_id,item_id,request_id,continuation_owner,started_at)
        SELECT ?1,i.id,r.id,'worker',?2 FROM personal_assistant_requests r
        JOIN assistant_items i ON i.request_id=r.id WHERE r.turn_id=?1
        UNION ALL SELECT ?1,f.item_id,r.id,'worker',?2 FROM personal_assistant_requests r
        JOIN assistant_followups f ON f.request_id=r.id WHERE r.turn_id=?1",
        params![turn, chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

pub(crate) fn bind_stream_with_conn(
    conn: &Connection,
    input: &crate::session::CreateStreamRun,
) -> Result<()> {
    let Some(turn) = input.turn_id.as_deref() else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO assistant_execution_contexts(execution_id,root_turn_id,item_id,item_revision)
        SELECT ?1,ar.turn_id,ar.item_id,i.revision FROM assistant_item_runs ar
        JOIN assistant_items i ON i.id=ar.item_id WHERE ar.turn_id=?2",
        params![input.run_id, turn],
    )?;
    conn.execute("UPDATE assistant_followups SET state='claimed',consumer_execution_id=?1
        WHERE state='queued' AND request_id IN(SELECT request_id FROM assistant_item_runs WHERE turn_id=?2)",
        params![input.run_id,turn])?;
    // Explicit Continue carries undelivered owner data to a verification turn;
    // it does not certify the failed consumer's effects or repeat its writes.
    conn.execute("UPDATE assistant_followups SET state='claimed',consumer_execution_id=?1 WHERE id IN(
        SELECT f.id FROM assistant_followups f JOIN assistant_item_runs ar ON ar.item_id=f.item_id
        JOIN assistant_followups resume ON resume.request_id=ar.request_id AND resume.id LIKE 'owner-continue:%'
        LEFT JOIN chat_stream_runs old ON old.run_id=f.consumer_execution_id
        WHERE ar.turn_id=?2 AND (f.id LIKE 'owner-followup:%' OR f.id LIKE 'answer:%')
            AND (f.state='pending' OR (f.state='claimed' AND old.status IN('failed','interrupted','recovered')))
        ORDER BY CASE WHEN f.id LIKE 'answer:%' THEN 0 ELSE 1 END,f.due_at,f.id LIMIT 8)",
        params![input.run_id,turn])?;
    Ok(())
}

fn execution_item(conn: &Connection, execution: &str) -> Result<Option<(String, String, i64)>> {
    let current=conn.query_row("SELECT item_id,root_turn_id,item_revision FROM assistant_execution_contexts WHERE execution_id=?1",[execution],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if current.is_none() {
        receipts::deny_removed_execution(conn, execution)?;
    }
    Ok(current)
}

fn needs_owner_input(conn: &Connection, item: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_item_questions iq
        JOIN ask_user_questions q ON q.request_id=iq.question_id
        WHERE iq.item_id=?1 AND q.status='pending')",
        [item],
        |r| r.get(0),
    )?)
}

fn has_internal_continuation(conn: &Connection, item: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts c
        JOIN chat_stream_runs s ON s.run_id=c.execution_id WHERE c.item_id=?1 AND s.status='running')
        OR EXISTS(SELECT 1 FROM assistant_owned_sources b LEFT JOIN chat_stream_runs consumer
            ON consumer.run_id=b.consumed_execution_id
            WHERE b.item_id=?1 AND b.inline_settled=0 AND (consumer.status IS NULL OR consumer.status<>'committed')
            AND NOT EXISTS(SELECT 1 FROM subagent_runs child WHERE b.source_kind='subagent'
                AND child.run_id=b.source_id AND child.delivery_kind='group'
                AND child.status IN('completed','error','timeout','killed','interrupted')))
        OR EXISTS(SELECT 1 FROM assistant_item_questions iq JOIN ask_user_questions q ON q.request_id=iq.question_id
            WHERE iq.item_id=?1 AND q.status='pending')",[item],|r|r.get(0))?)
}

fn item_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantItem> {
    let state: String = r.get(4)?;
    Ok(AssistantItem {
        id: r.get(0)?,
        request_id: r.get(1)?,
        target_session_id: r.get(2)?,
        instruction: r.get(3)?,
        state: match state.as_str() {
            "waiting" => ItemState::Waiting,
            "done" => ItemState::Done,
            "stopped" => ItemState::Stopped,
            _ => ItemState::Open,
        },
        reason: r.get(5)?,
        revision: r.get(6)?,
        turns_used: r.get(7)?,
        tools_used: r.get(8)?,
        active_seconds: r.get(9)?,
        turn_limit: r.get(10)?,
        tool_limit: r.get(11)?,
        active_seconds_limit: r.get(12)?,
        updated_at: r.get(13)?,
        expires_at: r.get(14)?,
    })
}

impl SessionDB {
    pub fn assistant_settlement_targets(&self) -> Result<Vec<(String, String)>> {
        let conn = self.read_conn()?;
        let mut stmt=conn.prepare("SELECT c.execution_id,s.session_id
            FROM assistant_execution_contexts c JOIN chat_stream_runs s ON s.run_id=c.execution_id
            JOIN assistant_items i ON i.id=c.item_id
            WHERE c.settled=0 AND c.is_subtask=0 AND s.status<>'running' AND i.state IN('open','waiting')
            ORDER BY c.rowid")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn settle_assistant_execution(&self, execution: &str) -> Result<()> {
        let pending = self.assistant_pending_work(execution)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((item, _, revision)) = execution_item(&tx, execution)? else {
            return Ok(());
        };
        if validate_item(&tx, &item).is_err() {
            tx.execute(
                "UPDATE assistant_items SET state='stopped',reason='source_unavailable',
                revision=revision+1,updated_at=?2 WHERE id=?1 AND state<>'stopped'",
                params![item, chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
            drop(conn);
            emit_inbox_changed();
            return Ok(());
        }
        let pending_question: bool = tx.query_row(
            "SELECT EXISTS(
            SELECT 1 FROM assistant_item_questions iq JOIN ask_user_questions q
            ON q.request_id=iq.question_id WHERE iq.item_id=?1 AND q.status='pending')",
            [&item],
            |r| r.get(0),
        )?;
        let handed_off: bool = tx.query_row(
            "SELECT closed_for_question FROM assistant_execution_contexts
            WHERE execution_id=?1",
            [execution],
            |r| r.get(0),
        )?;
        if pending_question || handed_off {
            tx.execute(
                "UPDATE assistant_execution_contexts SET settled=1 WHERE execution_id=?1",
                [execution],
            )?;
            tx.commit()?;
            return Ok(());
        }
        let (state, message): (String, Option<i64>) = tx.query_row(
            "SELECT s.status,(SELECT m.id FROM messages m
                WHERE m.persistence_run_id=s.run_id AND m.role='assistant'
                ORDER BY m.logical_block_seq DESC LIMIT 1)
             FROM chat_stream_runs s WHERE s.run_id=?1",
            [execution],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if state == "running" {
            return Ok(());
        }
        let budget_waiting: bool = tx.query_row(
            "SELECT reason='budget_exhausted' FROM assistant_items WHERE id=?1",
            [&item],
            |r| r.get::<_, Option<bool>>(0).map(|v| v.unwrap_or(false)),
        )?;
        if pending != AssistantPendingWork::Ready {
            let reason = if pending == AssistantPendingWork::Unknown {
                "work_state_unknown"
            } else if budget_waiting {
                "budget_exhausted"
            } else {
                "background_work"
            };
            let changed = tx.execute(
                "UPDATE assistant_items SET state='waiting',reason=?2,updated_at=?3
                WHERE id=?1 AND (state<>'waiting' OR reason IS NOT ?2)",
                params![item, reason, chrono::Utc::now().to_rfc3339()],
            )?;
            if pending == AssistantPendingWork::Unknown {
                tx.execute("INSERT OR IGNORE INTO assistant_updates(id,item_id,event_key,kind,summary,summary_code,source_revision,created_at)
                    VALUES(?1,?2,?3,'blocked',?4,'work_state_unknown',?5,?6)",params![uuid::Uuid::new_v4().to_string(),item,
                    format!("unknown:{item}:{revision}"),"A background outcome is unknown. Inspect the work before repeating an action.",revision,
                    chrono::Utc::now().to_rfc3339()])?;
            }
            tx.commit()?;
            drop(conn);
            if changed > 0 {
                emit_inbox_changed()
            }
            return Ok(());
        }
        // A delivered child continuation supersedes its earlier parent's report.
        let latest: String = tx.query_row(
            "SELECT execution_id FROM assistant_execution_contexts
            WHERE item_id=?1 AND is_subtask=0 ORDER BY rowid DESC LIMIT 1",
            [&item],
            |r| r.get(0),
        )?;
        if latest != execution {
            tx.execute(
                "UPDATE assistant_execution_contexts SET settled=1 WHERE execution_id=?1",
                [execution],
            )?;
            tx.commit()?;
            return Ok(());
        }
        let report: Option<(String, String, Option<String>, i64)> = tx
            .query_row(
                "SELECT outcome,summary,next_instruction,item_revision FROM assistant_item_reports
             WHERE execution_id=?1",
                [execution],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let report = if budget_waiting {
            Some((
                "budget".to_owned(),
                "Execution allowance exhausted; your explicit continuation is required.".to_owned(),
                None,
                revision,
            ))
        } else {
            report
        };
        let (item_state,reason,kind,summary,next)=match report{
            Some((outcome,summary,next,reported_revision)) if reported_revision==revision && (state=="committed" || outcome=="budget")=>{
                match outcome.as_str(){
                    "budget"=>("waiting",Some("budget_exhausted"),"blocked",summary,None),
                    "done"=>("done",None,"result",summary,None),
                    "continue"=>("open",None,"progress",summary,next),
                    "waiting"=>("waiting",Some("external_dependency"),"waiting",summary,None),
                    _=>("waiting",Some("execution_failed"),"blocked",summary,None),
                }
            }
            _=>("waiting",Some("awaiting_verification"),"needs_review",
                "The execution ended without a verified final report. Review the work before continuing.".to_owned(),None),
        };
        let owner_followup: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_followups f LEFT JOIN chat_stream_runs s ON s.run_id=f.consumer_execution_id
            WHERE f.item_id=?1 AND (f.id LIKE 'owner-followup:%' OR f.id LIKE 'answer:%')
                AND (f.state IN('pending','queued') OR (f.state='claimed' AND COALESCE(s.status,'missing')<>'committed')))",
            [&item],
            |r| r.get(0),
        )?;
        let item_state = if item_state == "done" && owner_followup {
            "open"
        } else {
            item_state
        };
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE assistant_items SET state=?2,reason=?3,updated_at=?4
            WHERE id=?1 AND revision=?5 AND state IN('open','waiting')",
            params![item, item_state, reason, now, revision],
        )?;
        tx.execute(
            "INSERT INTO assistant_updates(id,item_id,event_key,kind,summary,result_message_id,
            source_revision,created_at,summary_code) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
            ON CONFLICT(event_key) DO NOTHING",
            params![
                uuid::Uuid::new_v4().to_string(),
                item,
                format!("settle:{execution}"),
                kind,
                summary,
                message,
                revision,
                now,
                if budget_waiting {
                    Some("budget_exhausted")
                } else if kind == "needs_review" {
                    Some("awaiting_verification")
                } else {
                    None
                }
            ],
        )?;
        let repeated:bool=tx.query_row("SELECT COUNT(*)>=2 FROM (SELECT summary,next_instruction
            FROM assistant_item_reports p JOIN assistant_execution_contexts c ON c.execution_id=p.execution_id
            WHERE c.item_id=?1 AND p.outcome='continue' ORDER BY p.rowid DESC LIMIT 2)
            WHERE summary=?2 AND next_instruction IS ?3",params![item,summary,next],|r|r.get(0))?;
        if repeated {
            tx.execute(
                "UPDATE assistant_items SET state='waiting',reason='no_progress' WHERE id=?1",
                [&item],
            )?;
        }
        if let Some(next) = next.filter(|_| !repeated) {
            tx.execute("INSERT INTO assistant_followups(id,item_id,item_revision,source_turn_id,instruction,due_at)
                VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING",
                params![format!("next:{execution}"),item,revision,execution,next,
                    chrono::Utc::now().timestamp()+2])?;
        }
        tx.execute(
            "UPDATE assistant_execution_contexts SET settled=1 WHERE execution_id=?1",
            [execution],
        )?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(())
    }

    pub fn queue_assistant_followups(&self) -> Result<usize> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let due = {
            let mut stmt = tx.prepare(
                "SELECT id,item_id,item_revision,instruction FROM assistant_followups
                WHERE state='pending' AND due_at<=?1 ORDER BY CASE WHEN id LIKE 'owner-continue:%' THEN 0 ELSE 1 END,due_at,id LIMIT 16",
            )?;
            let rows = stmt
                .query_map([chrono::Utc::now().timestamp()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut count = 0;
        for (event, item, revision, next) in due {
            let occupied=has_internal_continuation(&tx,&item)? || tx.query_row("SELECT
                EXISTS(SELECT 1 FROM assistant_item_runs ar JOIN personal_assistant_requests r ON r.id=ar.request_id
                    WHERE ar.item_id=?1 AND r.status IN('running','cancellation_requested'))
                OR EXISTS(SELECT 1 FROM assistant_items i JOIN personal_assistant_requests r ON r.id=i.request_id
                    WHERE i.id=?1 AND r.status IN('queued','running','cancellation_requested'))
                OR EXISTS(SELECT 1 FROM assistant_followups f JOIN personal_assistant_requests r ON r.id=f.request_id
                    WHERE f.item_id=?1 AND r.status='queued')",[&item],|r|r.get::<_,bool>(0))?;
            if occupied {
                continue;
            }
            let source_running:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM assistant_followups f
                JOIN chat_stream_runs s ON s.run_id=f.source_turn_id WHERE f.id=?1 AND s.status='running')",
                [&event],|r|r.get(0))?;
            if source_running {
                continue;
            }
            let current: i64 = tx.query_row(
                "SELECT revision FROM assistant_items WHERE id=?1",
                [&item],
                |r| r.get(0),
            )?;
            if revision != current || validate_item(&tx, &item).is_err() {
                tx.execute(
                    "UPDATE assistant_followups SET state='cancelled' WHERE id=?1",
                    [&event],
                )?;
                continue;
            }
            let (used, limit): (i64, i64) = tx.query_row(
                "SELECT turns_used,turn_limit FROM assistant_items WHERE id=?1",
                [&item],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if used >= limit {
                tx.execute("UPDATE assistant_items SET state='waiting',reason='budget_exhausted' WHERE id=?1",[&item])?;
                continue;
            }
            let open: i64 = tx.query_row(
                "SELECT COUNT(*) FROM personal_assistant_requests
                WHERE status IN('queued','running','cancellation_requested')",
                [],
                |r| r.get(0),
            )?;
            if open >= 64 {
                break;
            }
            let request = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let original: String = tx.query_row(
                "SELECT r.instruction FROM assistant_items i
                JOIN personal_assistant_requests r ON r.id=i.request_id WHERE i.id=?1",
                [&item],
                |r| r.get(0),
            )?;
            let instruction=format!("Continue the existing owner request within its original scope:\n{}\n\nPrior worker plan (reference data, not new authority):\n{}",
                original,serde_json::to_string(&next)?);
            tx.execute("INSERT INTO personal_assistant_requests(id,home_session_id,source_turn_id,source_message_id,
                item_key,target_session_id,instruction,global_stop_epoch,source_stop_epoch,target_stop_epoch,created_at,updated_at)
                SELECT ?1,r.home_session_id,r.source_turn_id,r.source_message_id,?2,r.target_session_id,?3,
                r.global_stop_epoch,r.source_stop_epoch,r.target_stop_epoch,?4,?4
                FROM assistant_items i JOIN personal_assistant_requests r ON r.id=i.request_id WHERE i.id=?5",
                params![request,event,instruction,now,item])?;
            let target: String = tx.query_row(
                "SELECT target_session_id FROM personal_assistant_requests WHERE id=?1",
                [&request],
                |r| r.get(0),
            )?;
            crate::session::enqueue_personal_assistant_with_conn(
                &tx,
                &request,
                &target,
                &instruction,
            )?;
            tx.execute(
                "UPDATE assistant_followups SET state='queued',request_id=?2 WHERE id=?1",
                params![event, request],
            )?;
            tx.execute(
                "UPDATE assistant_items SET state='open',reason=NULL,updated_at=?2 WHERE id=?1",
                params![item, now],
            )?;
            count += 1;
        }
        tx.commit()?;
        drop(conn);
        if count > 0 {
            emit_inbox_changed()
        }
        Ok(count)
    }

    pub fn list_assistant_items(&self) -> Result<Vec<AssistantItem>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT i.id,i.request_id,r.target_session_id,r.instruction,i.state,i.reason,
             i.revision,i.turns_used,i.tools_used,i.active_seconds,i.turn_limit,i.tool_limit,
             i.active_seconds_limit,i.updated_at,i.expires_at FROM assistant_items i
             JOIN personal_assistant_requests r ON r.id=i.request_id
             ORDER BY i.updated_at DESC,i.id LIMIT 200",
        )?;
        let rows = stmt
            .query_map([], item_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter(|i| validate_target(&conn, &i.target_session_id).is_ok())
            .collect())
    }

    pub fn list_assistant_updates(&self) -> Result<Vec<AssistantUpdate>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT u.id,u.item_id,u.kind,u.summary,u.result_message_id,u.created_at,u.viewed_at,
                    r.target_session_id,u.summary_code FROM assistant_updates u
             JOIN assistant_items i ON i.id=u.item_id
             JOIN personal_assistant_requests r ON r.id=i.request_id
             WHERE u.source_revision<=i.revision
             ORDER BY u.created_at DESC,u.id DESC LIMIT 200",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    AssistantUpdate {
                        id: r.get(0)?,
                        item_id: r.get(1)?,
                        kind: r.get(2)?,
                        summary: r.get(3)?,
                        summary_code: r.get(8)?,
                        result_message_id: r.get(4)?,
                        created_at: r.get(5)?,
                        viewed_at: r.get(6)?,
                    },
                    r.get::<_, String>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .rev()
            .filter(|(_, s)| validate_target(&conn, s).is_ok())
            .map(|(u, _)| u)
            .collect())
    }

    pub fn view_assistant_update(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let target: String=conn.query_row(
            "SELECT r.target_session_id FROM assistant_updates u JOIN assistant_items i ON i.id=u.item_id
             JOIN personal_assistant_requests r ON r.id=i.request_id WHERE u.id=?1",[id],|r|r.get(0))?;
        validate_target(&conn, &target)?;
        conn.execute(
            "UPDATE assistant_updates SET viewed_at=COALESCE(viewed_at,?2) WHERE id=?1",
            params![id, chrono::Utc::now().to_rfc3339()],
        )?;
        drop(conn);
        emit_inbox_changed();
        Ok(())
    }

    pub fn stop_assistant_item(&self, id: &str) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_items WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if !exists {
            bail!(AssistantError::Unavailable)
        }
        let changed=tx.execute("UPDATE assistant_items SET state='stopped',reason='owner_stopped',
            revision=revision+1,stop_epoch=stop_epoch+1,updated_at=?2 WHERE id=?1 AND state<>'stopped'",
            params![id,chrono::Utc::now().to_rfc3339()])?;
        tx.execute(
            "UPDATE assistant_followups SET state='cancelled' WHERE item_id=?1 AND state='pending'",
            [id],
        )?;
        tx.execute("UPDATE ask_user_questions SET status='answered',answered_at=?2 WHERE request_id IN(
            SELECT question_id FROM assistant_item_questions WHERE item_id=?1) AND status='pending'",
            params![id,chrono::Utc::now().to_rfc3339()])?;
        tx.commit()?;
        drop(conn);
        if changed > 0 {
            emit_inbox_changed()
        }
        Ok(())
    }

    /// Runs only inside the formal tool guard. IDs are derived from this live
    /// worker turn; a model cannot report another item's completion.
    pub fn report_assistant_item(
        &self,
        session: &str,
        turn: &str,
        outcome: &str,
        summary: &str,
        next_instruction: Option<&str>,
    ) -> Result<()> {
        if !matches!(outcome, "done" | "waiting" | "continue" | "failed")
            || summary.trim().is_empty()
            || summary.len() > 16_384
            || next_instruction.is_some_and(|s| s.len() > 16_384)
            || (outcome == "continue" && next_instruction.is_none_or(|s| s.trim().is_empty()))
        {
            bail!(AssistantError::InvalidInput);
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (item,revision):(String,i64)=tx.query_row(
            "SELECT i.id,i.revision FROM assistant_execution_contexts ar
             JOIN assistant_items i ON i.id=ar.item_id JOIN chat_stream_runs t ON t.run_id=ar.execution_id
             WHERE ar.execution_id=?1 AND ar.is_subtask=0 AND ar.closed_for_question=0
                AND ar.item_revision=i.revision AND t.session_id=?2 AND t.status='running'",
            params![turn,session],|r|Ok((r.get(0)?,r.get(1)?)))
            .optional()?.ok_or(AssistantError::Unavailable)?;
        validate_item(&tx, &item)?;
        tx.execute("INSERT INTO assistant_item_reports(execution_id,item_revision,outcome,summary,next_instruction,created_at)
            VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(execution_id) DO UPDATE SET
            outcome=excluded.outcome,summary=excluded.summary,next_instruction=excluded.next_instruction
            WHERE assistant_item_reports.item_revision=excluded.item_revision",
            params![turn,revision,outcome,summary,next_instruction,chrono::Utc::now().to_rfc3339()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn assistant_item_for_turn(&self, turn: &str) -> Result<Option<String>> {
        Ok(self
            .read_conn()?
            .query_row(
                "SELECT item_id FROM assistant_item_runs WHERE turn_id=?1",
                [turn],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn assistant_item_for_request(&self, request: &str) -> Result<Option<String>> {
        Ok(self
            .read_conn()?
            .query_row(
                "SELECT id FROM assistant_items WHERE request_id=?1
            UNION ALL SELECT item_id FROM assistant_followups WHERE request_id=?1 LIMIT 1",
                [request],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn request_assistant_item_followup(
        &self,
        item: &str,
        source_turn: &str,
        call: &str,
        target: &str,
        instruction: &str,
        epoch: u64,
    ) -> Result<String> {
        if instruction.trim().is_empty() || instruction.len() > 16_384 || !super::valid_key(call) {
            bail!(AssistantError::InvalidInput)
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        super::source_turn(&tx, source_turn, epoch)?;
        validate_item(&tx, item)?;
        let (revision,original_target,state):(i64,String,String)=tx.query_row("SELECT i.revision,r.target_session_id,i.state
            FROM assistant_items i JOIN personal_assistant_requests r ON r.id=i.request_id WHERE i.id=?1",
            [item],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if original_target != target || !matches!(state.as_str(), "open" | "waiting") {
            bail!(AssistantError::Unavailable)
        }
        let question:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM assistant_item_questions iq
            JOIN ask_user_questions q ON q.request_id=iq.question_id WHERE iq.item_id=?1 AND q.status='pending')",
            [item],|r|r.get(0))?;
        if question {
            bail!("Answer the existing durable question before adding follow-up work")
        }
        let event = format!("owner-followup:{source_turn}:{call}");
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT item_id,instruction FROM assistant_followups
            WHERE id=?1",
                [&event],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_item, old_instruction)) = previous {
            if old_item != item || old_instruction != instruction {
                bail!(AssistantError::Conflict)
            }
            return Ok(event);
        }
        let pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM assistant_followups WHERE item_id=?1 AND state='pending'",
            [item],
            |r| r.get(0),
        )?;
        if pending >= 8 {
            bail!(AssistantError::LimitReached)
        }
        tx.execute("INSERT INTO assistant_followups(id,item_id,item_revision,source_turn_id,instruction,due_at)
            VALUES(?1,?2,?3,?4,?5,?6)",params![event,item,revision,source_turn,instruction,chrono::Utc::now().timestamp()])?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(event)
    }

    pub(super) fn park_assistant_budget_exhausted(&self, item: &str) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_item(&tx, item)?;
        let (limit, _, _) = owner_limits_with_conn(&tx, chrono::Utc::now().timestamp())?;
        let owner = owner_turns_with_conn(&tx, chrono::Utc::now().timestamp())?;
        let (used, item_limit, revision): (i64, i64, i64) = tx.query_row(
            "SELECT turns_used,turn_limit,revision
            FROM assistant_items WHERE id=?1",
            [item],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if owner < limit && used < item_limit {
            return Ok(());
        }
        let changed = tx.execute(
            "UPDATE assistant_items SET state='waiting',reason='budget_exhausted',updated_at=?2
            WHERE id=?1 AND state IN('open','waiting') AND reason IS NOT 'budget_exhausted'",
            params![item, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.execute("INSERT OR IGNORE INTO assistant_updates(id,item_id,event_key,kind,summary,summary_code,source_revision,created_at)
            VALUES(?1,?2,?3,'blocked',?4,'budget_exhausted',?5,?6)",params![uuid::Uuid::new_v4().to_string(),item,
                format!("budget:{item}:{revision}:{item_limit}:{limit}"),
                "Execution allowance exhausted; your explicit continuation is required.",revision,chrono::Utc::now().to_rfc3339()])?;
        tx.commit()?;
        drop(conn);
        if changed > 0 {
            emit_inbox_changed()
        }
        Ok(())
    }

    pub fn assistant_execution_owner_events(&self, execution: &str) -> Result<String> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT instruction FROM assistant_followups
            WHERE consumer_execution_id=?1 AND state='claimed' ORDER BY due_at,id LIMIT 8",
        )?;
        let events = stmt
            .query_map([execution], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(serde_json::json!({"ownerFollowupData":events}).to_string())
    }

    pub fn assistant_execution_role(&self, execution: &str) -> Result<Option<bool>> {
        let conn = self.read_conn()?;
        receipts::role(&conn, execution)
    }

    pub fn assistant_execution_waiting_for_owner(&self, execution: &str) -> Result<bool> {
        Ok(self.read_conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts
            WHERE execution_id=?1 AND closed_for_question=1)",
            [execution],
            |r| r.get(0),
        )?)
    }

    /// Explicit owner action, exposed only by authenticated UI adapters.
    /// The independent grant ledger survives deletion of the item content.
    pub fn continue_assistant_item(&self, id: &str) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_item(&tx, id)?;
        let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM assistant_item_questions iq
            JOIN ask_user_questions q ON q.request_id=iq.question_id WHERE iq.item_id=?1 AND q.status='pending')",
            [id],|r|r.get(0))?;
        let waiting: bool = tx.query_row(
            "SELECT state='waiting' FROM assistant_items WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        if pending || !waiting {
            bail!(AssistantError::Conflict)
        }
        let owned_continuation = has_internal_continuation(&tx, id)?;
        let previous_reason: Option<String> = tx.query_row(
            "SELECT reason FROM assistant_items WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        let key = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "INSERT INTO assistant_budget_grants(id,item_id,issued_at,turns,tools,seconds)
            VALUES(?1,?2,?3,8,64,1800)",
            params![key, id, now],
        )?;
        tx.execute(
            "UPDATE assistant_items SET turn_limit=turn_limit+8,tool_limit=tool_limit+64,
            active_seconds_limit=active_seconds_limit+1800,state='open',reason=NULL,updated_at=?2,
            expires_at=?3 WHERE id=?1",
            params![id, chrono::Utc::now().to_rfc3339(), now + 7 * 86_400],
        )?;
        if owned_continuation {
            tx.execute(
                "UPDATE assistant_items SET state='waiting',reason=?2 WHERE id=?1",
                params![
                    id,
                    if previous_reason.as_deref() == Some("work_state_unknown") {
                        "work_state_unknown"
                    } else {
                        "background_work"
                    }
                ],
            )?;
            tx.commit()?;
            drop(conn);
            emit_inbox_changed();
            return Ok(());
        }
        tx.execute(
            "UPDATE assistant_followups SET state='cancelled' WHERE item_id=?1 AND state='pending'
                AND id NOT LIKE 'owner-followup:%' AND id NOT LIKE 'answer:%'",
            [id],
        )?;
        tx.execute("INSERT INTO assistant_followups(id,item_id,item_revision,source_turn_id,instruction,due_at)
            SELECT ?1,id,revision,COALESCE((SELECT execution_id FROM assistant_execution_contexts
                WHERE item_id=?2 ORDER BY rowid DESC LIMIT 1),request_id),?3,?4
            FROM assistant_items WHERE id=?2",params![format!("owner-continue:{key}"),id,
                "The owner explicitly continued this existing item with an additional bounded allowance. Verify unknown prior outcomes before any repeat write; preserve the original request scope.",now])?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(())
    }

    pub(crate) fn assistant_dispatch_failed_with_conn(
        conn: &Connection,
        request: &str,
    ) -> Result<()> {
        let item: Option<String> = conn
            .query_row(
                "SELECT id FROM assistant_items WHERE request_id=?1
            UNION ALL SELECT item_id FROM assistant_followups WHERE request_id=?1 LIMIT 1",
                [request],
                |r| r.get(0),
            )
            .optional()?;
        let Some(item) = item else { return Ok(()) };
        let now = chrono::Utc::now().timestamp();
        let (turns, tools, seconds) = owner_limits_with_conn(conn, now)?;
        let used = owner_turns_with_conn(conn, now)?;
        let spent:(i64,i64)=conn.query_row("SELECT COALESCE(SUM(CASE WHEN kind='tool' THEN 1 ELSE 0 END),0),
            COALESCE(SUM(COALESCE(charged_seconds,reserved_seconds)),0) FROM assistant_budget_reservations
            WHERE started_at>=?1",[now-86_400],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let exhausted: bool = conn.query_row(
            "SELECT turns_used>=turn_limit OR tools_used>=tool_limit
            OR active_seconds>=active_seconds_limit FROM assistant_items WHERE id=?1",
            [&item],
            |r| r.get(0),
        )?;
        let reason = if exhausted || used >= turns || spent.0 >= tools || spent.1 >= seconds {
            "budget_exhausted"
        } else {
            "dispatch_failed"
        };
        conn.execute("UPDATE assistant_items SET state='waiting',reason=?2,updated_at=?3 WHERE id=?1 AND state='open'",
            params![item,reason,chrono::Utc::now().to_rfc3339()])?;
        conn.execute("INSERT OR IGNORE INTO assistant_updates(id,item_id,event_key,kind,summary,summary_code,source_revision,created_at)
            SELECT ?1,id,?2,'blocked',?3,?6,revision,?4 FROM assistant_items WHERE id=?5",
            params![uuid::Uuid::new_v4().to_string(),format!("dispatch:{request}"),
                if reason=="budget_exhausted"{"Execution allowance exhausted; your explicit continuation is required."}
                else{"The work could not start. Review the item before continuing."},chrono::Utc::now().to_rfc3339(),item,reason])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
