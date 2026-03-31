use anyhow::Result;
use rusqlite::params;
use std::sync::Arc;

use super::types::ChatType;
use crate::session::SessionDB;

/// Manages the `channel_conversations` table that maps IM conversations to sessions.
pub struct ChannelDB {
    session_db: Arc<SessionDB>,
}

/// A row from the channel_conversations table.
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
    pub created_at: String,
    pub updated_at: String,
}

impl ChannelDB {
    pub fn new(session_db: Arc<SessionDB>) -> Self {
        Self { session_db }
    }

    /// Run the migration to create channel_conversations table.
    /// Called once during app startup.
    pub fn migrate(&self) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let has_table = conn
            .prepare("SELECT id FROM channel_conversations LIMIT 1")
            .is_ok();

        if !has_table {
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
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                    UNIQUE (channel_id, account_id, chat_id, thread_id)
                );
                CREATE INDEX IF NOT EXISTS idx_channel_conv_session ON channel_conversations(session_id);
                CREATE INDEX IF NOT EXISTS idx_channel_conv_lookup ON channel_conversations(channel_id, account_id, chat_id);"
            )?;
        }

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
        // Check for existing mapping
        if let Some(existing) = self.get_session(channel_id, account_id, chat_id, thread_id)? {
            // Update timestamp and sender info
            let now = chrono::Utc::now().to_rfc3339();
            let conn = self
                .session_db
                .conn
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            conn.execute(
                "UPDATE channel_conversations SET updated_at = ?1, sender_id = COALESCE(?2, sender_id), sender_name = COALESCE(?3, sender_name) WHERE session_id = ?4",
                params![now, sender_id, sender_name, existing],
            )?;
            return Ok(existing);
        }

        // Create a new session
        let session_meta = self.session_db.create_session(agent_id)?;
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

        // Insert channel_conversations mapping
        let now = chrono::Utc::now().to_rfc3339();
        let chat_type_str = match chat_type {
            ChatType::Dm => "dm",
            ChatType::Group => "group",
            ChatType::Forum => "forum",
            ChatType::Channel => "channel",
        };
        conn.execute(
            "INSERT INTO channel_conversations (channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, chat_type, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, chat_type_str, now, now],
        )?;

        Ok(session_id)
    }

    /// Get the session ID for an existing channel conversation.
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
                "SELECT session_id FROM channel_conversations WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id = ?4",
                params![channel_id, account_id, chat_id, tid],
                |row| row.get::<_, String>(0),
            )
        } else {
            conn.query_row(
                "SELECT session_id FROM channel_conversations WHERE channel_id = ?1 AND account_id = ?2 AND chat_id = ?3 AND thread_id IS NULL",
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

        let mut stmt = conn.prepare(
            "SELECT id, channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, chat_type, created_at, updated_at FROM channel_conversations WHERE channel_id = ?1 AND account_id = ?2 ORDER BY updated_at DESC"
        )?;

        let rows = stmt
            .query_map(params![channel_id, account_id], |row| {
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
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Look up a channel conversation by its linked session ID.
    pub fn get_conversation_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ChannelConversation>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let result = conn.query_row(
            "SELECT id, channel_id, account_id, chat_id, thread_id, session_id, sender_id, sender_name, chat_type, created_at, updated_at FROM channel_conversations WHERE session_id = ?1",
            params![session_id],
            |row| {
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
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        );

        match result {
            Ok(conv) => Ok(Some(conv)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Remap an existing channel conversation to a different session_id.
    /// Used when a slash command (/new, /agent) creates a new session for this conversation.
    /// Returns true if a row was updated, false if no mapping existed.
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

        let rows = if let Some(tid) = thread_id {
            conn.execute(
                "UPDATE channel_conversations SET session_id = ?1, updated_at = ?2 WHERE channel_id = ?3 AND account_id = ?4 AND chat_id = ?5 AND thread_id = ?6",
                params![new_session_id, now, channel_id, account_id, chat_id, tid],
            )?
        } else {
            conn.execute(
                "UPDATE channel_conversations SET session_id = ?1, updated_at = ?2 WHERE channel_id = ?3 AND account_id = ?4 AND chat_id = ?5 AND thread_id IS NULL",
                params![new_session_id, now, channel_id, account_id, chat_id],
            )?
        };

        Ok(rows > 0)
    }
}
