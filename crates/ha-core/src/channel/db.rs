use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

use super::types::ChatType;
use crate::session::SessionDB;

/// Manages the `channel_conversations` table that maps IM conversations to sessions.
pub struct ChannelDB {
    session_db: Arc<SessionDB>,
}

/// A row from the channel_conversations table.
///
/// Each row is an "attach": one (channel, account, chat, thread) is currently
/// associated with one `session_id`. Multiple chats can attach to the same
/// session (multi-IM observe), but only one row per session is `is_primary`,
/// which decides where outbound assistant text is delivered.
#[derive(Debug, Clone)]
pub struct ChannelConversation {
    pub id: i64,
    pub channel_id: String,
    pub account_id: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub session_id: String,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub chat_type: String,
    /// Whether this row is the primary attach for `session_id`. Outbound
    /// assistant text is only sent to the channel of the primary row;
    /// secondary rows still observe streaming events but do not receive
    /// final messages.
    pub is_primary: bool,
    /// How this attach was created: `"inbound"` (auto, IM message), `"attach"`
    /// (explicit `/session <id>` from IM), or `"handover"` (explicit GUI
    /// handover or `/handover`).
    pub source: String,
    /// When this attach was created/last reattached. RFC3339.
    pub attached_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Source-of-attach values stored in `channel_conversations.source`. Use these
/// constants instead of inline strings so a typo trips the compiler.
pub const ATTACH_SOURCE_INBOUND: &str = "inbound";
pub const ATTACH_SOURCE_ATTACH: &str = "attach";
pub const ATTACH_SOURCE_HANDOVER: &str = "handover";

/// EventBus topic emitted when the primary attach for a session changes —
/// after [`ChannelDB::attach_session`], [`ChannelDB::detach_session`],
/// [`ChannelDB::set_primary`], or [`ChannelDB::update_session`] mutates an
/// `is_primary` row. Channel workers subscribe to deliver "you are now
/// primary" / "you are now observing" messages on the wire.
///
/// Payload shape: `{ "sessionId": "<sid>" }`. Subscribers re-query
/// `list_attached` for the full detail to avoid baking a struct schema
/// into the topic.
pub const EVENT_CHANNEL_PRIMARY_CHANGED: &str = "channel:primary_changed";

fn emit_primary_changed(session_id: &str) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            EVENT_CHANNEL_PRIMARY_CHANGED,
            serde_json::json!({ "sessionId": session_id }),
        );
    }
}

fn chat_type_str(chat_type: &ChatType) -> &'static str {
    match chat_type {
        ChatType::Dm => "dm",
        ChatType::Group => "group",
        ChatType::Forum => "forum",
        ChatType::Channel => "channel",
    }
}

