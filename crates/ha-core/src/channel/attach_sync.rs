//! Attach catch-up — when an IM chat takes over an existing session
//! (`/session <id>` from IM, GUI / desktop `/handover`, HTTP handover
//! route), the new chat had zero context for the conversation that's
//! been happening up to that point. This helper reads the session's
//! latest completed turn from the `messages` table and replays it as
//! one Final-mode delivery so the IM user sees where the conversation
//! left off.
//!
//! Best-effort by design: any failure here is logged and swallowed —
//! the attach itself already succeeded (`channel_db.attach_session`
//! returned `Ok`), and missing the catch-up is a missed echo not a
//! missed turn.
//!
//! In-flight turns are detected via the global
//! [`ChannelCancelRegistry`](crate::channel::ChannelCancelRegistry) and
//! get an extra "⏳ A reply is being generated and will arrive shortly."
//! line appended after the catch-up. The live reply itself is pushed by
//! the `im_mirror` final-reply path when that turn completes.

use std::sync::Arc;

use crate::attachments::MediaItem;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ChannelAccountConfig, ReplyPayload};
use crate::channel::worker::pipeline::DeliveryTarget;
use crate::channel::worker::{deliver_media_to_chat, send_text_chunks};
use crate::session::MessageRole;

/// Read the latest completed turn from the session and deliver assistant
/// final text + media to the chat as a one-shot `Final`-mode delivery.
///
/// Skips silently when the session has no assistant text and no media yet.
/// Appends a "reply is being generated" hint when an in-flight channel
/// turn is still active for the session.
pub async fn deliver_attach_catchup(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) {
    let session_db = match crate::globals::get_session_db() {
        Some(db) => db,
        None => {
            crate::app_warn!(
                "channel",
                "attach_sync",
                "session_db not initialised; skipping attach catch-up for {}",
                session_id
            );
            return;
        }
    };

    // Only need the last turn — 50 rows is a generous bound that covers
    // even very long thinking + tool_result + assistant chains. The
    // helper aligns the window to the latest `user` row so we always
    // have a clean turn boundary to slice from.
    const CATCHUP_WINDOW: u32 = 50;
    let messages = match session_db.load_session_messages_latest(session_id, CATCHUP_WINDOW) {
        Ok((msgs, _total, _has_more)) => msgs,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "attach_sync",
                "load_session_messages_latest({}) failed: {}",
                session_id,
                e
            );
            return;
        }
    };

    let snapshot = match latest_turn_snapshot(&messages) {
        Some(s) => s,
        None => return,
    };

    let caps = plugin.capabilities();
    let in_flight = crate::globals::get_channel_cancels()
        .map(|reg| reg.is_active(session_id))
        .unwrap_or(false);

    // 1. Send the assistant final text (if any). Re-uses the dispatcher's
    //    `send_text_chunks` so the markdown → native → chunk_message
    //    sequence + error logging stays in one place. Catch-up has no
    //    inbound message to quote, so `reply_to_message_id=None` and
    //    `preview=None` (no live preview to edit).
    if !snapshot.text.is_empty() {
        let target = DeliveryTarget {
            account_id: &account.id,
            chat_id,
            thread_id,
            reply_to_message_id: None,
        };
        send_text_chunks(plugin, &target, &snapshot.text, None).await;
    }

    // 2. Re-send the latest turn's media. We do not regenerate or
    //    re-upload — `deliver_media_to_chat` resolves each MediaItem's
    //    `local_path` through the plugin's normal native-vs-fallback
    //    partition (same path used by every live IM round delivery).
    if !snapshot.medias.is_empty() {
        deliver_media_to_chat(
            plugin,
            &account.id,
            chat_id,
            thread_id,
            &snapshot.medias,
            &caps,
        )
        .await;
    }

    // 3. In-flight hint: another channel-driven turn is currently running
    //    for this session, so the live final reply will land on this
    //    chat shortly via the IM mirror path. Surface that to the user
    //    so the catch-up doesn't look like the only thing they'll see.
    if in_flight {
        let payload = ReplyPayload {
            text: Some("⏳ A reply is being generated and will arrive shortly.".to_string()),
            thread_id: thread_id.map(|s| s.to_string()),
            parse_mode: None,
            ..ReplyPayload::text("")
        };
        if let Err(e) = plugin.send_message(&account.id, chat_id, &payload).await {
            crate::app_warn!(
                "channel",
                "attach_sync",
                "Catch-up in-flight hint to {}/{} failed: {}",
                account.id,
                chat_id,
                e
            );
        }
    }
}

