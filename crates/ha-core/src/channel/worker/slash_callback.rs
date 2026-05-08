use tokio::sync::mpsc;

use crate::channel::types::{ChannelId, ChatType, MsgContext};

/// Re-inject a `slash:<cmd> <arg>` button-callback payload as a synthetic
/// inbound `/cmd arg` message, so the worker processes it through the normal
/// slash-command dispatch path.
///
/// `rest` is the payload **after** the `slash:` prefix (caller is expected to
/// strip it). All 7 button-capable channels (Telegram / Feishu / Discord /
/// Slack / QQ Bot / LINE / Google Chat) route through this single helper —
/// arg-picker buttons are always preceded by a real inbound `/cmd` that wrote
/// `chat_type` into `channel_conversations`, so we look it up by chat_id +
/// thread_id; missing rows fall back to `Dm` (the conservative default —
/// matches `ChatType::from_lowercase` and won't trip group mention-gating in
/// the dispatcher).
#[allow(clippy::too_many_arguments)]
pub async fn inject_slash_callback(
    channel_id: ChannelId,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    sender_id: &str,
    message_id: &str,
    rest: &str,
    inbound_tx: &mpsc::Sender<MsgContext>,
    source: &'static str,
) {
    if chat_id.is_empty() {
        app_warn!(
            "channel",
            source,
            "[{}] slash callback dropped: empty chat_id (rest={})",
            account_id,
            rest
        );
        return;
    }

    let chat_type = crate::globals::get_channel_db()
        .and_then(|db| {
            db.get_chat_type(&channel_id.to_string(), account_id, chat_id, thread_id)
                .ok()
                .flatten()
        })
        .unwrap_or(ChatType::Dm);

    let msg = MsgContext {
        channel_id,
        account_id: account_id.to_string(),
        sender_id: sender_id.to_string(),
        sender_name: None,
        sender_username: None,
        chat_id: chat_id.to_string(),
        chat_type,
        chat_title: None,
        thread_id: thread_id.map(str::to_string),
        message_id: message_id.to_string(),
        text: Some(format!("/{}", rest)),
        media: Vec::new(),
        reply_to_message_id: None,
        timestamp: chrono::Utc::now(),
        was_mentioned: true,
        raw: serde_json::json!({ "slash_callback_rest": rest }),
    };

    if let Err(e) = inbound_tx.send(msg).await {
        app_warn!(
            "channel",
            source,
            "[{}] Failed to inject slash callback: {}",
            account_id,
            e
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inject_slash_callback_builds_msgctx_with_slash_text() {
        let (tx, mut rx) = mpsc::channel(4);
        inject_slash_callback(
            ChannelId::Discord,
            "acct-1",
            "channel-42",
            None,
            "user-7",
            "msg-99",
            "think high",
            &tx,
            "test",
        )
        .await;

        let msg = rx.recv().await.expect("inbound msg sent");
        assert_eq!(msg.channel_id, ChannelId::Discord);
        assert_eq!(msg.account_id, "acct-1");
        assert_eq!(msg.chat_id, "channel-42");
        assert_eq!(msg.sender_id, "user-7");
        assert_eq!(msg.message_id, "msg-99");
        assert_eq!(msg.text.as_deref(), Some("/think high"));
        assert!(msg.was_mentioned);
        assert!(msg.media.is_empty());
        // No initialized channel_db in this test → Dm fallback.
        assert_eq!(msg.chat_type, ChatType::Dm);
    }

    #[tokio::test]
    async fn inject_slash_callback_drops_empty_chat_id() {
        let (tx, mut rx) = mpsc::channel(4);
        inject_slash_callback(
            ChannelId::Discord,
            "acct-1",
            "",
            None,
            "user-7",
            "msg-99",
            "think high",
            &tx,
            "test",
        )
        .await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn inject_slash_callback_passes_thread_id_through() {
        let (tx, mut rx) = mpsc::channel(4);
        inject_slash_callback(
            ChannelId::Slack,
            "acct",
            "C1",
            Some("1700000000.000100"),
            "U1",
            "1700000000.000200",
            "permission yolo",
            &tx,
            "test",
        )
        .await;
        let msg = rx.recv().await.expect("inbound msg sent");
        assert_eq!(msg.thread_id.as_deref(), Some("1700000000.000100"));
        assert_eq!(msg.text.as_deref(), Some("/permission yolo"));
    }
}
