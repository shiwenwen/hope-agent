//! Content deletion must never turn a previously owned execution into ordinary
//! work. These immutable identity witnesses contain no prompts/results or FKs.
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::AssistantError;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS assistant_execution_receipts(
        execution_id TEXT PRIMARY KEY,session_id TEXT NOT NULL,root_turn_id TEXT NOT NULL,
        item_id TEXT NOT NULL,item_revision INTEGER NOT NULL,is_subtask INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS assistant_execution_receipts_session ON assistant_execution_receipts(session_id);
    CREATE TABLE IF NOT EXISTS assistant_source_receipts(
        source_kind TEXT NOT NULL,source_id TEXT NOT NULL,parent_session_id TEXT NOT NULL,
        root_turn_id TEXT NOT NULL,item_id TEXT NOT NULL,item_revision INTEGER NOT NULL,
        parent_is_subtask INTEGER NOT NULL,child_session_id TEXT,
        PRIMARY KEY(source_kind,source_id)
    );
    CREATE INDEX IF NOT EXISTS assistant_source_receipts_child ON assistant_source_receipts(child_session_id);
    INSERT OR IGNORE INTO assistant_execution_receipts
        SELECT c.execution_id,s.session_id,c.root_turn_id,c.item_id,c.item_revision,c.is_subtask
        FROM assistant_execution_contexts c JOIN chat_stream_runs s ON s.run_id=c.execution_id;
    INSERT OR IGNORE INTO assistant_source_receipts
        SELECT source_kind,source_id,parent_session_id,root_turn_id,item_id,item_revision,parent_is_subtask,NULL
        FROM assistant_owned_sources;
    CREATE TRIGGER IF NOT EXISTS assistant_execution_identity_receipt AFTER INSERT ON assistant_execution_contexts
    BEGIN INSERT INTO assistant_execution_receipts(execution_id,session_id,root_turn_id,item_id,item_revision,is_subtask)
        VALUES(NEW.execution_id,(SELECT session_id FROM chat_stream_runs WHERE run_id=NEW.execution_id),
            NEW.root_turn_id,NEW.item_id,NEW.item_revision,NEW.is_subtask); END;
    CREATE TRIGGER IF NOT EXISTS assistant_source_identity_receipt AFTER INSERT ON assistant_owned_sources
    BEGIN INSERT INTO assistant_source_receipts(source_kind,source_id,parent_session_id,root_turn_id,item_id,item_revision,parent_is_subtask,child_session_id)
        VALUES(NEW.source_kind,NEW.source_id,NEW.parent_session_id,NEW.root_turn_id,NEW.item_id,NEW.item_revision,
            NEW.parent_is_subtask,CASE WHEN NEW.source_kind='subagent' THEN (SELECT child_session_id FROM subagent_runs WHERE run_id=NEW.source_id) ELSE NULL END); END;
    CREATE TRIGGER IF NOT EXISTS assistant_removed_item_questions BEFORE DELETE ON assistant_items
    BEGIN UPDATE ask_user_questions SET status='cancelled',answered_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
        WHERE status='pending' AND request_id IN(SELECT question_id FROM assistant_item_questions WHERE item_id=OLD.id); END;
    CREATE TRIGGER IF NOT EXISTS assistant_removed_execution_questions BEFORE DELETE ON assistant_execution_contexts
    BEGIN UPDATE ask_user_questions SET status='cancelled',answered_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
        WHERE status='pending' AND request_id IN(SELECT question_id FROM assistant_item_questions WHERE execution_id=OLD.execution_id); END;")?;
    if super::super::table_exists(conn, "subagent_runs")? {
        conn.execute("UPDATE assistant_source_receipts SET child_session_id=(SELECT child_session_id FROM subagent_runs s WHERE s.run_id=assistant_source_receipts.source_id)
            WHERE source_kind='subagent' AND child_session_id IS NULL",[])?;
    }
    Ok(())
}

pub(super) fn deny_removed_execution(conn: &Connection, execution: &str) -> Result<()> {
    let owned: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_execution_receipts WHERE execution_id=?1)",
        [execution],
        |r| r.get(0),
    )?;
    if owned {
        bail!(AssistantError::Unavailable)
    }
    Ok(())
}

pub(super) fn deny_removed_source(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    let owned:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_source_receipts WHERE source_kind=?1 AND source_id=?2)",params![kind,id],|r|r.get(0))?;
    if owned {
        bail!(AssistantError::Unavailable)
    }
    Ok(())
}

pub(super) fn role(conn: &Connection, execution: &str) -> Result<Option<bool>> {
    Ok(conn
        .query_row(
            "SELECT is_subtask FROM assistant_execution_receipts WHERE execution_id=?1",
            [execution],
            |r| r.get(0),
        )
        .optional()?)
}
