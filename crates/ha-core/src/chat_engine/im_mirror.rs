//! GUI / HTTP → IM final-reply mirror — pushes the assistant's final
//! response to the IM attach (if any) for the session at the success
//! path of `run_chat_engine`. IM-driven and background turns skip this.
//!
//! With 1:1 attach, a session has at most one IM chat attached, so the
//! mirror is a single `Option<ImMirror>`.
//!
//! Live (per-frame) GUI → IM streaming is **not** done here — see
//! `docs/plans/review-followups.md` F-066 for the live-mirror follow-up.

use std::sync::Arc;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ParseMode, ReplyPayload};
use crate::chat_engine::stream_seq::ChatSource;

/// The single IM attach to mirror to, captured at turn start.
pub(crate) struct ImMirror {
    plugin: Arc<dyn ChannelPlugin>,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
}

/// Aggregate state held by `run_chat_engine` for a `Desktop` / `Http`
/// turn. Default is empty — IM-only or background turns end up here, as
/// do desktop turns on sessions without an IM attach.
#[derive(Default)]
pub(crate) struct ImMirrorState {
    mirror: Option<ImMirror>,
}

impl ImMirrorState {
    pub(crate) fn is_empty(&self) -> bool {
        self.mirror.is_none()
    }
}

/// Look up the session's IM attach (if any) and remember plugin + chat
/// coordinates for `finalize_im_mirrors`. No-op when the session has
/// no IM attach or when channel globals aren't initialised.
pub(crate) fn attach_im_mirrors(session_id: &str, source: ChatSource) -> ImMirrorState {
    if !matches!(source, ChatSource::Desktop | ChatSource::Http) {
        return ImMirrorState::default();
    }
    let Some(channel_db) = crate::globals::get_channel_db() else {
        return ImMirrorState::default();
    };
    let Some(registry) = crate::globals::get_channel_registry() else {
        return ImMirrorState::default();
    };

    let attach = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return ImMirrorState::default(),
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "get_conversation_by_session({}) failed: {}",
                session_id,
                e
            );
            return ImMirrorState::default();
        }
    };

    let channel_id_typed: crate::channel::types::ChannelId =
        match serde_json::from_value(serde_json::Value::String(attach.channel_id.clone())) {
            Ok(c) => c,
            Err(e) => {
                crate::app_warn!(
                    "channel",
                    "mirror",
                    "Unknown channel id {} on attach: {}",
                    attach.channel_id,
                    e
                );
                return ImMirrorState::default();
            }
        };

    let plugin = match registry.get_plugin(&channel_id_typed) {
        Some(p) => p,
        None => return ImMirrorState::default(),
    };

    ImMirrorState {
        mirror: Some(ImMirror {
            plugin: plugin.clone(),
            account_id: attach.account_id,
            chat_id: attach.chat_id,
            thread_id: attach.thread_id,
        }),
    }
}

/// Send `response` (markdown) to the collected mirror, if any. Plugin
/// errors are logged and swallowed; the chat engine has already
/// persisted the response, so a missed mirror is a missed echo not a
/// missed turn.
pub(crate) async fn finalize_im_mirrors(state: ImMirrorState, response: &str) {
    if let Some(mirror) = state.mirror {
        send_to_mirror(&mirror, response).await;
    }
}

async fn send_to_mirror(mirror: &ImMirror, response: &str) {
    let native_text = mirror.plugin.markdown_to_native(response);
    let chunks = mirror.plugin.chunk_message(&native_text);
    for (i, chunk) in chunks.iter().enumerate() {
        let payload = ReplyPayload {
            text: Some(chunk.clone()),
            thread_id: mirror.thread_id.clone(),
            parse_mode: Some(ParseMode::Html),
            ..ReplyPayload::text("")
        };
        if let Err(e) = mirror
            .plugin
            .send_message(&mirror.account_id, &mirror.chat_id, &payload)
            .await
        {
            crate::app_warn!(
                "channel",
                "mirror",
                "Mirror chunk {} to {}/{} failed: {}",
                i,
                mirror.account_id,
                mirror.chat_id,
                e
            );
        }
    }
}