fn row_to_conversation(row: &rusqlite::Row) -> rusqlite::Result<ChannelConversation> {
    Ok(ChannelConversation {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        account_id: row.get(2)?,
        chat_id: row.get(3)?,
        thread_id: row.get(4)?,
        session_id: row.get(5)?,
        sender_id: row.get(6)?,
        sender_name: row.get(7)?,
        chat_type: row.get(8)?,
        is_primary: row.get::<_, i64>(9)? != 0,
        source: row.get(10)?,
        attached_at: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

const FULL_COLS: &str =
    "id, channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, \
     chat_type, is_primary, source, attached_at, created_at, updated_at";

impl ChannelDB {
    pub fn new(session_db: Arc<SessionDB>) -> Self {
        Self { session_db }
    }

    /// Run the migration to create channel_conversations table.
    /// Called once during app startup.
    ///
    /// Pre-handover schema lacked `is_primary` / `source` / `attached_at` and
    /// used a different UNIQUE shape that allowed multiple rows per chat. The
    /// new model gives a chat a single attach row at any time, so we wipe the
    /// legacy table and let it rebuild — the IM worker will lazily re-create
    /// channel_conversations on the next inbound message. Project↔channel
    /// reverse-claim is gone (see Phase A1) so no auto-routing is lost.
    pub fn migrate(&self) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let has_table = conn
            .prepare("SELECT id FROM channel_conversations LIMIT 1")
            .is_ok();
        let has_is_primary = conn
            .prepare("SELECT is_primary FROM channel_conversations LIMIT 1")
            .is_ok();

        if has_table && !has_is_primary {
            // Legacy schema — drop and rebuild on the new shape.
            conn.execute_batch("DROP TABLE IF EXISTS channel_conversations;")?;
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS channel_conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                chat_id TEXT NOT NULL,
                thread_id TEXT,
                session_id TEXT NOT NULL,
                sender_id TEXT,
                sender_name TEXT,
                chat_type TEXT NOT NULL DEFAULT 'dm',
                is_primary INTEGER NOT NULL DEFAULT 1,
                source TEXT NOT NULL DEFAULT 'inbound',
                attached_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            -- A chat (channel, account, chat, thread) is attached to one
            -- session at a time. NULL thread_id needs COALESCE — SQLite
            -- treats NULL ≠ NULL by default, which would let a thread-less
            -- chat have two attach rows.
            CREATE UNIQUE INDEX IF NOT EXISTS uq_channel_conv_chat
                ON channel_conversations(channel_id, account_id, chat_id, COALESCE(thread_id, ''));
            CREATE INDEX IF NOT EXISTS idx_channel_conv_session
                ON channel_conversations(session_id);
            CREATE INDEX IF NOT EXISTS idx_channel_conv_lookup
                ON channel_conversations(channel_id, account_id, chat_id);
            CREATE INDEX IF NOT EXISTS idx_channel_conv_primary
                ON channel_conversations(session_id, is_primary);",
        )?;

        Ok(())
    }

    /// Resolve an existing session for the given channel conversation, or create a new one.
    ///
    /// Returns the session_id (existing or newly created).
    pub fn resolve_or_create_session(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        sender_id: Option<&str>,
        sender_name: Option<&str>,
        chat_type: &ChatType,
        agent_id: &str,
    ) -> Result<String> {
        // Check for existing mapping (hold lock across check+insert to avoid race condition)
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let existing: Option<String> = conn
            .query_row(
                "SELECT session_id FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 \
                   AND (thread_id IS ?4 OR (?4 IS NULL AND thread_id IS NULL))",
                params![channel_id, account_id, chat_id, thread_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(existing) = existing {
            // Update timestamp and sender info — scoped to **this** attach row
            // (the (channel, account, chat, thread) tuple), not the entire
            // session. With multi-attach, a session can have several IM chats
            // pointed at it; only the speaker's row should advance its
            // `updated_at` / sender metadata. Touching every row would
            // pollute `/status`, the channel chip, and the
            // updated-at-based fallback in `detach_session`.
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE channel_conversations \
                 SET updated_at = ?1, \
                     sender_id = COALESCE(?2, sender_id), \
                     sender_name = COALESCE(?3, sender_name) \
                 WHERE channel_id = ?4 AND account_id = ?5 AND chat_id = ?6 \
                   AND (thread_id IS ?7 OR (?7 IS NULL AND thread_id IS NULL))",
                params![
                    now,
                    sender_id,
                    sender_name,
                    channel_id,
                    account_id,
                    chat_id,
                    thread_id,
                ],
            )?;
            return Ok(existing);
        }

        // Release lock before creating session (which also acquires the lock)
        drop(conn);

        // Project ↔ channel reverse-claim is gone: IM messages no longer
        // auto-route into a project. To attach a session to a project, use
        // `/project <id>` from inside the IM chat.
        let session_meta = self
            .session_db
            .create_session_with_project(agent_id, None, None)?;
        let session_id = session_meta.id;

        // Title is left as None here — auto_title from first message content
        // will be applied in worker.rs, same as normal chat sessions.

        // Store the context_json with channel info
        let context = serde_json::json!({
            "channel": {
                "channelId": channel_id,
                "accountId": account_id,
                "chatId": chat_id,
                "threadId": thread_id,
                "chatType": format!("{:?}", chat_type).to_lowercase(),
                "senderName": sender_name,
            }
        });
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE sessions SET context_json = ?1 WHERE id = ?2",
            params![context.to_string(), session_id],
        )?;

        // Insert channel_conversations mapping. Inbound from IM ⇒
        // source = "inbound", is_primary = 1.
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO channel_conversations \
                (channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, \
                 chat_type, is_primary, source, attached_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?10, ?10)",
            params![
                channel_id,
                account_id,
                chat_id,
                thread_id,
                session_id,
                sender_id,
                sender_name,
                chat_type_str(chat_type),
                ATTACH_SOURCE_INBOUND,
                now,
            ],
        )?;
        // Channel sessions are driven by an external counterparty whose
        // messages must persist — incognito and channel are mutually exclusive.
        conn.execute(
            "UPDATE sessions SET incognito = 0 WHERE id = ?1 AND incognito = 1",
            params![session_id],
        )?;

        Ok(session_id)
    }

    /// Get the session ID currently attached to (channel, account, chat, thread).
    pub fn get_session(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let result = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT session_id FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4 \
                 ORDER BY updated_at DESC LIMIT 1",
                params![channel_id, account_id, chat_id, tid],
                |row| row.get::<_, String>(0),
            )
        } else {
            conn.query_row(
                "SELECT session_id FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL \
                 ORDER BY updated_at DESC LIMIT 1",
                params![channel_id, account_id, chat_id],
                |row| row.get::<_, String>(0),
            )
        };

        match result {
            Ok(sid) => Ok(Some(sid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all channel conversations for a given channel and account.
    pub fn list_conversations(
        &self,
        channel_id: &str,
        account_id: &str,
    ) -> Result<Vec<ChannelConversation>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let sql = format!(
            "SELECT {FULL_COLS} FROM channel_conversations \
             WHERE channel_id = ?1 AND account_id = ?2 ORDER BY updated_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![channel_id, account_id], row_to_conversation)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Look up the primary attach row for a session. Multi-IM observers
    /// surface here too — but the primary is what callers (approval / ask_user
    /// / final reply delivery) want, so we order is_primary DESC first.
    pub fn get_conversation_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ChannelConversation>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let sql = format!(
            "SELECT {FULL_COLS} FROM channel_conversations \
             WHERE session_id = ?1 \
             ORDER BY is_primary DESC, updated_at DESC LIMIT 1"
        );
        let result = conn.query_row(&sql, params![session_id], row_to_conversation);

        match result {
            Ok(conv) => Ok(Some(conv)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Re-point an existing chat to a new session_id (used by `/new` and
    /// `/agent` from inside an IM chat). The (channel, account, chat, thread)
    /// row already exists; we just swap its session_id and bump `updated_at`.
    /// Returns true when a row was updated.
    pub fn update_session(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        new_session_id: &str,
    ) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        // Snapshot the existing row so we can backfill the previous
        // session's primary if we're stealing it. Without this step a
        // `/new` from a chat that was the previous session's primary
        // leaves that session headless — outbound mirror / watcher
        // fan-outs would then drop messages on the floor.
        let existing: Option<(String, i64)> = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        };

        // Ensure the new session has only one primary across all attaches.
        // Demote any existing primary rows that point at the new session
        // before we promote this chat.
        conn.execute(
            "UPDATE channel_conversations SET is_primary = 0 WHERE session_id = ?1",
            params![new_session_id],
        )?;

        let rows = if let Some(tid) = thread_id {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, is_primary = 1, updated_at = ?2 \
                 WHERE channel_id = ?3 AND account_id = ?4 AND chat_id = ?5 AND thread_id = ?6",
                params![new_session_id, now, channel_id, account_id, chat_id, tid],
            )?
        } else {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, is_primary = 1, updated_at = ?2 \
                 WHERE channel_id = ?3 AND account_id = ?4 AND chat_id = ?5 AND thread_id IS NULL",
                params![new_session_id, now, channel_id, account_id, chat_id],
            )?
        };

        // Backfill primary on the orphaned previous session, if any.
        let mut orphaned: Option<String> = None;
        if rows > 0 {
            if let Some((prev_sid, prev_primary)) = existing {
                if prev_primary != 0 && prev_sid != new_session_id {
                    conn.execute(
                        "UPDATE channel_conversations SET is_primary = 1 \
                         WHERE id = (SELECT id FROM channel_conversations \
                                      WHERE session_id = ?1 \
                                      ORDER BY updated_at DESC LIMIT 1)",
                        params![prev_sid],
                    )?;
                    orphaned = Some(prev_sid);
                }
            }
        }

        drop(conn);
        if rows > 0 {
            emit_primary_changed(new_session_id);
            if let Some(prev_sid) = orphaned {
                emit_primary_changed(&prev_sid);
            }
        }
        Ok(rows > 0)
    }

    /// Attach (channel, account, chat, thread) to `session_id`, replacing any
    /// existing attach for that chat. The new attach is set primary by
    /// default; any other primary row on the same session is demoted.
    ///
    /// Used by `/session <id>` (source = "attach") and `/handover` /
    /// GUI handover flow (source = "handover"). Inbound auto-attach goes
    /// through [`Self::resolve_or_create_session`] (source = "inbound")
    /// instead.
    #[allow(clippy::too_many_arguments)]
    pub fn attach_session(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        session_id: &str,
        source: &str,
        sender_id: Option<&str>,
        sender_name: Option<&str>,
        chat_type: &ChatType,
    ) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        let chat_type_s = chat_type_str(chat_type);

        // 1. Snapshot the existing row, if any. If we're moving a
        //    chat from session A → session B, we'll need to know
        //    whether to backfill A's primary after the swap.
        let existing: Option<(String, i64)> = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        };

        // 2. Demote any existing primary on the target session.
        conn.execute(
            "UPDATE channel_conversations SET is_primary = 0 WHERE session_id = ?1",
            params![session_id],
        )?;

        // 3. UPDATE the existing chat row, or INSERT a new one.
        let updated = if let Some(tid) = thread_id {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, source = ?2, attached_at = ?3, is_primary = 1, \
                     sender_id = COALESCE(?4, sender_id), \
                     sender_name = COALESCE(?5, sender_name), \
                     chat_type = ?6, updated_at = ?3 \
                 WHERE channel_id = ?7 AND account_id = ?8 AND chat_id = ?9 AND thread_id = ?10",
                params![
                    session_id,
                    source,
                    now,
                    sender_id,
                    sender_name,
                    chat_type_s,
                    channel_id,
                    account_id,
                    chat_id,
                    tid,
                ],
            )?
        } else {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, source = ?2, attached_at = ?3, is_primary = 1, \
                     sender_id = COALESCE(?4, sender_id), \
                     sender_name = COALESCE(?5, sender_name), \
                     chat_type = ?6, updated_at = ?3 \
                 WHERE channel_id = ?7 AND account_id = ?8 AND chat_id = ?9 AND thread_id IS NULL",
                params![
                    session_id,
                    source,
                    now,
                    sender_id,
                    sender_name,
                    chat_type_s,
                    channel_id,
                    account_id,
                    chat_id,
                ],
            )?
        };

        if updated == 0 {
            conn.execute(
                "INSERT INTO channel_conversations \
                    (channel_id, account_id, chat_id, thread_id, session_id, sender_id, \
                     sender_name, chat_type, is_primary, source, attached_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?10, ?10)",
                params![
                    channel_id,
                    account_id,
                    chat_id,
                    thread_id,
                    session_id,
                    sender_id,
                    sender_name,
                    chat_type_s,
                    source,
                    now,
                ],
            )?;
        }

        // 4. Make the attached session non-incognito (channel has external
        //    counterparty whose messages must persist).
        conn.execute(
            "UPDATE sessions SET incognito = 0 WHERE id = ?1 AND incognito = 1",
            params![session_id],
        )?;

        // 5. If the chat was previously primary for a *different*
        //    session, that source session is now headless — promote
        //    its next-most-recent attach so outbound deliveries
        //    (final-reply mirror, primary watcher) still find a
        //    receiver.
        let mut orphaned: Option<String> = None;
        if let Some((prev_sid, prev_primary)) = existing {
            if prev_primary != 0 && prev_sid != session_id {
                conn.execute(
                    "UPDATE channel_conversations SET is_primary = 1 \
                     WHERE id = (SELECT id FROM channel_conversations \
                                  WHERE session_id = ?1 \
                                  ORDER BY updated_at DESC LIMIT 1)",
                    params![prev_sid],
                )?;
                orphaned = Some(prev_sid);
            }
        }

        drop(conn);
        emit_primary_changed(session_id);
        if let Some(prev_sid) = orphaned {
            emit_primary_changed(&prev_sid);
        }
        Ok(())
    }

    /// Remove the attach row for (channel, account, chat, thread). If the
    /// removed row was the primary for its session, the most-recently-attached
    /// remaining row on that session is promoted to primary.
    /// Returns the detached `session_id` (or `None` when no row matched).
    pub fn detach_session(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let row: Option<(String, i64)> = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        };

        let Some((sid, was_primary)) = row else {
            return Ok(None);
        };

        if let Some(tid) = thread_id {
            conn.execute(
                "DELETE FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
            )?;
        } else {
            conn.execute(
                "DELETE FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
            )?;
        }

        let primary_changed = was_primary != 0;
        if primary_changed {
            // Promote next-most-recent attach for this session (if any).
            conn.execute(
                "UPDATE channel_conversations SET is_primary = 1 \
                 WHERE id = (SELECT id FROM channel_conversations \
                              WHERE session_id = ?1 \
                              ORDER BY updated_at DESC LIMIT 1)",
                params![sid],
            )?;
        }

        drop(conn);
        if primary_changed {
            emit_primary_changed(&sid);
        }
        Ok(Some(sid))
    }

    /// Promote the row matching (channel, account, chat, thread) to primary
    /// for its session, demoting any other primary on the same session.
    /// Returns the affected `session_id` (or `None` when no row matched).
    pub fn set_primary(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let target: Option<(String, i64)> = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT session_id, is_primary FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        };

        let Some((sid, was_primary)) = target else {
            return Ok(None);
        };

        // Skip the demote/promote dance + watcher notification when the
        // row is already the only primary on its session — saves a
        // spurious "you are now primary" message on repeat calls.
        if was_primary != 0 {
            let other_primaries: i64 = conn.query_row(
                "SELECT COUNT(*) FROM channel_conversations \
                 WHERE session_id = ?1 AND is_primary = 1 \
                   AND NOT (channel_id = ?2 AND account_id = ?3 AND chat_id = ?4 \
                            AND COALESCE(thread_id, '') = COALESCE(?5, ''))",
                params![sid, channel_id, account_id, chat_id, thread_id],
                |r| r.get(0),
            )?;
            if other_primaries == 0 {
                return Ok(Some(sid));
            }
        }

        conn.execute(
            "UPDATE channel_conversations SET is_primary = 0 WHERE session_id = ?1",
            params![sid],
        )?;

        if let Some(tid) = thread_id {
            conn.execute(
                "UPDATE channel_conversations SET is_primary = 1 \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
            )?;
        } else {
            conn.execute(
                "UPDATE channel_conversations SET is_primary = 1 \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
            )?;
        }

        drop(conn);
        emit_primary_changed(&sid);
        Ok(Some(sid))
    }

    /// Cheap "is anything attached?" probe for the GUI → IM mirror fast
    /// path: skips the row materialisation + ORDER BY that
    /// [`Self::list_attached`] does, so chat-engine startup pays only an
    /// `EXISTS` round-trip when the session has no IM attaches (the
    /// common case for desktop-only users).
    pub fn has_attached(&self, session_id: &str) -> Result<bool> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let exists: i64 = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM channel_conversations WHERE session_id = ?1)",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    /// List every attach row for a session, primary first.
    pub fn list_attached(&self, session_id: &str) -> Result<Vec<ChannelConversation>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let sql = format!(
            "SELECT {FULL_COLS} FROM channel_conversations \
             WHERE session_id = ?1 \
             ORDER BY is_primary DESC, updated_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![session_id], row_to_conversation)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
