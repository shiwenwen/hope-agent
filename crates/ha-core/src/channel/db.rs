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
/// associated with one `session_id`. The mapping is **1:1 in both
/// directions** — a chat can only attach to one session at a time, and a
/// session can only be reached from one IM chat at a time. When a new chat
/// takes over a session, the previously-attached chat is physically
/// detached and notified via [`EVENT_CHANNEL_SESSION_EVICTED`].
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

/// EventBus topic emitted when an existing IM attach is **evicted** because
/// another chat is taking over the same `session_id` — see
/// [`ChannelDB::attach_session`] / [`ChannelDB::update_session`]. The
/// eviction watcher subscribes and pushes a "this chat has been taken
/// over; you've left the previous session" notice to the affected chat.
///
/// One event per evicted chat. Payload field names are listed in
/// [`payload_keys`].
pub const EVENT_CHANNEL_SESSION_EVICTED: &str = "channel:session_evicted";

/// JSON payload keys for [`EVENT_CHANNEL_SESSION_EVICTED`]. Shared between
/// the emit site (db.rs) and the subscriber (eviction_watcher) so a
/// rename can't drift the two halves out of sync.
pub mod payload_keys {
    pub const CHANNEL_ID: &str = "channelId";
    pub const ACCOUNT_ID: &str = "accountId";
    pub const CHAT_ID: &str = "chatId";
    pub const THREAD_ID: &str = "threadId";
    pub const SESSION_ID: &str = "sessionId";
}

/// One row evicted during a takeover: the chat that used to attach the
/// target session before someone else came in.
struct Evictee {
    channel_id: String,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
}

