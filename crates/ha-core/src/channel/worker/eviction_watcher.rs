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
//! The notice text is intentionally hard-coded English-with-emoji.
//! IM servers don't carry per-recipient locale, so attempting to
//! localize on the backend would either pick the wrong language or
//! force every account to declare one. A future enhancement could read
//! `ChannelAccountConfig.locale` if we add the field.

use std::sync::Arc;

use crate::channel::db::{payload_keys, EVENT_CHANNEL_SESSION_EVICTED};
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{ParseMode, ReplyPayload};

const EVICTED_TEXT: &str = "📢 This chat has been taken over by another endpoint. \
                            You've left the previous session — \
                            send a new message to start a fresh one.";

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

            let store = crate::config::cached_config();
            let account = match store.channels.find_account(account_id) {
                Some(c) if c.notify_session_eviction => c.clone(),
                _ => continue,
            };

            let channel_id: crate::channel::types::ChannelId =
                match serde_json::from_value(serde_json::Value::String(channel_id_str.to_string()))
                {
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

            let reply = ReplyPayload {
                text: Some(plugin.markdown_to_native(EVICTED_TEXT)),
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
