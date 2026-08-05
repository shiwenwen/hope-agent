//! `channel:session_evicted` watcher — sends a "this chat has been
//! taken over" notice to any IM chat that was just evicted from a
//! session because another chat attached to the same session_id.
//!
//! Subscriber path:
//! 1. [`ha_core::channel::db::ChannelDB::attach_session`] /
//!    [`ha_core::channel::db::ChannelDB::update_session`] emit one
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

use super::pipeline::DeliveryTarget;
use super::provider_lane::{reserve_provider_lane, ProviderMutationGuard, ProviderMutationOutcome};
use ha_core::channel::db::{payload_keys, EVENT_CHANNEL_SESSION_EVICTED};
use ha_core::channel::registry::ChannelRegistry;
use ha_core::channel::types::{ChatType, ParseMode, ReplyPayload};

/// Spawn the EventBus subscriber that turns `channel:session_evicted`
/// events into a system message on the evicted chat. No-op when the
/// event bus hasn't been initialised yet (server / acp paths bring the
/// bus up before this is called, so in practice the early return only
/// fires in unit-test contexts).
pub fn spawn_channel_eviction_watcher(registry: Arc<ChannelRegistry>) {
    let Some(bus) = ha_core::globals::get_event_bus() else {
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
            // active. Any approval captured on the evicted attach can no longer be
            // answered here — deny only those exact stale identities (so a delayed
            // event cannot reject replacement/core-only approvals) and clear their
            // corresponding text-reply entries.
            // Runs BEFORE the `notify_session_eviction` gate below: the cleanup is
            // unconditional; only the user-facing "taken over" notice is gated.
            if let Some(session_id) = payload
                .get(payload_keys::SESSION_ID)
                .and_then(|v| v.as_str())
            {
                // ask_user has no tool-side approval to deny. Remove only
                // entries whose captured attach id is no longer live, so a
                // delayed eviction event cannot clear a replacement chat's
                // newly registered prompt for the same session.
                crate::channel::worker::ask_user::drop_stale_pending_for_session(session_id).await;
                let core_pending =
                    ha_core::tools::pending_request_ids_for_session(session_id).await;
                let cleanup = crate::channel::worker::approval::take_stale_pending_for_session(
                    session_id,
                    &core_pending,
                )
                .await;
                for request_id in &cleanup.request_ids {
                    let _ = ha_core::tools::submit_approval_response(
                        request_id,
                        ha_core::tools::ApprovalResponse::Deny,
                        ha_core::tools::ApprovalResolutionSource::Eviction,
                    )
                    .await;
                }
                if !cleanup.request_ids.is_empty() {
                    app_info!(
                        "channel",
                        "eviction_watcher",
                        "denied {} pending approval(s) on evicted chat for session {}",
                        cleanup.request_ids.len(),
                        session_id
                    );
                }
                if cleanup.skipped_without_identity > 0 {
                    app_info!(
                        "channel",
                        "eviction_watcher",
                        "left {} core approval(s) without matching IM attach identity to owner/timeout for session {}",
                        cleanup.skipped_without_identity,
                        session_id
                    );
                }
            }

            let store = ha_core::config::cached_config();
            let account = match store.channels.find_account(account_id) {
                Some(c) if c.notify_session_eviction => c.clone(),
                _ => continue,
            };

            let channel_id =
                match ha_core::channel::types::ChannelId::from_storage_str(channel_id_str) {
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
            let evicted_text = ha_core::i18n::localized_backend_message(
                ha_core::i18n::BackendMessage::ChannelSessionEvicted,
                ha_core::i18n::effective_ui_locale(&store),
            );

            // Reserve before detaching the send task. This eviction notice is
            // part of the same physical-provider sequence as any old mirror
            // cleanup, so it cannot appear and then be overwritten by a late
            // edit/push from the evicted generation.
            let lane_chat_type = ChatType::Dm;
            let lane_target = DeliveryTarget {
                account_id: &account.id,
                chat_id,
                chat_type: &lane_chat_type,
                thread_id: thread_id.as_deref(),
                reply_to_message_id: None,
                recipient_user_id: None,
                recipient_tenant_id: None,
            };
            let provider_lane = reserve_provider_lane(&lane_target);
            let provider_guard = ProviderMutationGuard::new(
                provider_lane.waiter(),
                provider_lane.task_hold(),
                Arc::new(|| true),
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
            let mutation = async move {
                plugin
                    .send_message(&account_id_owned, &chat_id_owned, &reply)
                    .await
            };
            let ticket = provider_guard.submit(mutation);
            tokio::spawn(async move {
                let _provider_lane = provider_lane;
                match ticket.wait().await {
                    ProviderMutationOutcome::Completed(Ok(_)) => {}
                    ProviderMutationOutcome::Completed(Err(error)) => app_warn!(
                        "channel",
                        "eviction_watcher",
                        "send_message failed for {}: {}",
                        channel_id_owned,
                        error
                    ),
                    ProviderMutationOutcome::Invalid => app_warn!(
                        "channel",
                        "eviction_watcher",
                        "eviction notice provider target became invalid for {}",
                        channel_id_owned
                    ),
                    ProviderMutationOutcome::TaskFailed => app_warn!(
                        "channel",
                        "eviction_watcher",
                        "eviction notice provider task failed for {}",
                        channel_id_owned
                    ),
                }
            });
        }
    });
}
