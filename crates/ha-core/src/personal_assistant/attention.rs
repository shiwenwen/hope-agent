//! Durable, body-free interruption identities. Content is projected only at
//! the final channel claim after current authority and full-input checks.
use crate::session::SessionDB;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

mod claims;
pub(crate) use claims::read_scope_current;
pub use claims::AssistantAttentionImSend;
mod disclosure;
pub(crate) use disclosure::source_disclosure_status;
pub(crate) mod quota;
pub(crate) mod replies;
pub(crate) mod signals;
pub use signals::NotificationOwnership;

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS assistant_attention_private_origins(session_id TEXT PRIMARY KEY);
    WITH RECURSIVE origins(id) AS(SELECT id FROM sessions WHERE kind IN('personal_assistant','personal_assistant_im','personal_assistant_research')
        UNION SELECT s.id FROM sessions s JOIN origins p ON s.parent_session_id=p.id OR s.forked_from_session_id=p.id)
    INSERT OR IGNORE INTO assistant_attention_private_origins SELECT id FROM origins;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_private_origin_insert AFTER INSERT ON sessions
        WHEN NEW.kind IN('personal_assistant','personal_assistant_im','personal_assistant_research') OR EXISTS(SELECT 1 FROM assistant_attention_private_origins WHERE session_id=NEW.parent_session_id OR session_id=NEW.forked_from_session_id)
        BEGIN INSERT OR IGNORE INTO assistant_attention_private_origins(session_id) VALUES(NEW.id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_private_origin_update AFTER UPDATE OF kind,parent_session_id,forked_from_session_id ON sessions
        WHEN NEW.kind IN('personal_assistant','personal_assistant_im','personal_assistant_research') OR EXISTS(SELECT 1 FROM assistant_attention_private_origins WHERE session_id=NEW.parent_session_id OR session_id=NEW.forked_from_session_id)
        BEGIN INSERT OR IGNORE INTO assistant_attention_private_origins(session_id) VALUES(NEW.id); END;
    CREATE TABLE IF NOT EXISTS assistant_attention_private_executions(execution_id TEXT PRIMARY KEY);
    INSERT OR IGNORE INTO assistant_attention_private_executions SELECT run_id FROM chat_stream_runs WHERE session_id IN(SELECT session_id FROM assistant_attention_private_origins);
    INSERT OR IGNORE INTO assistant_attention_private_executions SELECT id FROM chat_turns WHERE session_id IN(SELECT session_id FROM assistant_attention_private_origins);
    CREATE TRIGGER IF NOT EXISTS assistant_attention_private_stream AFTER INSERT ON chat_stream_runs WHEN EXISTS(SELECT 1 FROM assistant_attention_private_origins WHERE session_id=NEW.session_id)
        BEGIN INSERT OR IGNORE INTO assistant_attention_private_executions(execution_id) VALUES(NEW.run_id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_private_turn AFTER INSERT ON chat_turns WHEN EXISTS(SELECT 1 FROM assistant_attention_private_origins WHERE session_id=NEW.session_id)
        BEGIN INSERT OR IGNORE INTO assistant_attention_private_executions(execution_id) VALUES(NEW.id); END;
    CREATE TABLE IF NOT EXISTS assistant_attention_owned_requests(request_id TEXT PRIMARY KEY);
    INSERT OR IGNORE INTO assistant_attention_owned_requests SELECT request_id FROM assistant_items;
    INSERT OR IGNORE INTO assistant_attention_owned_requests SELECT request_id FROM assistant_followups WHERE request_id IS NOT NULL;
    INSERT OR IGNORE INTO assistant_attention_owned_requests SELECT request_id FROM assistant_item_runs;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_followup_identity AFTER UPDATE OF request_id ON assistant_followups WHEN NEW.request_id IS NOT NULL
        BEGIN INSERT OR IGNORE INTO assistant_attention_owned_requests(request_id) VALUES(NEW.request_id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_followup_insert_identity AFTER INSERT ON assistant_followups WHEN NEW.request_id IS NOT NULL
        BEGIN INSERT OR IGNORE INTO assistant_attention_owned_requests(request_id) VALUES(NEW.request_id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_item_run_identity AFTER INSERT ON assistant_item_runs
        BEGIN INSERT OR IGNORE INTO assistant_attention_owned_requests(request_id) VALUES(NEW.request_id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_request_identity AFTER INSERT ON assistant_items BEGIN INSERT OR IGNORE INTO assistant_attention_owned_requests(request_id) VALUES(NEW.request_id); END;
    CREATE TABLE IF NOT EXISTS assistant_attention_boundary(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),eligible_after INTEGER NOT NULL,desktop_eligible_after INTEGER NOT NULL DEFAULT 0,im_eligible_after INTEGER NOT NULL DEFAULT 0
    );
    INSERT OR IGNORE INTO assistant_attention_boundary(singleton,eligible_after,desktop_eligible_after)
        VALUES(1,CAST(strftime('%s','now') AS INTEGER),CAST(strftime('%s','now') AS INTEGER));
    CREATE TABLE IF NOT EXISTS assistant_attention_presence(
        client_id TEXT PRIMARY KEY,expires_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS assistant_attention_candidates(
        logical_key TEXT PRIMARY KEY,kind TEXT NOT NULL,subject_id TEXT NOT NULL,subject_revision INTEGER NOT NULL,
        source_run_id TEXT,source_set_json TEXT,home_session_id TEXT NOT NULL,global_stop_epoch INTEGER NOT NULL,
        home_stop_epoch INTEGER NOT NULL,created_at INTEGER NOT NULL,ready_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,
        im_reply_id TEXT
    );
    CREATE TABLE IF NOT EXISTS assistant_attention_deliveries(
        id TEXT PRIMARY KEY,route TEXT NOT NULL,claim_key TEXT NOT NULL UNIQUE,client_id TEXT,
        state TEXT NOT NULL,created_at INTEGER NOT NULL,submitted_at INTEGER NOT NULL,settled_at INTEGER,
        connection_id TEXT,generation INTEGER,chat_id INTEGER,message_id INTEGER,
        expires_at INTEGER NOT NULL,instance_id TEXT NOT NULL,policy_revision INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS assistant_attention_delivery_items(
        delivery_id TEXT NOT NULL,ordinal INTEGER NOT NULL,logical_key TEXT NOT NULL UNIQUE,
        kind TEXT NOT NULL,subject_id TEXT NOT NULL,subject_revision INTEGER NOT NULL,
        source_run_id TEXT,source_set_json TEXT,disclosure_set_json TEXT,im_grant_revision INTEGER,home_session_id TEXT NOT NULL,global_stop_epoch INTEGER NOT NULL,
        home_stop_epoch INTEGER NOT NULL,expires_at INTEGER NOT NULL,candidate_created_at INTEGER NOT NULL,im_reply_id TEXT,
        PRIMARY KEY(delivery_id,ordinal)
    );
    CREATE TABLE IF NOT EXISTS assistant_attention_payloads(
        delivery_id TEXT NOT NULL,ordinal INTEGER NOT NULL,body TEXT NOT NULL,expires_at INTEGER NOT NULL,
        PRIMARY KEY(delivery_id,ordinal)
    );
    CREATE TRIGGER IF NOT EXISTS assistant_attention_source_payload_changed AFTER UPDATE OF state,revision,version,error ON assistant_sources
      WHEN NEW.state<>'active' OR NEW.revision<>OLD.revision OR NEW.version IS NOT OLD.version OR NEW.error<>0
      BEGIN DELETE FROM assistant_attention_payloads WHERE delivery_id IN(
        SELECT item.delivery_id FROM assistant_attention_delivery_items item,json_each(item.source_set_json) source WHERE json_extract(source.value,'$.sourceId')=NEW.id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_source_payload_deleted AFTER DELETE ON assistant_sources
      BEGIN DELETE FROM assistant_attention_payloads WHERE delivery_id IN(
        SELECT item.delivery_id FROM assistant_attention_delivery_items item,json_each(item.source_set_json) source WHERE json_extract(source.value,'$.sourceId')=OLD.id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_attention_home_payload_deleted AFTER DELETE ON sessions
      BEGIN DELETE FROM assistant_attention_payloads WHERE delivery_id IN(
        SELECT delivery_id FROM assistant_attention_delivery_items WHERE home_session_id=OLD.id OR source_run_id IN(
            SELECT id FROM assistant_research_runs WHERE session_id=OLD.id)); END;
    CREATE TABLE IF NOT EXISTS assistant_source_im_disclosures(
        source_id TEXT NOT NULL,source_revision INTEGER NOT NULL,connection_id TEXT NOT NULL,
        generation INTEGER NOT NULL,revision INTEGER NOT NULL,enabled INTEGER NOT NULL,expires_at INTEGER NOT NULL,
        PRIMARY KEY(source_id,source_revision,connection_id,generation)
    );
    CREATE TABLE IF NOT EXISTS assistant_attention_inbounds(
        id TEXT PRIMARY KEY,connection_id TEXT NOT NULL,generation INTEGER NOT NULL,cursor_epoch INTEGER NOT NULL,
        update_id INTEGER NOT NULL,user_id INTEGER NOT NULL,chat_id INTEGER NOT NULL,
        delivery_id TEXT NOT NULL,ordinal INTEGER NOT NULL,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,
        home_session_id TEXT NOT NULL,global_stop_epoch INTEGER NOT NULL,home_stop_epoch INTEGER NOT NULL,
        state TEXT NOT NULL,query_id TEXT,question_id TEXT,
        UNIQUE(connection_id,generation,cursor_epoch,update_id)
    );
    CREATE TABLE IF NOT EXISTS assistant_attention_handoffs(
        id TEXT PRIMARY KEY,execution_id TEXT NOT NULL,item_id TEXT NOT NULL,item_revision INTEGER NOT NULL,
        active INTEGER NOT NULL,instance_id TEXT NOT NULL,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL
    );")?;
    let has_im_boundary = conn
        .prepare("PRAGMA table_info(assistant_attention_boundary)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "im_eligible_after");
    if !has_im_boundary {
        conn.execute("ALTER TABLE assistant_attention_boundary ADD COLUMN im_eligible_after INTEGER NOT NULL DEFAULT 0",[])?;
    }
    let has_im_reply = conn
        .prepare("PRAGMA table_info(assistant_research_queries)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "im_reply_id");
    if !has_im_reply {
        conn.execute(
            "ALTER TABLE assistant_research_queries ADD COLUMN im_reply_id TEXT",
            [],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionReceipt {
    pub id: String,
    pub route: String,
    pub state: String,
    pub submitted_at: i64,
    pub item_count: usize,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAttentionPairing {
    pub connection_id: String,
    pub generation: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAttentionStatus {
    pub pairing: Option<AssistantAttentionPairing>,
    pub paired: bool,
    pub deferred_reason: Option<String>,
    pub next_eligible_at: Option<i64>,
    pub pending_count: usize,
    pub receipts: Vec<AttentionReceipt>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDesktopNotice {
    pub id: String,
    pub title: String,
    pub body: String,
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionOutcome {
    Accepted,
    Unknown,
    NotSubmitted,
}

#[derive(Debug, Clone)]
struct Candidate {
    key: String,
    kind: String,
    subject: String,
    revision: i64,
    source_run: Option<String>,
    source_set: Option<String>,
    home: String,
    global_epoch: i64,
    home_epoch: i64,
    created: i64,
    ready: i64,
    expires: i64,
    im_reply: Option<String>,
}
const COLUMNS:&str="logical_key,kind,subject_id,subject_revision,source_run_id,source_set_json,home_session_id,global_stop_epoch,home_stop_epoch,created_at,ready_at,expires_at,im_reply_id";
fn row_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        key: row.get(0)?,
        kind: row.get(1)?,
        subject: row.get(2)?,
        revision: row.get(3)?,
        source_run: row.get(4)?,
        source_set: row.get(5)?,
        home: row.get(6)?,
        global_epoch: row.get(7)?,
        home_epoch: row.get(8)?,
        created: row.get(9)?,
        ready: row.get(10)?,
        expires: row.get(11)?,
        im_reply: row.get(12)?,
    })
}
fn boundary_current(conn: &Connection, candidate: &Candidate, now: i64) -> Result<bool> {
    if candidate.expires <= now {
        return Ok(false);
    }
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=?1 AND s.kind='personal_assistant' AND s.incognito=0
        AND ?2=(SELECT epoch FROM runtime_control_epochs WHERE key='global_stop')
        AND ?3=(SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=s.id)
        AND NOT EXISTS(SELECT 1 FROM session_autonomy_pauses WHERE session_id=s.id AND resumed_at IS NULL))",
        params![candidate.home,candidate.global_epoch,candidate.home_epoch],|row|row.get(0))?)
}
fn validate_client(value: &str) -> Result<()> {
    if uuid::Uuid::parse_str(value).is_err() {
        bail!("A current owner client identity is required");
    }
    Ok(())
}
fn changed() {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "personal_assistant:attention_changed",
            serde_json::json!({}),
        );
    }
}

impl SessionDB {
    pub fn update_assistant_presence(&self, client_id: &str, active: bool, now: i64) -> Result<()> {
        validate_client(client_id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant presence unavailable"))?;
        conn.execute(
            "DELETE FROM assistant_attention_presence WHERE expires_at<=?1",
            [now],
        )?;
        if active {
            conn.execute("INSERT INTO assistant_attention_presence(client_id,expires_at) VALUES(?1,?2) ON CONFLICT(client_id) DO UPDATE SET expires_at=excluded.expires_at",params![client_id,now+30])?;
        } else {
            conn.execute(
                "DELETE FROM assistant_attention_presence WHERE client_id=?1",
                [client_id],
            )?;
        }
        Ok(())
    }
    pub fn reconcile_assistant_attention(&self, now: i64) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant attention unavailable"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM assistant_attention_presence WHERE expires_at<=?1",
            [now],
        )?;
        tx.execute("UPDATE assistant_attention_deliveries SET state='unknown',settled_at=?1 WHERE state='submitting' AND submitted_at<=?2",params![now,now-60])?;
        claims::collect_candidates(&tx, now)?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_assistant_desktop_notice(
        &self,
        id: &str,
        client: &str,
        claim_key: &str,
        outcome: AttentionOutcome,
        now: i64,
    ) -> Result<()> {
        validate_client(client)?;
        validate_client(claim_key)?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant receipt unavailable"))?;
        conn.execute("UPDATE assistant_attention_deliveries SET state=?4,settled_at=?5 WHERE id=?1 AND client_id=?2 AND claim_key=?3 AND route='desktop' AND state IN('submitting','unknown')",
            params![id,client,claim_key,match outcome {AttentionOutcome::Accepted=>"accepted",AttentionOutcome::Unknown=>"unknown",AttentionOutcome::NotSubmitted=>"cancelled"},now])?;
        drop(conn);
        changed();
        Ok(())
    }
}

pub(crate) fn note_policy_boundary(
    conn: &Connection,
    settings: &super::policy::AssistantPolicySettings,
    now: i64,
) -> Result<()> {
    let previous = super::policy::current(conn)?;
    let reset = previous.as_ref().is_none_or(|old| {
        old.settings.notice_route != settings.notice_route || old.settings.paused
    }) || settings.paused;
    let desktop = previous
        .as_ref()
        .is_none_or(|old| !old.settings.desktop_to_im_notices && settings.desktop_to_im_notices);
    conn.execute("INSERT OR IGNORE INTO assistant_attention_boundary(singleton,eligible_after,desktop_eligible_after) VALUES(1,?1,?1)",[now])?;
    if reset {
        conn.execute("UPDATE assistant_attention_boundary SET eligible_after=MAX(eligible_after,?1) WHERE singleton=1",[now])?;
    }
    if desktop {
        conn.execute("UPDATE assistant_attention_boundary SET desktop_eligible_after=MAX(desktop_eligible_after,?1) WHERE singleton=1",[now])?;
    }
    if settings.paused {
        replies::invalidate_im_queries(conn, None, now)?;
    }
    Ok(())
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests;

pub(crate) fn note_im_notice_resume(conn: &Connection, now: i64) -> Result<()> {
    let changed=conn.execute("UPDATE assistant_attention_boundary SET im_eligible_after=MAX(im_eligible_after,?1) WHERE singleton=1",[now])?;
    if changed != 1 {
        bail!("Assistant notification boundary unavailable");
    }
    Ok(())
}
