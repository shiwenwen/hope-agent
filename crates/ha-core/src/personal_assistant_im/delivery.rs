use super::store::{
    check_lease, command_source_current, eligible_session, grant, lease, load, read_command,
    session_stop_epoch,
};
use super::*;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::session::SessionDB;

fn scope_is_current(
    conn: &rusqlite::Connection,
    connection_id: &str,
    request_id: &str,
    scope: &str,
    now: i64,
) -> Result<bool> {
    let Some(command) = read_command(conn, request_id)? else {
        return Ok(false);
    };
    if command.connection_id != connection_id || !command_source_current(conn, &command)? {
        return Ok(false);
    }
    let targets: Vec<(String, i64)> = serde_json::from_str(scope)?;
    for (session_id, revision) in targets {
        if !eligible_session(conn, &session_id)?
            || command.target_stop_epochs.get(&session_id).copied()
                != Some(u64::try_from(session_stop_epoch(conn, &session_id)?)?)
            || !grant(conn, connection_id, &session_id)?.is_some_and(|permission| {
                permission.revision == revision
                    && permission.expires_at > now
                    && permission.query_status
            })
        {
            return Ok(false);
        }
    }
    Ok(true)
}

// Authority loss is expected; database/corruption errors must remain retryable
// errors instead of permanently suppressing a legitimate notification.
fn authorization_valid(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.downcast_ref::<AssistantImError>().is_some() => Ok(false),
        Err(error) => Err(error),
    }
}

