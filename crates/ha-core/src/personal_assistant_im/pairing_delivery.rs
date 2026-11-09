//! Owner confirmation is durable across Primary/Secondary processes. Only the
//! Primary claims the one permitted activation message before making HTTP.
use super::store::{hash, lease, load};
use super::*;
use crate::session::SessionDB;
use anyhow::Result;
use rusqlite::{params, Connection as SqlConnection, OptionalExtension, TransactionBehavior};

pub(super) fn migrate(conn: &SqlConnection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assistant_im_activations (
            id TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            callback_data TEXT NOT NULL,
            state TEXT NOT NULL,
            message_id INTEGER,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            UNIQUE(connection_id,generation)
        );",
    )?;
    Ok(())
}

impl SessionDB {
    pub fn prepare_assistant_im_activation(&self, now: i64) -> Result<Option<PreparedActivation>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        type Pending = (String, String, i64, i64, i64, String, i64);
        let pending: Option<Pending> = tx.query_row(
            "SELECT id,connection_id,generation,user_id,chat_id,callback_data,expires_at
             FROM assistant_im_activations WHERE state = 'pending' ORDER BY created_at,rowid LIMIT 1",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
        ).optional()?;
        let Some((id, connection_id, generation, user_id, chat_id, callback_data, expires_at)) =
            pending
        else {
            return Ok(None);
        };
        let connection = load(&tx)?;
        let allowed = connection.as_ref().is_some_and(|state| {
            state.view.connection_id == connection_id
                && state.view.generation == generation
                && state.view.state == PairingState::AwaitingActivation
                && state.view.user_id == Some(user_id)
                && state.view.chat_id == Some(chat_id)
                && state.activation_expires_at == expires_at
                && expires_at > now
                && callback_data
                    .strip_prefix(ACTIVATE_PREFIX)
                    .is_some_and(|value| {
                        state.activation_hash.as_deref() == Some(hash(value).as_str())
                    })
        });
        // Clear the short-lived nonce when claimed. A crash after this marker
        // is ambiguous and cannot replay the operation from persisted data.
        tx.execute(
            "UPDATE assistant_im_activations SET state = ?1,callback_data = '' WHERE id = ?2",
            params![if allowed { "submitting" } else { "cancelled" }, id],
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        if !allowed {
            return Ok(None);
        }
        let connection = connection.ok_or(AssistantImError::Unavailable)?;
        Ok(Some(PreparedActivation {
            id,
            prompt: ActivationPrompt {
                lease: lease(&connection),
                chat_id,
                callback_data,
            },
        }))
    }

    pub fn recover_assistant_im_activations(&self) -> Result<usize> {
        self.with_conn_internal(|conn| {
            Ok(conn.execute("UPDATE assistant_im_activations SET state = 'unknown',callback_data = '' WHERE state = 'submitting'",[])?)
        })
    }

    /// Only use when the adapter can prove it has not started any send.
    pub fn cancel_unsubmitted_assistant_im_activation(&self, id: &str) -> Result<()> {
        let updated = self.with_conn_internal(|conn| {
            Ok(conn.execute("UPDATE assistant_im_activations SET state = 'cancelled',callback_data = '' WHERE id = ?1 AND state = 'submitting'",[id])?)
        })?;
        if updated > 0 {
            changed();
        }
        Ok(())
    }

    pub fn finish_assistant_im_activation(&self, id: &str, message_id: Option<i64>) -> Result<()> {
        let message_id = message_id.filter(|id| *id > 0);
        let updated =
            self.with_conn_internal(|conn| {
                Ok(conn.execute(
                "UPDATE assistant_im_activations SET state = ?1,message_id = ?2,callback_data = ''
                 WHERE id = ?3 AND state = 'submitting'",
                params![if message_id.is_some() { "accepted" } else { "unknown" },message_id,id],
            )?)
            })?;
        if updated > 0 {
            changed();
        }
        Ok(())
    }
}
