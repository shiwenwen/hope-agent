//! Shared cleanup for "Hope generated the reply, but IM delivery failed".
//!
//! Three sites end with the same 8-step sequence when a [`DeliveryReport`]
//! comes back non-success: warn (redacted first failure) + append an
//! `error_event` message to the session + emit `channel:message_update`.
//! Only the caller-context prefix on the warn line and the exact user-facing
//! notice differ; everything downstream is fixed policy (redaction,
//! [`ChatSource::Channel`] source, sidebar refresh event).
//!
//! Keeping it here means changes to the warn category, redaction, error
//! event source, or emit topic land in one place — the three callers
//! (`im_mirror::finalize_im_live_mirror`, `channel::attach_sync::LateMirror`
//! and the main IM worker's post-`deliver_rounds` branch) stay in lockstep.

use std::sync::Arc;

use ha_core::session::SessionDB;

use super::dispatcher::{emit_channel_update, DeliveryReport};

pub(crate) async fn report_delivery_failure(
    session_db: &Arc<SessionDB>,
    session_id: &str,
    warn_context: &str,
    notice_body: &'static str,
    report: &DeliveryReport,
) {
    let failure = report
        .failures
        .first()
        .cloned()
        .unwrap_or_else(|| "delivery result was incomplete".to_string());
    app_warn!(
        "channel",
        "delivery_failed",
        "{} (attempted={}, succeeded={}): {}",
        warn_context,
        report.attempted,
        report.succeeded,
        ha_core::logging::redact_sensitive(&failure),
    );
    let notice = ha_core::session::NewMessage::error_event(notice_body)
        .with_source(ha_core::chat_engine::ChatSource::Channel);
    let session_id_owned = session_id.to_string();
    let update_session_id = session_id_owned.clone();
    let _ = session_db
        .run(move |db| db.append_message(&session_id_owned, &notice))
        .await;
    emit_channel_update(&update_session_id);
}
