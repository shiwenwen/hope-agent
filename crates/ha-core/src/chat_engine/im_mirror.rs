//! GUI / HTTP → IM final-reply mirror — pushes the assistant's final
//! response to every primary IM attach for the session at the success
//! path of `run_chat_engine`. IM-driven and background turns skip this.
//!
//! Live (per-frame) GUI → IM streaming is **not** done here. Phase B7's
//! initial follow-up wired up secondary `ChannelStreamSink` + per-mirror
//! `spawn_channel_stream_task`, but main's split-streaming /
//! per-round-preview rework (`ImReplyMode` / `RoundTextAccumulator`)
//! reshaped that subsystem in incompatible ways. Re-implementing the
//! live mirror on top of the new state machine is a follow-up; for now
//! the must-have path — "the IM user sees the final answer" — runs
//! through this module unchanged.

use std::sync::Arc;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ParseMode, ReplyPayload};
use crate::chat_engine::stream_seq::ChatSource;

/// One mirror tied to a single primary attach row.
pub(crate) struct ImMirror {
    plugin: Arc<dyn ChannelPlugin>,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
}

/// Aggregate state held by `run_chat_engine` for a `Desktop` / `Http`
/// turn. Default is empty — IM-only or background turns end up here.
#[derive(Default)]
pub(crate) struct ImMirrorState {
    mirrors: Vec<ImMirror>,
}

impl ImMirrorState {
    pub(crate) fn is_empty(&self) -> bool {
        self.mirrors.is_empty()
    }
}

/// Walk the session's primary IM attaches and remember plugin + chat
/// coordinates for `finalize_im_mirrors`. No-op when the session has
/// no IM attaches (the EXISTS probe short-circuits before any heavier
/// work) or when channel globals aren't initialised.
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

    match channel_db.has_attached(session_id) {
        Ok(false) => return ImMirrorState::default(),
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "has_attached({}) failed: {}",
                session_id,
                e
            );
            return ImMirrorState::default();
        }
        Ok(true) => {}
    }

    let attaches = match channel_db.list_attached(session_id) {
        Ok(v) => v,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "list_attached({}) failed: {}",
                session_id,
                e
            );
            return ImMirrorState::default();
        }
    };

    let mut state = ImMirrorState::default();
    for attach in attaches.iter() {
        if !attach.is_primary {
            continue;
        }

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
                    continue;
                }
            };

        let plugin = match registry.get_plugin(&channel_id_typed) {
            Some(p) => p,
            None => continue,
        };

        state.mirrors.push(ImMirror {
            plugin: plugin.clone(),
            account_id: attach.account_id.clone(),
            chat_id: attach.chat_id.clone(),
            thread_id: attach.thread_id.clone(),
        });
    }
    state
}

/// Send `response` (markdown) to every collected mirror in parallel.
/// Per-mirror plugin errors are logged and skipped; the chat engine
/// has already persisted the response, so a missed mirror is a
/// missed echo not a missed turn.
pub(crate) async fn finalize_im_mirrors(state: ImMirrorState, response: &str) {
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(state.mirrors.len());
    for mirror in state.mirrors {
        let response = response.to_string();
        tasks.push(tokio::spawn(async move {
            send_to_mirror(&mirror, &response).await;
        }));
    }
    for t in tasks {
        let _ = t.await;
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
