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
//! Desktop / HTTP turns that are already in flight when the attach happens
//! get a late IM mirror registered through `SinkRegistry`; it streams any
//! remaining deltas and replaces the preview with the complete final answer
//! when the turn finishes.

use std::sync::Arc;
use std::time::Duration;

use crate::attachments::MediaItem;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ChannelAccountConfig, ChatType, ImReplyMode};
use crate::channel::worker::pipeline::{
    await_stream_pipeline, deliver_full_response, spawn_stream_pipeline, DeliveryTarget,
    StreamPipeline,
};
use crate::channel::worker::{deliver_media_to_chat, send_text_chunks};
use crate::chat_engine::im_mirror::{
    attach_still_matches, guarded_mirror_sink, try_claim_mirror_attach, MirrorAttachGuard,
};
use crate::chat_engine::sink_registry::{sink_registry, SinkHandle};
use crate::chat_engine::stream_seq::ChatSource;
use crate::session::{ChatTurnStatus, MessageRole, SessionDB};

const CATCHUP_WINDOW: u32 = 50;
const ACTIVE_TURN_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachKind {
    SessionAttach,
    Handover,
}

/// Read the latest completed turn from the session and deliver assistant
/// final text + media to the chat as a one-shot `Final`-mode delivery.
///
/// Skips silently when the session has no assistant text and no media yet.
/// If a desktop / HTTP turn is already active, registers a late mirror so
/// the IM chat receives the rest of the stream plus the complete final
/// answer for that turn.
pub async fn deliver_attach_catchup(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) {
    deliver_attach_catchup_inner(
        plugin,
        account,
        session_id,
        chat_id,
        thread_id,
        AttachKind::SessionAttach,
    )
    .await;
}

/// Same catch-up path as [`deliver_attach_catchup`], with a GUI/HTTP
/// handover notice sent into the receiving IM chat.
pub async fn deliver_handover_catchup(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) {
    deliver_attach_catchup_inner(
        plugin,
        account,
        session_id,
        chat_id,
        thread_id,
        AttachKind::Handover,
    )
    .await;
}

async fn deliver_attach_catchup_inner(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    kind: AttachKind,
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

    let active_turn = active_desktop_or_http_turn(session_id);
    if kind == AttachKind::Handover {
        send_handover_notice(plugin, account, chat_id, thread_id, active_turn.is_some()).await;
    }

    if let Some(active) = active_turn {
        if start_late_mirror(
            plugin,
            account,
            session_db.clone(),
            session_id,
            chat_id,
            thread_id,
            active,
        )
        .await
        {
            return;
        }
    }

    // Only need the last turn — 50 rows is a generous bound that covers
    // even very long thinking + tool_result + assistant chains. The
    // helper aligns the window to the latest `user` row so we always
    // have a clean turn boundary to slice from.
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
        send_text_chunks(plugin, &target, &snapshot.text, None, &[]).await;
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
}

fn active_desktop_or_http_turn(
    session_id: &str,
) -> Option<crate::chat_engine::active_turn::ActiveTurnSnapshot> {
    crate::chat_engine::active_turn::current(session_id)
        .filter(|active| matches!(active.source, ChatSource::Desktop | ChatSource::Http))
}

async fn send_handover_notice(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    chat_id: &str,
    thread_id: Option<&str>,
    in_flight: bool,
) {
    let text = if in_flight {
        "📨 Session handed over from Hope Agent. A reply is already in progress; live updates will continue here."
    } else {
        "📨 Session handed over from Hope Agent."
    };
    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id,
        thread_id,
        reply_to_message_id: None,
    };
    send_text_chunks(plugin, &target, text, None, &[]).await;
}

async fn start_late_mirror(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_db: Arc<SessionDB>,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    active: crate::chat_engine::active_turn::ActiveTurnSnapshot,
) -> bool {
    let channel_db = match crate::globals::get_channel_db() {
        Some(db) => db,
        None => return false,
    };
    let attach = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(conv)) => conv,
        Ok(None) => return false,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "attach_sync",
                "get_conversation_by_session({}) failed before late mirror: {}",
                session_id,
                e
            );
            return false;
        }
    };
    if attach.account_id != account.id
        || attach.chat_id != chat_id
        || attach.thread_id.as_deref() != thread_id
    {
        return false;
    }
    let Some(mirror_guard) = try_claim_mirror_attach(session_id, attach.id) else {
        return true;
    };

    if let Some(quote) = latest_user_quote(&session_db, session_id, active.source) {
        let target = DeliveryTarget {
            account_id: &account.id,
            chat_id,
            thread_id,
            reply_to_message_id: None,
        };
        send_text_chunks(plugin, &target, quote.trim_end(), None, &[]).await;
    }

    let chat_type = ChatType::from_lowercase(&attach.chat_type);
    let mut mirror_account = account.clone();
    if matches!(mirror_account.im_reply_mode(), ImReplyMode::Split) {
        mirror_account.set_im_reply_mode(ImReplyMode::Preview);
    }

    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id,
        thread_id,
        reply_to_message_id: None,
    };
    let pipeline = spawn_stream_pipeline(
        plugin,
        &mirror_account,
        &chat_type,
        session_id,
        &target,
        false,
    );
    let guarded_sink = guarded_mirror_sink(
        session_id.to_string(),
        attach.id,
        pipeline.event_sink.clone(),
    );
    let sink_handle = sink_registry().attach(session_id.to_string(), guarded_sink);

    let mirror = LateMirror {
        sink_handle,
        _mirror_guard: mirror_guard,
        pipeline,
        plugin: plugin.clone(),
        account: mirror_account,
        session_db,
        session_id: session_id.to_string(),
        turn_id: active.turn_id,
        attach_id: attach.id,
        chat_id: chat_id.to_string(),
        thread_id: thread_id.map(str::to_string),
    };
    tokio::spawn(async move {
        mirror.run().await;
    });
    true
}

