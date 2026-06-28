//! `channel:session_evicted` watcher — sends a "this chat has been
//! taken over" notice to any IM chat that was just evicted from a
//! session because another chat attached to the same session_id.
//!
//! Subscriber path:
//! 1. [`crate::channel::db::ChannelDB::attach_session`] /
//!    [`crate::channel::db::ChannelDB::update_session`] emit one
//!    `EVENT_CHANNEL_SESSION_EVICTED` event per evicted chat after
//!    physically deleting that chat's attach row.
//! 2. This watcher subscribes to the global EventBus and dispatches a
//!    single system message to the evicted chat's plugin. The
//!    `notify_session_eviction` toggle on the affected account (default
//!    `true`) can mute the notice.
//!
//! Notice text follows `AppConfig.language` through `ha_core::i18n`.
//! IM servers do not carry per-recipient locale today, so this is a global
//! process preference rather than per-chat language negotiation.

use std::sync::Arc;

use crate::channel::db::{payload_keys, EVENT_CHANNEL_SESSION_EVICTED};
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{ParseMode, ReplyPayload};

/// Spawn the EventBus subscriber that turns `channel:session_evicted`
/// events into a system message on the evicted chat. No-op when the
/// event bus hasn't been initialised yet (server / acp paths bring the
/// bus up before this is called, so in practice the early return only
/// fires in unit-test contexts).
pub fn spawn_channel_eviction_watcher(registry: Arc<ChannelRegistry>) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app_warn!(
                        "channel",
                        "eviction_watcher",
                        "Lagged {} EventBus events; some eviction notices may be missed",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

            if event.name != EVENT_CHANNEL_SESSION_EVICTED {
                continue;
            }

            let payload = &event.payload;
            let Some(channel_id_str) = payload
                .get(payload_keys::CHANNEL_ID)
                .and_then(|v| v.as_str())
            else {
                app_warn!(
                    "channel",
                    "eviction_watcher",
                    "{} payload missing channelId: {}",
                    EVENT_CHANNEL_SESSION_EVICTED,
                    payload
                );
                continue;
            };
            let Some(account_id) = payload
                .get(payload_keys::ACCOUNT_ID)
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let Some(chat_id) = payload.get(payload_keys::CHAT_ID).and_then(|v| v.as_str()) else {
                continue;
            };
            let thread_id = payload
                .get(payload_keys::THREAD_ID)
                .and_then(|v| v.as_str())
                .map(str::to_string);

            // G5 (SURFACE-4): the chat was taken over while its session stayed
            // active. Any approval prompted on this chat can no longer be answered
            // here — deny each pending approval (so the blocked tool turn unblocks
            // and every surface dismisses) and clear the chat's text-reply stack.
            // Runs BEFORE the `notify_session_eviction` gate below: the cleanup is
            // unconditional; only the user-facing "taken over" notice is gated.
            if let Some(session_id) = payload
                .get(payload_keys::SESSION_ID)
                .and_then(|v| v.as_str())
            {
                let pending =
                    crate::tools::approval::pending_request_ids_for_session(session_id).await;
                for request_id in &pending {
                    let _ = crate::tools::approval::submit_approval_response(
                        request_id,
                        crate::tools::approval::ApprovalResponse::Deny,
                        crate::tools::approval::ApprovalResolutionSource::Eviction,
                    )
                    .await;
                }
                if !pending.is_empty() {
                    app_info!(
                        "channel",
                        "eviction_watcher",
                        "denied {} pending approval(s) on evicted chat for session {}",
                        pending.len(),
                        session_id
                    );
                }
            }
            crate::channel::worker::approval::drop_pending_for_chat(account_id, chat_id).await;

            let store = crate::config::cached_config();
            let account = match store.channels.find_account(account_id) {
                Some(c) if c.notify_session_eviction => c.clone(),
                _ => continue,
            };

            let channel_id =
                match crate::channel::types::ChannelId::from_storage_str(channel_id_str) {
                    Ok(c) => c,
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "eviction_watcher",
                            "Unknown channel_id {} on eviction: {}",
                            channel_id_str,
                            e
                        );
                        continue;
                    }
                };

            let plugin = match registry.get_plugin(&channel_id) {
                Some(p) => p.clone(),
                None => continue,
            };
            let evicted_text = crate::i18n::localized_backend_message(
                crate::i18n::BackendMessage::ChannelSessionEvicted,
                crate::i18n::effective_ui_locale(&store),
            );

            let reply = ReplyPayload {
                text: Some(plugin.markdown_to_native(evicted_text)),
                thread_id,
                parse_mode: Some(ParseMode::Html),
                ..ReplyPayload::text("")
            };

            let chat_id_owned = chat_id.to_string();
            let account_id_owned = account.id.clone();
            let channel_id_owned = channel_id_str.to_string();
            tokio::spawn(async move {
                if let Err(e) = plugin
                    .send_message(&account_id_owned, &chat_id_owned, &reply)
                    .await
                {
                    app_warn!(
                        "channel",
                        "eviction_watcher",
                        "send_message failed for {}/{}: {}",
                        channel_id_owned,
                        chat_id_owned,
                        e
                    );
                }
            });
        }
    });
}
