//! Backend-owned FIFO pump for ordinary IM messages that arrive while their
//! attached session already has an active Desktop / HTTP / Channel turn.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use ha_core::channel::types::{ChatType, MsgContext};

static PUMPING_SESSIONS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChannelQueuedOrigin {
    pub channel_id: String,
    pub account_id: String,
    pub sender_id: String,
    pub sender_name: Option<String>,
    pub sender_username: Option<String>,
    pub chat_id: String,
    pub chat_type: String,
    pub chat_title: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: String,
    pub reply_to_message_id: Option<String>,
    pub timestamp: String,
    pub was_mentioned: bool,
}

impl ChannelQueuedOrigin {
    pub(super) fn from_message(msg: &MsgContext) -> Self {
        Self {
            channel_id: msg.channel_id.to_string(),
            account_id: msg.account_id.clone(),
            sender_id: msg.sender_id.clone(),
            sender_name: msg.sender_name.clone(),
            sender_username: msg.sender_username.clone(),
            chat_id: msg.chat_id.clone(),
            chat_type: match msg.chat_type {
                ChatType::Dm => "dm",
                ChatType::Group => "group",
                ChatType::Forum => "forum",
                ChatType::Channel => "channel",
            }
            .to_string(),
            chat_title: msg.chat_title.clone(),
            thread_id: msg.thread_id.clone(),
            message_id: msg.message_id.clone(),
            reply_to_message_id: msg.reply_to_message_id.clone(),
            timestamp: msg.timestamp.to_rfc3339(),
            was_mentioned: msg.was_mentioned,
        }
    }

    pub(super) fn request_id(&self) -> String {
        // Provider message ids are normally unique per account, but include
        // the full conversation coordinate before hashing so a buggy provider
        // cannot collide two chats. Hashing also keeps sender/chat ids out of
        // logs and GUI queue projections where request ids are visible.
        // A few community adapters may omit message ids; retain deterministic
        // retry deduplication there without collapsing every message in the
        // chat onto the same empty identity.
        let provider_identity = if self.message_id.trim().is_empty() {
            format!("timestamp:{}\0sender:{}", self.timestamp, self.sender_id)
        } else {
            format!("message:{}", self.message_id)
        };
        let identity = format!(
            "{}\0{}\0{}\0{}\0{}",
            self.channel_id,
            self.account_id,
            self.chat_id,
            self.thread_id.as_deref().unwrap_or_default(),
            provider_identity
        );
        format!("channel:{}", blake3::hash(identity.as_bytes()).to_hex())
    }

    pub(super) fn into_message(self) -> anyhow::Result<MsgContext> {
        Ok(MsgContext {
            channel_id: ha_core::channel::types::ChannelId::from_storage_str(&self.channel_id)?,
            account_id: self.account_id,
            sender_id: self.sender_id,
            sender_name: self.sender_name,
            sender_username: self.sender_username,
            chat_id: self.chat_id,
            chat_type: ChatType::from_lowercase(&self.chat_type),
            chat_title: self.chat_title,
            thread_id: self.thread_id,
            message_id: self.message_id,
            text: None,
            media: Vec::new(),
            reply_to_message_id: self.reply_to_message_id,
            timestamp: chrono::DateTime::parse_from_rfc3339(&self.timestamp)
                .map(|value| value.with_timezone(&chrono::Utc))?,
            was_mentioned: self.was_mentioned,
            raw: serde_json::Value::Null,
        })
    }
}

pub(super) fn kick(session_id: &str) {
    let should_spawn = PUMPING_SESSIONS
        .lock()
        .map(|mut sessions| sessions.insert(session_id.to_string()))
        .unwrap_or(false);
    if !should_spawn {
        return;
    }

    let session_id = session_id.to_string();
    tokio::spawn(async move {
        run_pump(&session_id).await;
        if let Ok(mut sessions) = PUMPING_SESSIONS.lock() {
            sessions.remove(&session_id);
        }

        // Close the empty-check/remove race: an enqueue that happened after
        // the pump's final SELECT but before removal observed us as active.
        let should_rekick = if let Some(db) = ha_core::get_session_db() {
            let sid = session_id.clone();
            db.run(move |db| -> anyhow::Result<bool> {
                Ok(db
                    .list_channel_queued_session_ids()?
                    .iter()
                    .any(|queued_sid| queued_sid == &sid))
            })
            .await
            .unwrap_or(false)
        } else {
            false
        };
        if should_rekick {
            kick(&session_id);
        }
    });
}

pub(super) fn recover_all() {
    let Some(db) = ha_core::get_session_db() else {
        return;
    };
    tokio::spawn(async move {
        match db.run(|db| db.list_channel_queued_session_ids()).await {
            Ok(session_ids) => {
                for session_id in session_ids {
                    kick(&session_id);
                }
            }
            Err(error) => app_warn!(
                "channel",
                "turn_queue_recover",
                "Failed to list queued IM sessions: {}",
                error
            ),
        }
    });
}