fn parse_state(state: &str) -> rusqlite::Result<DeliveryState> {
    match state {
        "pending" => Ok(DeliveryState::Pending),
        "submitting" => Ok(DeliveryState::Submitting),
        "accepted" => Ok(DeliveryState::Accepted),
        "unknown" => Ok(DeliveryState::Unknown),
        "cancelled" => Ok(DeliveryState::Cancelled),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

impl SessionDB {
    /// Project a proven terminal target into one status-only reply. The source
    /// command and its frozen permissions remain attached to the outbox item.
    pub fn enqueue_assistant_im_request_completion(
        &self,
        request_id: &str,
        now: i64,
    ) -> Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        type CompletionRow = (String, String, String, Option<String>, String, i64);
        let result: Option<CompletionRow> = tx
            .query_row(
                "SELECT r.source_turn_id,r.target_session_id,t.status,s.title,c.connection_id,c.generation
             FROM personal_assistant_requests r JOIN chat_turns t ON t.id = r.turn_id AND t.session_id = r.target_session_id
             JOIN sessions s ON s.id = r.target_session_id
             JOIN assistant_im_commands c ON c.request_id = r.source_turn_id
             WHERE r.id = ?1 AND t.status IN ('completed','failed','interrupted','cancelled')",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,row.get(4)?,row.get(5)?)),
            )
            .optional()?;
        let Some((command_id, session_id, status, title, connection_id, generation)) = result
        else {
            return Ok(false);
        };
        let state = load(&tx)?;
        let connection_allowed = state.as_ref().is_some_and(|state| {
            state.view.connection_id == connection_id
                && state.view.generation == generation
                && state.view.state == PairingState::Active
                && !state.view.notifications_paused
        });
        let permission = grant(&tx, &connection_id, &session_id)?;
        let allowed = connection_allowed
            && permission.as_ref().is_some_and(|permission| {
                permission.completion_notice
                    && permission.query_status
                    && permission.expires_at > now
            })
            && authorization_valid(super::revalidate_assistant_im_dispatch(
                &tx,
                &command_id,
                &session_id,
                now,
            ))?;
        let text = if allowed {
            let status = match status.as_str() {
                "completed" => "已完成",
                "interrupted" | "cancelled" => "已停止",
                _ => "执行失败",
            };
            let label = title
                .filter(|title| !title.is_empty())
                .map(|title| format!("「{}」", crate::truncate_utf8(&title, 160)))
                .unwrap_or_default();
            format!("工作{label}{status}。请在 Hope 查看详情。")
        } else {
            String::new()
        };
        // Suppressed terminal notifications are final too: retain a body-free
        // tombstone so unpausing/re-pairing cannot replay history, and old
        // ineligible rows cannot starve the bounded completion scanner.
        let id = uuid::Uuid::new_v4().to_string();
        let inserted = tx.execute(
            "INSERT INTO assistant_im_deliveries
             (id,connection_id,generation,request_id,session_id,grant_revision,origin_command_id,
              completion_notice,text,state,created_at,expires_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?10,?11)
             ON CONFLICT(connection_id,generation,request_id) DO NOTHING",
            params![
                id,
                connection_id,
                generation,
                format!("{request_id}:complete"),
                session_id,
                permission.as_ref().map_or(0, |p| p.revision),
                command_id,
                text,
                if allowed { "pending" } else { "cancelled" },
                now,
                permission
                    .as_ref()
                    .map_or(now, |p| p.expires_at.min(now + 86_400))
            ],
        )?;
        tx.commit()?;
        drop(conn);
        if inserted > 0 {
            changed();
        }
        Ok(inserted > 0 && allowed)
    }
    /// Retain the exact model-view scope and require all of its grants to
    /// remain valid when a multi-target dialog reply leaves the process.
    pub fn enqueue_assistant_im_dialog_reply(
        &self,
        request_id: &str,
        text: &str,
        now: i64,
    ) -> Result<String> {
        if text.is_empty() || text.chars().count() > MAX_REPLY_CHARS {
            return Err(AssistantImError::InvalidInput.into());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        let scope_key: String = tx
            .query_row(
                "SELECT v.scope_key FROM assistant_im_command_views cv
             JOIN assistant_im_views v ON v.session_id = cv.view_session_id
             JOIN assistant_im_commands c ON c.request_id = cv.request_id
             WHERE c.request_id = ?1 AND c.connection_id = ?2 AND c.generation = ?3
               AND c.state IN ('pending','running','reply_pending')",
                params![request_id, state.view.connection_id, state.view.generation],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(AssistantImError::PermissionDenied)?;
        if state.view.state != PairingState::Active
            || !scope_is_current(&tx, &state.view.connection_id, request_id, &scope_key, now)?
        {
            return Err(AssistantImError::PermissionDenied.into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO assistant_im_deliveries
             (id,connection_id,generation,request_id,session_id,grant_revision,grant_revisions,
              completion_notice,text,state,created_at,expires_at)
             VALUES (?1,?2,?3,?4,NULL,0,?5,0,?6,'pending',?7,?8)
             ON CONFLICT(connection_id,generation,request_id) DO NOTHING",
            params![
                id,
                state.view.connection_id,
                state.view.generation,
                request_id,
                scope_key,
                text,
                now,
                now + 86_400
            ],
        )?;
        let saved_id = tx.query_row(
            "SELECT id FROM assistant_im_deliveries WHERE connection_id = ?1 AND generation = ?2 AND request_id = ?3",
            params![state.view.connection_id,state.view.generation,request_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(saved_id)
    }

    /// Store a bounded projection from the trusted command service. Text is
    /// never assembled by the platform worker from full session histories.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_assistant_im_reply(
        &self,
        expected: &PollLease,
        request_id: &str,
        session_id: &str,
        grant_revision: i64,
        completion_notice: bool,
        text: &str,
        now: i64,
    ) -> Result<String> {
        if request_id.is_empty()
            || request_id.len() > 128
            || text.is_empty()
            || text.chars().count() > MAX_REPLY_CHARS
        {
            return Err(AssistantImError::InvalidInput.into());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        check_lease(&state, expected)?;
        let permission = grant(&tx, &expected.connection_id, session_id)?
            .ok_or(AssistantImError::PermissionDenied)?;
        if state.view.state != PairingState::Active
            || permission.revision != grant_revision
            || permission.expires_at <= now
            || !permission.query_status
            || !eligible_session(&tx, session_id)?
            || (completion_notice
                && (!permission.completion_notice || state.view.notifications_paused))
        {
            return Err(AssistantImError::PermissionDenied.into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO assistant_im_deliveries
             (id, connection_id, generation, request_id, session_id, grant_revision,
              completion_notice, text, state, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10)
             ON CONFLICT(connection_id, generation, request_id) DO NOTHING",
            params![
                id,
                expected.connection_id,
                expected.generation,
                request_id,
                session_id,
                grant_revision,
                completion_notice,
                text,
                now,
                permission.expires_at.min(now + 86_400)
            ],
        )?;
        let saved_id: String = tx.query_row(
            "SELECT id FROM assistant_im_deliveries
             WHERE connection_id = ?1 AND generation = ?2 AND request_id = ?3",
            params![expected.connection_id, expected.generation, request_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(saved_id)
    }

    /// This commit is the non-replay boundary. The Primary calls it only when
    /// its destination queue is ready to issue a single HTTP mutation.
    pub fn prepare_assistant_im_delivery(&self, now: i64) -> Result<Option<PreparedDelivery>> {
        self.prepare_assistant_im_delivery_with_policy(now, &notification_policy())
    }

    pub(super) fn prepare_assistant_im_delivery_with_policy(
        &self,
        now: i64,
        policy: &NotificationPolicy,
    ) -> Result<Option<PreparedDelivery>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(state) = load(&tx)? else {
            return Ok(None);
        };
        if state.view.state != PairingState::Active {
            return Ok(None);
        }
        let expired = tx.execute(
            "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
             WHERE state = 'pending' AND expires_at <= ?1",
            [now],
        )?;
        let completion_ready =
            super::notification_policy::deferred_reason(&tx, policy.timezone(), now)?.is_none();
        type PendingRow = (
            String,
            Option<String>,
            i64,
            bool,
            String,
            i64,
            Option<String>,
        );
        let candidate: Option<PendingRow> = tx
            .query_row(
                "SELECT id, session_id, grant_revision, completion_notice, text, expires_at, grant_revisions
             FROM assistant_im_deliveries
             WHERE connection_id = ?1 AND generation = ?2 AND state = 'pending'
               AND (completion_notice = 0 OR ?3)
             ORDER BY completion_notice, created_at, rowid LIMIT 1",
                params![state.view.connection_id, state.view.generation, completion_ready],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, session_id, revision, completion_notice, text, expires_at, scope_key)) =
            candidate
        else {
            tx.commit()?;
            drop(conn);
            if expired > 0 {
                changed();
            }
            return Ok(None);
        };
        let scope_allowed = if let Some(scope_key) = scope_key {
            let request_id: String = tx.query_row(
                "SELECT request_id FROM assistant_im_deliveries WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )?;
            scope_is_current(&tx, &state.view.connection_id, &request_id, &scope_key, now)?
        } else if let Some(session_id) = session_id {
            grant(&tx, &state.view.connection_id, &session_id)?.is_some_and(|permission| {
                permission.revision == revision
                    && permission.expires_at > now
                    && permission.query_status
                    && (!completion_notice || permission.completion_notice)
            }) && eligible_session(&tx, &session_id)?
        } else {
            false
        };
        let allowed = scope_allowed
            && expires_at > now
            && (!completion_notice || !state.view.notifications_paused);
        let origin: Option<String> = tx.query_row(
            "SELECT origin_command_id FROM assistant_im_deliveries WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )?;
        let origin_allowed = match origin {
            Some(command_id) => {
                let target: String = tx.query_row(
                    "SELECT session_id FROM assistant_im_deliveries WHERE id = ?1",
                    [&id],
                    |row| row.get(0),
                )?;
                authorization_valid(super::revalidate_assistant_im_dispatch(
                    &tx,
                    &command_id,
                    &target,
                    now,
                ))?
            }
            None => true,
        };
        if !allowed || !origin_allowed {
            tx.execute(
                "UPDATE assistant_im_deliveries SET state = 'cancelled', text = '' WHERE id = ?1",
                [&id],
            )?;
            tx.commit()?;
            drop(conn);
            changed();
            return Ok(None);
        }
        let chat_id = state.view.chat_id.ok_or(AssistantImError::Unavailable)?;
        if completion_notice {
            super::notification_policy::reserve_attempt(&tx, &id, now)?;
        }
        tx.execute(
            "UPDATE assistant_im_deliveries SET state = 'submitting' WHERE id = ?1 AND state = 'pending'",
            [&id],
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(Some(PreparedDelivery {
            id,
            lease: lease(&state),
            chat_id,
            text,
        }))
    }

    /// Called once at Primary startup before starting its sender. A prior
    /// process may have sent any submitting row, so it must never be replayed.
    pub fn recover_assistant_im_deliveries(&self) -> Result<usize> {
        self.with_conn_internal(|conn| {
            Ok(conn.execute(
                "UPDATE assistant_im_deliveries SET state = 'unknown', text = '' WHERE state = 'submitting'",
                [],
            )?)
        })
    }

    pub fn cancel_unsubmitted_assistant_im_delivery(&self, id: &str) -> Result<()> {
        let updated = self.with_conn_internal(|conn| {
            Ok(conn.execute(
                "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
                 WHERE id = ?1 AND state = 'submitting'",
                [id],
            )?)
        })?;
        if updated > 0 {
            changed();
        }
        Ok(())
    }

    pub fn finish_assistant_im_delivery(&self, id: &str, message_id: Option<i64>) -> Result<()> {
        let state = if message_id.is_some_and(|id| id > 0) {
            "accepted"
        } else {
            "unknown"
        };
        self.with_conn_internal(|conn| {
            conn.execute(
                "UPDATE assistant_im_deliveries SET state = ?1, message_id = ?2, text = ''
                 WHERE id = ?3 AND state = 'submitting'",
                params![state, message_id.filter(|id| *id > 0), id],
            )?;
            Ok(())
        })?;
        changed();
        Ok(())
    }

    pub fn assistant_im_delivery_receipts(&self) -> Result<Vec<DeliveryReceipt>> {
        self.assistant_im_notification_status()
            .map(|(_, receipts)| receipts)
    }

    /// One timezone snapshot drives both owner policy and pending explanations.
    pub fn assistant_im_notification_status(
        &self,
    ) -> Result<(NotificationPolicy, Vec<DeliveryReceipt>)> {
        let policy = notification_policy();
        let receipts = self
            .assistant_im_delivery_receipts_with_policy(chrono::Utc::now().timestamp(), &policy)?;
        Ok((policy, receipts))
    }

    pub(super) fn assistant_im_delivery_receipts_with_policy(
        &self,
        now: i64,
        policy: &NotificationPolicy,
    ) -> Result<Vec<DeliveryReceipt>> {
        let conn = self.read_conn()?;
        let deferred_reason =
            super::notification_policy::deferred_reason(&conn, policy.timezone(), now)?;
        let mut stmt = conn.prepare(
            "SELECT id, request_id, state, message_id, created_at, completion_notice, expires_at
             FROM assistant_im_deliveries ORDER BY created_at DESC, rowid DESC LIMIT 100",
        )?;
        let mut rows = stmt
            .query_map([], |row| {
                let state = parse_state(&row.get::<_, String>(2)?)?;
                Ok(DeliveryReceipt {
                    id: row.get(0)?,
                    request_id: row.get(1)?,
                    state,
                    message_id: row.get(3)?,
                    created_at: row.get(4)?,
                    deferred_reason: if state == DeliveryState::Pending
                        && row.get::<_, bool>(5)?
                        && row.get::<_, i64>(6)? > now
                    {
                        deferred_reason
                    } else {
                        None
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if let Some(state) =
            load(&conn)?.filter(|state| state.view.state == PairingState::AwaitingActivation)
        {
            let activation = conn
                .query_row(
                    "SELECT id,state,message_id,created_at FROM assistant_im_activations
                 WHERE connection_id = ?1 AND generation = ?2",
                    params![state.view.connection_id, state.view.generation],
                    |row| {
                        Ok(DeliveryReceipt {
                            id: row.get(0)?,
                            request_id: format!("pairing:{}", state.view.generation),
                            state: parse_state(&row.get::<_, String>(1)?)?,
                            message_id: row.get(2)?,
                            created_at: row.get(3)?,
                            deferred_reason: None,
                        })
                    },
                )
                .optional()?;
            if let Some(activation) = activation {
                rows.insert(0, activation);
                rows.truncate(100);
            }
        }
        Ok(rows)
    }
}
