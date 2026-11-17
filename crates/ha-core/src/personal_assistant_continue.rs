//! Continue authority is captured with a real owner home turn, never minted
//! by a later status lookup or a model-provided timestamp.
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::session::SessionDB;

#[derive(Clone)]
pub(crate) struct AssistantContinueProof {
    pub source_session_id: String,
    pub source_turn_id: String,
    pub target_session_id: String,
    pub pause_id: String,
    pub admission: (i64, i64, i64),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssistantPauseReceipt {
    pub session_id: String,
    pub pause_id: String,
}

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS personal_assistant_continue_receipts (
            source_turn_id TEXT NOT NULL REFERENCES chat_turns(id) ON DELETE CASCADE,
            target_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            pause_id TEXT NOT NULL REFERENCES session_autonomy_pauses(id) ON DELETE CASCADE,
            source_stop_epoch INTEGER NOT NULL,
            target_stop_epoch INTEGER NOT NULL,
            global_stop_epoch INTEGER NOT NULL,
            admitted_source_stop_epoch INTEGER NOT NULL,
            admitted_global_receipt_count INTEGER NOT NULL,
            PRIMARY KEY(source_turn_id, target_session_id)
         );",
    )?;
    Ok(())
}

/// Called only inside the formal interactive admission transaction after its
/// turn/message/stream rows have been inserted. A repeated admission never
/// reaches this point, including when the original snapshot was empty.
pub(crate) fn capture_with_conn(
    conn: &Connection,
    turn_id: &str,
    registration: &crate::session::StreamRunRegistration,
) -> Result<()> {
    let home: Option<String> = conn
        .query_row(
            "SELECT s.id FROM chat_turns t JOIN sessions s ON s.id=t.session_id
         JOIN messages m ON m.id=t.user_message_id AND m.session_id=s.id
         WHERE t.id=?1 AND t.status='running' AND t.source IN ('desktop','http')
           AND s.kind='personal_assistant' AND s.incognito=0 AND s.archived_at IS NULL
           AND m.role='user' AND COALESCE(m.source,'desktop') IN ('desktop','http')",
            [turn_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(home) = home else {
        return Ok(());
    };
    // Evaluation sources require a durable evaluation proof before acquiring
    // any autonomous resume authority.
    if crate::eval_context::context_for_session(&home).is_some() {
        return Ok(());
    }
    let candidates = {
        let mut stmt = conn.prepare(
            "SELECT session_id,id FROM session_autonomy_pauses WHERE resumed_at IS NULL",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (target, pause_id) in candidates {
        match crate::personal_assistant::validate_target(conn, &target) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<crate::personal_assistant::AssistantError>()
                    .is_some() =>
            {
                continue
            }
            Err(error) => return Err(error),
        }
        conn.execute(
            "INSERT INTO personal_assistant_continue_receipts
             (source_turn_id,target_session_id,pause_id,source_stop_epoch,target_stop_epoch,global_stop_epoch,admitted_source_stop_epoch,admitted_global_receipt_count)
             VALUES (?1,?2,?3,
               (SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=?4),
               (SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=?2),
               ?5,?6,?7)",
            params![turn_id,target,pause_id,home,i64::try_from(registration.admitted_global_stop_epoch)?,i64::try_from(registration.admitted_stop_epoch)?,i64::try_from(registration.admitted_global_stop_receipt_count)?],
        )?;
    }
    Ok(())
}

pub(crate) fn validate_with_conn(conn: &Connection, proof: &AssistantContinueProof) -> Result<()> {
    let valid: Option<i64> = conn.query_row(
        "SELECT r.global_stop_epoch FROM personal_assistant_continue_receipts r
         JOIN chat_turns t ON t.id=r.source_turn_id
         JOIN sessions s ON s.id=t.session_id
         JOIN session_autonomy_pauses p ON p.id=r.pause_id AND p.session_id=r.target_session_id
         WHERE r.source_turn_id=?1 AND r.target_session_id=?2 AND r.pause_id=?3 AND s.id=?4
           AND s.archived_at IS NULL
           AND r.admitted_source_stop_epoch=?5 AND r.global_stop_epoch=?6 AND r.admitted_global_receipt_count=?7
           AND r.source_stop_epoch=(SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=s.id)
           AND r.target_stop_epoch=(SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=r.target_session_id)
           AND NOT EXISTS(SELECT 1 FROM session_autonomy_pauses newer
               WHERE newer.session_id=r.target_session_id AND newer.resumed_at IS NULL AND newer.id<>r.pause_id)",
        params![proof.source_turn_id,proof.target_session_id,proof.pause_id,proof.source_session_id,proof.admission.0,proof.admission.1,proof.admission.2],
        |row| row.get(0),
    ).optional()?;
    let Some(epoch) = valid else {
        bail!("Continue receipt is unavailable or superseded; request a new owner turn");
    };
    crate::personal_assistant::source_turn(conn, &proof.source_turn_id, u64::try_from(epoch)?)?;
    crate::personal_assistant::validate_target(conn, &proof.target_session_id)?;
    Ok(())
}

impl SessionDB {
    pub(crate) fn assistant_continue_proof(
        &self,
        source: &str,
        turn: &str,
        target: &str,
        pause: &str,
        admission: (u64, u64, u64),
    ) -> Result<AssistantContinueProof> {
        let proof = AssistantContinueProof {
            source_session_id: source.into(),
            source_turn_id: turn.into(),
            target_session_id: target.into(),
            pause_id: pause.into(),
            admission: (
                i64::try_from(admission.0)?,
                i64::try_from(admission.1)?,
                i64::try_from(admission.2)?,
            ),
        };
        validate_with_conn(&*self.read_conn()?, &proof)?;
        Ok(proof)
    }

    pub(crate) fn assistant_pause_receipt(
        &self,
        source: &str,
        turn: &str,
        target: &str,
        admission: (u64, u64, u64),
    ) -> Result<Option<AssistantPauseReceipt>> {
        let conn = self.read_conn()?;
        let pause: Option<String> = conn.query_row(
            "SELECT r.pause_id FROM personal_assistant_continue_receipts r JOIN session_autonomy_pauses p ON p.id=r.pause_id
             WHERE r.source_turn_id=?1 AND r.target_session_id=?2 AND p.resumed_at IS NULL",
            params![turn,target], |row| row.get(0),
        ).optional()?;
        let Some(pause_id) = pause else {
            return Ok(None);
        };
        let proof = AssistantContinueProof {
            source_session_id: source.into(),
            source_turn_id: turn.into(),
            target_session_id: target.into(),
            pause_id: pause_id.clone(),
            admission: (
                i64::try_from(admission.0)?,
                i64::try_from(admission.1)?,
                i64::try_from(admission.2)?,
            ),
        };
        validate_with_conn(&conn, &proof)?;
        Ok(Some(AssistantPauseReceipt {
            session_id: target.into(),
            pause_id,
        }))
    }
}