/// Atomically delete every attach row bound to `target_session_id` whose
/// chat is **not** `(channel_id, account_id, chat_id, thread_id)`, and
/// return the deleted rows for downstream notification. One round-trip
/// via `DELETE ... RETURNING` so attach_session / update_session don't
/// pay a separate SELECT pass.
fn evict_others(
    conn: &rusqlite::Connection,
    target_session_id: &str,
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) -> Result<Vec<Evictee>> {
    let mut stmt = conn.prepare(
        "DELETE FROM channel_conversations \
         WHERE session_id = ?1 \
           AND NOT (channel_id = ?2 AND account_id = ?3 AND chat_id = ?4 \
                    AND COALESCE(thread_id, '') = COALESCE(?5, '')) \
         RETURNING channel_id, account_id, chat_id, thread_id",
    )?;
    let rows = stmt
        .query_map(
            params![
                target_session_id,
                channel_id,
                account_id,
                chat_id,
                thread_id
            ],
            |r| {
                Ok(Evictee {
                    channel_id: r.get(0)?,
                    account_id: r.get(1)?,
                    chat_id: r.get(2)?,
                    thread_id: r.get(3)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn emit_evictions(evictees: &[Evictee], session_id: &str) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    for e in evictees {
        bus.emit(
            EVENT_CHANNEL_SESSION_EVICTED,
            serde_json::json!({
                payload_keys::CHANNEL_ID: e.channel_id,
                payload_keys::ACCOUNT_ID: e.account_id,
                payload_keys::CHAT_ID: e.chat_id,
                payload_keys::THREAD_ID: e.thread_id,
                payload_keys::SESSION_ID: session_id,
            }),
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
        source: row.get(9)?,
        attached_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

const FULL_COLS: &str =
    "id, channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, \
     chat_type, source, attached_at, created_at, updated_at";

impl ChannelDB {
    pub fn new(session_db: Arc<SessionDB>) -> Self {
        Self { session_db }
    }

    /// Run the migration to create channel_conversations table.
    /// Called once during app startup.
    ///
    /// Two prior schema shapes get DROP-and-rebuilt: (a) the pre-handover
    /// table without `source` / `attached_at`; (b) the multi-attach table
    /// with `is_primary`. Both are dropped without preserving rows — the
    /// IM worker re-creates entries lazily on the next inbound message.
    pub fn migrate(&self) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let has_table = conn
            .prepare("SELECT id FROM channel_conversations LIMIT 1")
            .is_ok();
        let has_source = conn
            .prepare("SELECT source FROM channel_conversations LIMIT 1")
            .is_ok();
        let has_is_primary = conn
            .prepare("SELECT is_primary FROM channel_conversations LIMIT 1")
            .is_ok();

        if has_table && (!has_source || has_is_primary) {
            // Either legacy (no source) or multi-attach (still has
            // is_primary) — drop and rebuild on the new 1:1 shape.
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
            -- 1:1 in the other direction too: at most one attach row per
            -- session_id. attach_session evicts any existing row before
            -- inserting/updating to enforce this.
            CREATE UNIQUE INDEX IF NOT EXISTS uq_channel_conv_session
                ON channel_conversations(session_id);
            CREATE INDEX IF NOT EXISTS idx_channel_conv_lookup
                ON channel_conversations(channel_id, account_id, chat_id);",
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
            // Update timestamp + sender info on the existing attach row.
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
        // source = "inbound".
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO channel_conversations \
                (channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, \
                 chat_type, source, attached_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10)",
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

    /// Whitelist check: is `(channel, account, chat, thread)` a conversation this
    /// system has actually recorded (an inbound message or an explicit attach
    /// created the row)? Cron delivery uses this to refuse fan-out to chat ids a
    /// (possibly prompt-injected) model invented — only conversations surfaced by
    /// `list_channel_targets`, which reads the same `channel_conversations` table,
    /// are valid delivery destinations.
    pub fn conversation_exists(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<bool> {
        Ok(self
            .get_session(channel_id, account_id, chat_id, thread_id)?
            .is_some())
    }

    /// Look up the persisted `chat_type` for a (channel, account, chat, thread)
    /// quadruple. Used by callback re-injection paths (e.g. Feishu's
    /// `card.action.trigger` → synthetic inbound) that don't carry chat_type
    /// in their envelope but always follow at least one prior inbound message
    /// which already populated the conversation row.
    pub fn get_chat_type(
        &self,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChatType>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let result = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT chat_type FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4 \
                 ORDER BY updated_at DESC LIMIT 1",
                params![channel_id, account_id, chat_id, tid],
                |row| row.get::<_, String>(0),
            )
        } else {
            conn.query_row(
                "SELECT chat_type FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL \
                 ORDER BY updated_at DESC LIMIT 1",
                params![channel_id, account_id, chat_id],
                |row| row.get::<_, String>(0),
            )
        };

        match result {
            Ok(s) => Ok(Some(ChatType::from_lowercase(&s))),
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

    /// List conversations whose `updated_at` falls within the given
    /// activity window, ordered most-recent-first, filtered to "real"
    /// user conversations (non-incognito, non-cron, non-subagent).
    ///
    /// `window_secs` is the look-back window; `limit` caps the result.
    /// Used by `startup_watcher` to pick recipients for the "back
    /// online" notice. The 1:1 attach invariant (`uq_channel_conv_chat`
    /// + `uq_channel_conv_session`) guarantees each (channel, account,
    /// chat, thread) appears at most once.
    pub fn list_recent_active_conversations(
        &self,
        window_secs: u64,
        limit: usize,
    ) -> Result<Vec<ChannelConversation>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let cutoff =
            (chrono::Utc::now() - chrono::Duration::seconds(window_secs as i64)).to_rfc3339();
        // FULL_COLS would JOIN-collide with `sessions.id` / `sessions.session_id`, so
        // qualify every column with `cc.` here. Column order must match
        // `row_to_conversation`.
        let sql = "SELECT cc.id, cc.channel_id, cc.account_id, cc.chat_id, cc.thread_id, \
                          cc.session_id, cc.sender_id, cc.sender_name, cc.chat_type, \
                          cc.source, cc.attached_at, cc.created_at, cc.updated_at \
                   FROM channel_conversations cc \
                   JOIN sessions s ON s.id = cc.session_id \
                   WHERE s.incognito = 0 \
                     AND s.is_cron = 0 \
                     AND s.parent_session_id IS NULL \
                     AND cc.updated_at >= ?1 \
                   ORDER BY cc.updated_at DESC \
                   LIMIT ?2";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![cutoff, limit as i64], |row| {
                row_to_conversation(row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Look up the IM attach row for a session, if any. With 1:1 attach
    /// the unique index `uq_channel_conv_session` guarantees at most one
    /// row per session.
    pub fn get_conversation_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ChannelConversation>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let sql = format!("SELECT {FULL_COLS} FROM channel_conversations WHERE session_id = ?1");
        let result = conn.query_row(&sql, params![session_id], row_to_conversation);

        match result {
            Ok(conv) => Ok(Some(conv)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Re-point an existing chat to a new session_id (used by `/new` and
    /// `/agent` from inside an IM chat). Evicts any chat currently attached
    /// to `new_session_id` so the 1:1 invariant holds, then swaps this
    /// chat's session_id and bumps `updated_at`. Returns true when this
    /// chat's row was updated.
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

        // Evict any chat currently attached to new_session_id that is
        // NOT this chat, so the 1:1 invariant on session_id holds. Same
        // chat → same session_id is a no-op (the chat won't match its
        // own coords). Each evictee gets a notice via emit_evictions
        // after the lock is dropped.
        let evicted = evict_others(
            &conn,
            new_session_id,
            channel_id,
            account_id,
            chat_id,
            thread_id,
        )?;

        let rows = if let Some(tid) = thread_id {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, updated_at = ?2 \
                 WHERE channel_id = ?3 AND account_id = ?4 AND chat_id = ?5 AND thread_id = ?6",
                params![new_session_id, now, channel_id, account_id, chat_id, tid],
            )?
        } else {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, updated_at = ?2 \
                 WHERE channel_id = ?3 AND account_id = ?4 AND chat_id = ?5 AND thread_id IS NULL",
                params![new_session_id, now, channel_id, account_id, chat_id],
            )?
        };

        drop(conn);
        if rows > 0 {
            emit_evictions(&evicted, new_session_id);
        }
        Ok(rows > 0)
    }

    /// Attach (channel, account, chat, thread) to `session_id`, evicting
    /// whichever chat (if any) was previously attached to the same
    /// `session_id` so the 1:1 invariant holds. Each evicted chat gets one
    /// `EVENT_CHANNEL_SESSION_EVICTED` event after the lock is dropped.
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

        // 1. Physically detach any chat currently bound to the target
        //    session that isn't the incoming chat — 1:1 invariant.
        //    Each evictee gets a "you've been taken over" notice via
        //    EVENT_CHANNEL_SESSION_EVICTED after we drop the lock.
        let evicted = evict_others(
            &conn, session_id, channel_id, account_id, chat_id, thread_id,
        )?;

        // 2. UPDATE the existing chat row, or INSERT a new one. If the
        //    incoming chat was previously attached to another session, the
        //    UPDATE silently relocates it (the source session is now
        //    headless — no notice to send because the only attach row was
        //    this chat itself).
        let updated = if let Some(tid) = thread_id {
            conn.execute(
                "UPDATE channel_conversations \
                 SET session_id = ?1, source = ?2, attached_at = ?3, \
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
                 SET session_id = ?1, source = ?2, attached_at = ?3, \
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
                     sender_name, chat_type, source, attached_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10)",
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

        // 3. Make the attached session non-incognito (channel has external
        //    counterparty whose messages must persist).
        conn.execute(
            "UPDATE sessions SET incognito = 0 WHERE id = ?1 AND incognito = 1",
            params![session_id],
        )?;

        drop(conn);
        emit_evictions(&evicted, session_id);
        Ok(())
    }

    /// Remove the attach row for (channel, account, chat, thread). Returns
    /// the detached `session_id` (or `None` when no row matched). 1:1
    /// invariant means there's at most one row to delete; the session is
    /// simply left headless after this call (no event needed — `/session
    /// exit` already replies "Detached..." in the IM chat).
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

        let sid: Option<String> = if let Some(tid) = thread_id {
            conn.query_row(
                "SELECT session_id FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT session_id FROM channel_conversations \
                 WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
                params![channel_id, account_id, chat_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        };

        let Some(sid) = sid else {
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

        Ok(Some(sid))
    }
}

#[cfg(test)]
mod recent_active_tests {
    use super::*;
    use crate::session::SessionDB;
    use chrono::{Duration, Utc};

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}-{}-{}.sqlite3",
            name,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        std::env::temp_dir().join(unique)
    }

    /// Fresh SessionDB + ChannelDB pair with the channel_conversations
    /// table migrated. Cleans up on drop via the Cleanup guard the
    /// caller binds.
    struct TestDb {
        channel: ChannelDB,
        session: Arc<SessionDB>,
        path: std::path::PathBuf,
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn open_db(name: &str) -> TestDb {
        let path = temp_db_path(name);
        let session = Arc::new(SessionDB::open(&path).expect("open session db"));
        let channel = ChannelDB::new(session.clone());
        channel.migrate().expect("migrate channel db");
        TestDb {
            channel,
            session,
            path,
        }
    }

    /// Insert a `channel_conversations` row referencing `session_id`
    /// with an explicit `updated_at`. Bypasses the helper functions
    /// because we want to control the timestamp for window filtering.
    fn insert_conv(
        db: &TestDb,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        session_id: &str,
        updated_at: &str,
    ) {
        let conn = db.session.conn.lock().expect("lock");
        conn.execute(
            "INSERT INTO channel_conversations \
             (channel_id, account_id, chat_id, thread_id, session_id, chat_type, source, created_at, updated_at) \
             VALUES (?1, ?2, ?3, NULL, ?4, 'dm', 'inbound', ?5, ?5)",
            params![channel_id, account_id, chat_id, session_id, updated_at],
        )
        .expect("insert channel_conversations");
    }

    fn mark_cron(db: &TestDb, session_id: &str) {
        let conn = db.session.conn.lock().expect("lock");
        conn.execute(
            "UPDATE sessions SET is_cron = 1 WHERE id = ?1",
            params![session_id],
        )
        .expect("mark cron");
    }

    fn mark_incognito(db: &TestDb, session_id: &str) {
        let conn = db.session.conn.lock().expect("lock");
        conn.execute(
            "UPDATE sessions SET incognito = 1 WHERE id = ?1",
            params![session_id],
        )
        .expect("mark incognito");
    }

    #[test]
    fn filters_incognito_cron_and_subagent_sessions() {
        let db = open_db("startup-filter");
        let agent = crate::agent_loader::DEFAULT_AGENT_ID;
        let now = Utc::now();
        let stamp = now.to_rfc3339();

        let keep = db.session.create_session(agent).expect("keep").id;
        let incog = db.session.create_session(agent).expect("incog").id;
        let cron = db.session.create_session(agent).expect("cron").id;
        let sub = db
            .session
            .create_session_with_parent(agent, Some(&keep))
            .expect("sub")
            .id;

        mark_incognito(&db, &incog);
        mark_cron(&db, &cron);

        insert_conv(&db, "telegram", "acc", "chat-keep", &keep, &stamp);
        insert_conv(&db, "telegram", "acc", "chat-incog", &incog, &stamp);
        insert_conv(&db, "telegram", "acc", "chat-cron", &cron, &stamp);
        insert_conv(&db, "telegram", "acc", "chat-sub", &sub, &stamp);

        let rows = db
            .channel
            .list_recent_active_conversations(24 * 3600, 100)
            .expect("query");
        let chats: Vec<_> = rows.iter().map(|r| r.chat_id.as_str()).collect();
        assert_eq!(chats, vec!["chat-keep"], "got: {chats:?}");
    }

    #[test]
    fn respects_activity_window_and_global_max() {
        let db = open_db("startup-window");
        let agent = crate::agent_loader::DEFAULT_AGENT_ID;
        let now = Utc::now();

        // 5 sessions, alternating recent / old timestamps + one barely
        // outside the window.
        for i in 0..5 {
            let sid = db.session.create_session(agent).expect("session").id;
            let chat = format!("chat-{i}");
            // i=0,1 within window (1h ago), i=2,3 outside (3 days ago),
            // i=4 barely outside (window + 10s)
            let offset = match i {
                0 | 1 => Duration::hours(1),
                2 | 3 => Duration::days(3),
                _ => Duration::seconds(48 * 3600 + 10),
            };
            let stamp = (now - offset).to_rfc3339();
            insert_conv(&db, "telegram", "acc", &chat, &sid, &stamp);
        }

        // 48h window — keeps i=0, i=1; drops i=2, i=3, i=4
        let rows = db
            .channel
            .list_recent_active_conversations(48 * 3600, 100)
            .expect("query");
        assert_eq!(rows.len(), 2);

        // global_max=1 caps to most-recent only
        let rows = db
            .channel
            .list_recent_active_conversations(48 * 3600, 1)
            .expect("query");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn multiple_chats_on_one_account_each_returned_once() {
        let db = open_db("startup-fanout");
        let agent = crate::agent_loader::DEFAULT_AGENT_ID;
        let stamp = Utc::now().to_rfc3339();

        // One telegram bot active in 1 DM + 3 group chats.
        for i in 0..4 {
            let sid = db.session.create_session(agent).expect("session").id;
            insert_conv(
                &db,
                "telegram",
                "acc-bot",
                &format!("chat-{i}"),
                &sid,
                &stamp,
            );
        }

        let rows = db
            .channel
            .list_recent_active_conversations(24 * 3600, 100)
            .expect("query");
        assert_eq!(rows.len(), 4, "every chat on the bot should appear once");

        let mut chats: Vec<_> = rows.iter().map(|r| r.chat_id.clone()).collect();
        chats.sort();
        assert_eq!(chats, vec!["chat-0", "chat-1", "chat-2", "chat-3"]);
    }
}