async fn run_pump(session_id: &str) {
    let mut failure_backoff = Duration::from_millis(250);
    loop {
        if ha_core::chat_engine::active_turn::current(session_id).is_some() {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }

        let dispatch_permit = super::dispatcher::acquire_channel_dispatch_permit().await;
        // The shared permit may have been occupied by a long model turn. A
        // fresher inbound can become active while we wait; never claim behind
        // that turn while holding scarce global capacity.
        if ha_core::chat_engine::active_turn::current(session_id).is_some() {
            drop(dispatch_permit);
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }

        let Some(db) = ha_core::get_session_db() else {
            return;
        };
        let turn_id = uuid::Uuid::new_v4().to_string();
        let sid = session_id.to_string();
        let claim_turn_id = turn_id.clone();
        let record = match db
            .run(move |db| db.claim_next_channel_turn_message_for_dispatch(&sid, &claim_turn_id))
            .await
        {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(error) => {
                app_warn!(
                    "channel",
                    "turn_queue_claim",
                    "Failed to claim queued IM message for session {}: {}",
                    session_id,
                    error
                );
                tokio::time::sleep(failure_backoff).await;
                failure_backoff = failure_backoff
                    .saturating_mul(2)
                    .min(Duration::from_secs(30));
                continue;
            }
        };

        let request_id = record.request_id.clone();
        let dispatch_result = super::dispatcher::handle_queued_channel_message(record).await;
        drop(dispatch_permit);
        if let Err(error) = dispatch_result.as_ref() {
            app_warn!(
                "channel",
                "turn_queue_dispatch",
                "Queued IM message dispatch failed for session {}: {}",
                session_id,
                ha_core::logging::redact_sensitive(&error.to_string())
            );
        }
        // Success paths may still exit before the user row becomes durable
        // (Stop during typing/preflight, hook block, attachment failure).
        // Reconcile unconditionally: a committed row is consumed; an untouched
        // dispatching row returns to FIFO; held_after_stop remains held.
        loop {
            let sid = session_id.to_string();
            let request = request_id.clone();
            let failed_turn_id = turn_id.clone();
            match db
                .run(move |db| {
                    db.reconcile_failed_turn_message_dispatch(&sid, &request, &failed_turn_id)
                })
                .await
            {
                Ok(()) => break,
                Err(error) => {
                    app_warn!(
                        "channel",
                        "turn_queue_reconcile",
                        "Failed to reconcile queued IM claim for session {}; retrying: {}",
                        session_id,
                        error
                    );
                    tokio::time::sleep(failure_backoff).await;
                    failure_backoff = failure_backoff
                        .saturating_mul(2)
                        .min(Duration::from_secs(30));
                }
            }
        }
        if dispatch_result.is_err() {
            tokio::time::sleep(failure_backoff).await;
            failure_backoff = failure_backoff
                .saturating_mul(2)
                .min(Duration::from_secs(30));
            continue;
        }
        failure_backoff = Duration::from_millis(250);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::channel::types::ChannelId;

    #[test]
    fn queued_origin_does_not_persist_raw_provider_payload() {
        let msg = MsgContext {
            channel_id: ChannelId::WeChat,
            account_id: "account".into(),
            sender_id: "sender".into(),
            sender_name: None,
            sender_username: None,
            chat_id: "chat".into(),
            chat_type: ChatType::Dm,
            chat_title: None,
            thread_id: None,
            message_id: "message".into(),
            text: Some("hello".into()),
            media: Vec::new(),
            reply_to_message_id: None,
            timestamp: chrono::Utc::now(),
            was_mentioned: false,
            raw: serde_json::json!({"context_token": "secret"}),
        };
        let value = serde_json::to_value(ChannelQueuedOrigin::from_message(&msg)).unwrap();
        assert!(!value.to_string().contains("secret"));
    }

    #[test]
    fn request_identity_is_stable_and_does_not_expose_routing_ids() {
        let mut msg = MsgContext {
            channel_id: ChannelId::WeChat,
            account_id: "private-account".into(),
            sender_id: "sender".into(),
            sender_name: None,
            sender_username: None,
            chat_id: "private-chat".into(),
            chat_type: ChatType::Dm,
            chat_title: None,
            thread_id: Some("thread".into()),
            message_id: "provider-message".into(),
            text: Some("hello".into()),
            media: Vec::new(),
            reply_to_message_id: None,
            timestamp: chrono::Utc::now(),
            was_mentioned: false,
            raw: serde_json::Value::Null,
        };
        let first = ChannelQueuedOrigin::from_message(&msg).request_id();
        let second = ChannelQueuedOrigin::from_message(&msg).request_id();
        assert_eq!(first, second);
        assert!(first.starts_with("channel:"));
        assert!(!first.contains("private-account"));
        assert!(!first.contains("private-chat"));

        msg.chat_id = "another-chat".into();
        assert_ne!(first, ChannelQueuedOrigin::from_message(&msg).request_id());

        msg.chat_id = "private-chat".into();
        msg.message_id.clear();
        let empty_id_first = ChannelQueuedOrigin::from_message(&msg).request_id();
        msg.timestamp += chrono::Duration::seconds(1);
        let empty_id_second = ChannelQueuedOrigin::from_message(&msg).request_id();
        assert_ne!(empty_id_first, empty_id_second);
    }
}
