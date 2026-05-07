//! `channel:primary_changed` watcher — sends "you are now primary /
//! observing" system messages to IM chats whenever an attach row's
//! `is_primary` flips.
//!
//! Subscriber path:
//! 1. [`crate::channel::db::ChannelDB::attach_session`] /
//!    [`detach_session`] / [`set_primary`] / [`update_session`] emit
//!    `EVENT_CHANNEL_PRIMARY_CHANGED { sessionId }` after any DB write
//!    that toggles `is_primary`.
//! 2. This watcher subscribes to the global EventBus, looks up every
//!    current attach row for the affected session via
//!    [`list_attached`](ChannelDB::list_attached), and dispatches a
//!    single system message to each chat's plugin.
//!
//! Each account has a `notify_primary_changes` toggle (default `true`,
//! Phase B4) so noisy multi-attach setups can mute these notices on a
//! per-account basis.
//!
//! The notice text is intentionally hard-coded English-with-emoji.
//! IM servers don't carry per-recipient locale, so attempting to
//! localize on the backend would either pick the wrong language or
//! force every account to declare one. A future enhancement could read
//! `ChannelAccountConfig.locale` if we add the field.
//!
//! [`set_primary`]: ChannelDB::set_primary
//! [`detach_session`]: ChannelDB::detach_session
//! [`update_session`]: ChannelDB::update_session

use std::sync::Arc;

use crate::channel::db::{ChannelConversation, ChannelDB, EVENT_CHANNEL_PRIMARY_CHANGED};
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{ParseMode, ReplyPayload};

const PROMOTED_TEXT: &str =
    "📡 You are now the primary endpoint for this session — agent replies will be delivered here.";

const DEMOTED_TEXT: &str =
    "ℹ️ Another endpoint is now the primary recipient for this session; you are now observing.";

/// Spawn the EventBus subscriber that turns `channel:primary_changed`
/// events into per-attach system messages. No-op when the event bus
/// hasn't been initialised yet (server / acp paths bring the bus up
/// before this is called, so in practice the early return only fires
/// in unit-test contexts).
pub fn spawn_channel_primary_watcher(channel_db: Arc<ChannelDB>, registry: Arc<ChannelRegistry>) {
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
                        "primary_watcher",
                        "Lagged {} EventBus events; some primary-change notices may be missed",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

            if event.name != EVENT_CHANNEL_PRIMARY_CHANGED {
                continue;
            }

            let Some(session_id) = event
                .payload
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(str::to_string)
            else {
                app_warn!(
                    "channel",
                    "primary_watcher",
                    "{} payload missing sessionId: {}",
                    EVENT_CHANNEL_PRIMARY_CHANGED,
                    event.payload
                );
                continue;
            };

            let attaches: Vec<ChannelConversation> = match channel_db.list_attached(&session_id) {
                Ok(v) => v,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "primary_watcher",
                        "list_attached({}) failed: {}",
                        session_id,
                        e
                    );
                    continue;
                }
            };
            if attaches.is_empty() {
                continue;
            }

            let store = crate::config::cached_config();
            let mut sends: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(attaches.len());

            for conv in attaches.into_iter() {
                let account = match store.channels.find_account(&conv.account_id) {
                    Some(c) if c.notify_primary_changes => c.clone(),
                    _ => continue,
                };

                let channel_id: crate::channel::types::ChannelId = match serde_json::from_value(
                    serde_json::Value::String(conv.channel_id.clone()),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "primary_watcher",
                            "Unknown channel_id {} on attach: {}",
                            conv.channel_id,
                            e
                        );
                        continue;
                    }
                };

                let plugin = match registry.get_plugin(&channel_id) {
                    Some(p) => p,
                    None => continue,
                };

                let text = if conv.is_primary {
                    PROMOTED_TEXT
                } else {
                    DEMOTED_TEXT
                };
                let payload = ReplyPayload {
                    text: Some(plugin.markdown_to_native(text)),
                    thread_id: conv.thread_id.clone(),
                    parse_mode: Some(ParseMode::Html),
                    ..ReplyPayload::text("")
                };

                let chat_id = conv.chat_id.clone();
                let channel_id_str = conv.channel_id.clone();
                let account_id = account.id.clone();
                sends.push(tokio::spawn(async move {
                    if let Err(e) = plugin.send_message(&account_id, &chat_id, &payload).await {
                        app_warn!(
                            "channel",
                            "primary_watcher",
                            "send_message failed for {}/{}: {}",
                            channel_id_str,
                            chat_id,
                            e
                        );
                    }
                }));
            }

            for h in sends {
                let _ = h.await;
            }
        }
    });
}