struct LateMirror {
    sink_handle: SinkHandle,
    _mirror_guard: MirrorAttachGuard,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    account: ChannelAccountConfig,
    session_db: Arc<SessionDB>,
    session_id: String,
    turn_id: String,
    attach_id: i64,
    chat_id: String,
    thread_id: Option<String>,
}

impl LateMirror {
    async fn run(self) {
        let LateMirror {
            sink_handle,
            _mirror_guard,
            pipeline,
            plugin,
            account,
            session_db,
            session_id,
            turn_id,
            attach_id,
            chat_id,
            thread_id,
        } = self;

        let mut detached = false;
        loop {
            tokio::time::sleep(ACTIVE_TURN_POLL_INTERVAL).await;
            if !attach_still_matches(&session_id, attach_id) {
                detached = true;
                break;
            }
            match crate::chat_engine::active_turn::current(&session_id) {
                Some(active) if active.turn_id == turn_id => continue,
                _ => break,
            }
        }

        drop(sink_handle);
        let outcome = await_stream_pipeline(pipeline).await;
        if detached || !attach_still_matches(&session_id, attach_id) {
            return;
        }

        let target = DeliveryTarget {
            account_id: &account.id,
            chat_id: &chat_id,
            thread_id: thread_id.as_deref(),
            reply_to_message_id: None,
        };

        let turn = session_db.get_chat_turn(&turn_id).ok().flatten();
        let snapshot = turn
            .as_ref()
            .and_then(|t| t.user_message_id)
            .and_then(|user_id| turn_snapshot_after_user(&session_db, &session_id, user_id));

        if let Some(snapshot) = snapshot {
            let metrics =
                deliver_full_response(&plugin, &target, &outcome, &snapshot.text, &snapshot.medias)
                    .await;
            crate::app_info!(
                "channel",
                "attach_sync",
                "Delivered late handover mirror for session {} turn {} (text_chars={}, media={})",
                session_id,
                turn_id,
                metrics.text_chars,
                metrics.media_count,
            );
            return;
        }

        if let Some(turn) = turn {
            if matches!(turn.status, ChatTurnStatus::Failed) {
                let body = turn
                    .error
                    .as_deref()
                    .map(|e| format!("⚠️ Reply failed: {}", e))
                    .unwrap_or_else(|| "⚠️ Reply failed.".to_string());
                send_text_chunks(&plugin, &target, &body, None, &[]).await;
            }
        }
    }
}

fn latest_user_quote(
    session_db: &SessionDB,
    session_id: &str,
    source: ChatSource,
) -> Option<String> {
    let (messages, _total, _has_more) = session_db
        .load_session_messages_latest(session_id, CATCHUP_WINDOW)
        .ok()?;
    let user = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::User))?;
    crate::chat_engine::quote::build_user_quote_prefix(Some(
        &crate::chat_engine::quote::LastUserView {
            source: source.as_str(),
            text: &user.content,
            attachment_count: user_attachment_count(user.attachments_meta.as_deref()),
        },
    ))
}

fn user_attachment_count(meta: Option<&str>) -> usize {
    let Some(meta) = meta else { return 0 };
    match serde_json::from_str::<serde_json::Value>(meta) {
        Ok(serde_json::Value::Array(items)) => items.len(),
        _ => 0,
    }
}

fn turn_snapshot_after_user(
    session_db: &SessionDB,
    session_id: &str,
    user_message_id: i64,
) -> Option<TurnSnapshot> {
    let (messages, _has_more) = session_db
        .load_session_messages_after(session_id, user_message_id, 200)
        .ok()?;
    turn_snapshot_from_slice(&messages)
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
    turn_snapshot_from_slice(turn)
}

fn turn_snapshot_from_slice(messages: &[crate::session::SessionMessage]) -> Option<TurnSnapshot> {
    if messages.is_empty() {
        return None;
    }

    // Take the very last assistant row's content as the final answer.
    // Earlier `text_block` rows in the same turn are intermediate
    // narration that already streamed (and would have been delivered to
    // the IM live in `split` mode on a normal turn) — replaying them
    // would double-print to a user who's just attaching, so we keep it
    // simple and align with `ImReplyMode::Final` semantics.
    let text = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // Collect every tool result's media in turn order. Reuses
    // `agent::events::extract_media_items` so the parsing rules track
    // whatever the tool-event side emits (`__MEDIA_ITEMS__<json>\n…`).
    let mut medias: Vec<MediaItem> = Vec::new();
    for m in messages {
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
