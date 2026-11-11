//! Owner-paired personal-assistant transport state.
//!
//! This ledger is independent from ordinary channel accounts and 1:1 attaches.
//! Shells may change authority only after authenticating the owner. Channel
//! workers consume platform identities and immutable generations, never model
//! assertions about who sent a message.

mod delivery;
mod notification_policy;
mod pairing_delivery;
mod runner;
pub(crate) mod scoped_tools;
mod store;
mod view;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

pub use notification_policy::{notification_policy, DeliveryDeferredReason, NotificationPolicy};
pub use runner::process_pending_commands;
pub(crate) use runner::SYSTEM_PROMPT;
pub(crate) use store::migrate;
pub(crate) use view::authorize_assistant_im_cancel_with_conn;
pub(crate) use view::authorize_source_turn;
pub(crate) use view::revalidate_assistant_im_dispatch;
pub(crate) use view::validate_admitted_assistant_im_with_conn;
pub use view::{AssistantImContext, AssistantImTarget};

pub const CHANGED_EVENT: &str = "personal_assistant_im:changed";
pub const PAIR_COMMAND: &str = "/ha_pair ";
pub const ACTIVATE_PREFIX: &str = "ha_private_activate:";
pub const MAX_MESSAGE_BYTES: usize = 8_192;
pub const MAX_REPLY_CHARS: usize = 3_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotPurpose {
    Ordinary,
    PersonalAssistant,
}

impl BotPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::PersonalAssistant => "personal_assistant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    Unpaired,
    AwaitingCode,
    AwaitingConfirmation,
    AwaitingActivation,
    Active,
}

/// Deliberately excludes credential references, pairing hashes and raw updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub connection_id: String,
    pub bot_id: i64,
    pub bot_username: String,
    pub generation: i64,
    pub state: PairingState,
    pub user_id: Option<i64>,
    pub chat_id: Option<i64>,
    pub notifications_paused: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connection {
    pub view: ConnectionView,
    pub credential_ref: String,
    pub pair_hash: Option<String>,
    pub pair_expires_at: i64,
    pub pair_attempts: u32,
    pub activation_hash: Option<String>,
    pub activation_expires_at: i64,
    pub cursor: i64,
    pub cursor_epoch: i64,
    pub last_update_at: i64,
    pub last_poll_at: i64,
    #[serde(default)]
    pub last_expiry_event_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionGrant {
    pub session_id: String,
    pub revision: i64,
    pub query_status: bool,
    pub delegate: bool,
    pub stop: bool,
    pub completion_notice: bool,
    pub expires_at: i64,
}

/// Runtime metadata is never returned through a public owner/status endpoint.
#[derive(Clone)]
pub struct PollLease {
    pub connection_id: String,
    pub bot_id: i64,
    pub generation: i64,
    pub credential_ref: String,
    pub cursor: i64,
    pub cursor_epoch: i64,
}

/// Constructed only by the dedicated raw Telegram update decoder.
pub struct TelegramInput {
    pub update_id: i64,
    pub sent_at: Option<i64>,
    pub user_id: i64,
    pub chat_id: i64,
    pub kind: TelegramInputKind,
}

pub enum TelegramInputKind {
    PairCode(String),
    Activate(String),
    Message(String),
    Ignore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedCommand {
    pub request_id: String,
    pub connection_id: String,
    pub generation: i64,
    pub text: String,
    pub grant_revisions: std::collections::BTreeMap<String, i64>,
    pub global_stop_epoch: u64,
    pub home_stop_epoch: u64,
    pub target_stop_epochs: std::collections::BTreeMap<String, u64>,
    pub source_view_id: Option<String>,
    pub view_stop_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTerminal {
    Completed,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressOutcome {
    Ignored,
    Duplicate,
    PairingCandidate,
    Activated,
    Queued,
    Revoked,
    Paused,
}

/// The clear code is returned exactly once to the authenticated owner.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub connection_id: String,
    pub generation: i64,
    pub code: String,
    pub expires_at: i64,
}

/// One bounded, deterministic operation is permitted while pairing. The
/// handle is sent only to the candidate confirmed in the owner interface.
pub struct ActivationPrompt {
    pub lease: PollLease,
    pub chat_id: i64,
    pub callback_data: String,
}

/// Claimed by the Primary after authority revalidation, never by a GUI shell.
pub struct PreparedActivation {
    pub id: String,
    pub prompt: ActivationPrompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Submitting,
    Accepted,
    Unknown,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReceipt {
    pub id: String,
    pub request_id: String,
    pub state: DeliveryState,
    pub message_id: Option<i64>,
    pub created_at: i64,
    pub deferred_reason: Option<DeliveryDeferredReason>,
}

/// Created by the ledger after rechecking the latest grant and pairing. The
/// adapter must make one provider mutation and never automatically retry it.
pub struct PreparedDelivery {
    pub id: String,
    pub lease: PollLease,
    pub chat_id: i64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantImError {
    IdentityInUse,
    Unavailable,
    StaleGeneration,
    PermissionDenied,
    InvalidInput,
}

impl std::fmt::Display for AssistantImError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::IdentityInUse => "assistant_im_identity_in_use",
            Self::Unavailable => "assistant_im_unavailable",
            Self::StaleGeneration => "assistant_im_stale_generation",
            Self::PermissionDenied => "assistant_im_permission_denied",
            Self::InvalidInput => "assistant_im_invalid_input",
        })
    }
}

impl std::error::Error for AssistantImError {}

fn changed() {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(CHANGED_EVENT, serde_json::json!({}));
    }
}