/// Walk a session's messages bottom-up and return the latest turn's
/// assistant text + the media items emitted by tool calls in that turn.
///
/// "Latest turn" = everything with id strictly greater than the last
/// `user` row (or the entire vec when no `user` row exists). Returns
/// `None` when the latest turn has neither assistant text nor media —
/// the IM user has nothing to catch up on (fresh session, or only a
/// dangling user prompt with no model output yet).
fn latest_turn_snapshot(messages: &[crate::session::SessionMessage]) -> Option<TurnSnapshot> {
    if messages.is_empty() {
        return None;
    }

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User));
    let start = last_user_idx.map(|i| i + 1).unwrap_or(0);
    let turn = &messages[start..];
    if turn.is_empty() {
        return None;
    }

    // Take the very last assistant row's content as the final answer.
    // Earlier `text_block` rows in the same turn are intermediate
    // narration that already streamed (and would have been delivered to
    // the IM live in `split` mode on a normal turn) — replaying them
    // would double-print to a user who's just attaching, so we keep it
    // simple and align with `ImReplyMode::Final` semantics.
    let text = turn
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // Collect every tool result's media in turn order. Reuses
    // `agent::events::extract_media_items` so the parsing rules track
    // whatever the tool-event side emits (`__MEDIA_ITEMS__<json>\n…`).
    let mut medias: Vec<MediaItem> = Vec::new();
    for m in turn {
        if !matches!(m.role, MessageRole::Tool) {
            continue;
        }
        let Some(result) = m.tool_result.as_deref() else {
            continue;
        };
        let (_, items) = crate::agent::extract_media_items(result);
        medias.extend(items);
    }

    if text.is_empty() && medias.is_empty() {
        return None;
    }

    Some(TurnSnapshot { text, medias })
}

struct TurnSnapshot {
    text: String,
    medias: Vec<MediaItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{MessageRole, SessionMessage};

    fn mk_msg(id: i64, role: MessageRole, content: &str) -> SessionMessage {
        SessionMessage {
            id,
            session_id: "s1".into(),
            role,
            content: content.into(),
            timestamp: "2025-01-01T00:00:00Z".into(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
        }
    }

    fn mk_tool(id: i64, result: &str) -> SessionMessage {
        let mut m = mk_msg(id, MessageRole::Tool, "");
        m.tool_call_id = Some("call_1".into());
        m.tool_name = Some("send_attachment".into());
        m.tool_result = Some(result.to_string());
        m
    }

    #[test]
    fn empty_messages_returns_none() {
        assert!(latest_turn_snapshot(&[]).is_none());
    }

    #[test]
    fn fresh_user_only_returns_none() {
        let messages = vec![mk_msg(1, MessageRole::User, "hello")];
        assert!(latest_turn_snapshot(&messages).is_none());
    }

    #[test]
    fn assistant_only_text_no_media() {
        let messages = vec![
            mk_msg(1, MessageRole::User, "hi"),
            mk_msg(2, MessageRole::Assistant, "hello there"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "hello there");
        assert!(snap.medias.is_empty());
    }

    #[test]
    fn picks_only_last_turn_text() {
        let messages = vec![
            mk_msg(1, MessageRole::User, "u1"),
            mk_msg(2, MessageRole::Assistant, "old answer"),
            mk_msg(3, MessageRole::User, "u2"),
            mk_msg(4, MessageRole::Assistant, "new answer"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "new answer");
    }

    #[test]
    fn picks_final_assistant_after_intermediate_text_block() {
        // Intermediate text_block + tool round, then final assistant text.
        let messages = vec![
            mk_msg(1, MessageRole::User, "u"),
            mk_msg(2, MessageRole::TextBlock, "let me think..."),
            mk_msg(3, MessageRole::Assistant, "final answer"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "final answer");
    }

    #[test]
    fn extracts_media_from_tool_result() {
        let media_json = r#"[{"url":"/api/attachments/s/foo.png","localPath":"/tmp/foo.png","name":"foo.png","mimeType":"image/png","sizeBytes":1024,"kind":"image"}]"#;
        let result = format!("{}{}\nok", crate::agent::MEDIA_ITEMS_PREFIX, media_json);
        let messages = vec![
            mk_msg(1, MessageRole::User, "u"),
            mk_tool(2, &result),
            mk_msg(3, MessageRole::Assistant, "here"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "here");
        assert_eq!(snap.medias.len(), 1);
        assert_eq!(snap.medias[0].name, "foo.png");
    }

    #[test]
    fn ignores_old_turn_media() {
        let media_json = r#"[{"url":"/api/attachments/s/old.png","localPath":"/tmp/old.png","name":"old.png","mimeType":"image/png","sizeBytes":1,"kind":"image"}]"#;
        let result = format!("{}{}\nok", crate::agent::MEDIA_ITEMS_PREFIX, media_json);
        let messages = vec![
            mk_msg(1, MessageRole::User, "u1"),
            mk_tool(2, &result),
            mk_msg(3, MessageRole::Assistant, "old"),
            mk_msg(4, MessageRole::User, "u2"),
            mk_msg(5, MessageRole::Assistant, "new"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "new");
        assert!(snap.medias.is_empty(), "old turn media should be dropped");
    }
}
