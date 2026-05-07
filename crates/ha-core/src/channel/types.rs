use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Channel ID ───────────────────────────────────────────────────
// Enum variants ordered to match the canonical channel display order.

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelId {
    Telegram,
    #[serde(rename = "wechat")]
    WeChat,
    #[serde(rename = "whatsapp")]
    WhatsApp,
    Discord,
    Irc,
    #[serde(rename = "googlechat")]
    GoogleChat,
    Slack,
    Signal,
    #[serde(rename = "imessage")]
    IMessage,
    Line,
    Feishu,
    #[serde(rename = "qqbot")]
    QqBot,
    /// Extension channels not in the built-in list.
    #[serde(untagged)]
    Custom(String),
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelId::Telegram => write!(f, "telegram"),
            ChannelId::WeChat => write!(f, "wechat"),
            ChannelId::WhatsApp => write!(f, "whatsapp"),
            ChannelId::Discord => write!(f, "discord"),
            ChannelId::Irc => write!(f, "irc"),
            ChannelId::GoogleChat => write!(f, "googlechat"),
            ChannelId::Slack => write!(f, "slack"),
            ChannelId::Signal => write!(f, "signal"),
            ChannelId::IMessage => write!(f, "imessage"),
            ChannelId::Line => write!(f, "line"),
            ChannelId::Feishu => write!(f, "feishu"),
            ChannelId::QqBot => write!(f, "qqbot"),
            ChannelId::Custom(s) => write!(f, "{}", s),
        }
    }
}

// ── Chat Type ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatType {
    Dm,
    Group,
    Forum,
    Channel,
}

impl ChatType {
    /// Parse the lowercased string form persisted in
    /// `channel_conversations.chat_type` / surfaced from Tauri / HTTP
    /// payloads. Unknown values fall back to `Dm` — the conservative
    /// default for inbound resolution since solo chats are the only
    /// safe assumption when metadata is missing.
    pub fn from_lowercase(s: &str) -> Self {
        match s {
            "group" => Self::Group,
            "forum" => Self::Forum,
            "channel" => Self::Channel,
            _ => Self::Dm,
        }
    }
}

// ── Media Type ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Photo,
    Video,
    Audio,
    Document,
    Sticker,
    Voice,
    Animation,
}

// ── DM Policy ────────────────────────────────────────────────────
// Direct-message access policy per channel account.

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DmPolicy {
    #[default]
    Open,
    Allowlist,
    Pairing,
}

// ── Group Policy ─────────────────────────────────────────────────
// Group-message access policy per channel account.

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupPolicy {
    /// Groups bypass allowlist check, only mention-gating applies
    #[default]
    Open,
    /// Only allow groups explicitly listed in `groups` config
    Allowlist,
    /// Block all group messages entirely
    Disabled,
}

// ── Telegram Group Config ────────────────────────────────────────
// Per-group configuration for Telegram chats and forums.

/// Per-topic configuration within a group or DM.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramTopicConfig {
    /// If true, bot only responds when @mentioned or replied to.
    /// None = inherit from parent group/account default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_mention: Option<bool>,
    /// If false, disable the bot for this topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Optional allowlist for topic senders (Telegram user IDs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_from: Vec<String>,
    /// Route this topic to a specific agent (overrides group-level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Optional system prompt snippet for this topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// Per-group configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramGroupConfig {
    /// If true, bot only responds when @mentioned or replied to.
    /// None = default to true (require mention).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_mention: Option<bool>,
    /// Per-group override for group policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_policy: Option<GroupPolicy>,
    /// If false, disable the bot for this group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Optional allowlist for group senders (Telegram user IDs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_from: Vec<String>,
    /// Route this group to a specific agent (overrides account-level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Optional system prompt snippet for this group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Per-topic configuration (key is message_thread_id as string).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub topics: HashMap<String, TelegramTopicConfig>,
}

/// Per-channel (Telegram Channel broadcast) configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChannelConfig {
    /// If true, bot only responds when @mentioned or replied to.
    /// None = default to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_mention: Option<bool>,
    /// If false, ignore messages from this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Route this channel to a specific agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Optional system prompt for this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

// ── Parse Mode ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParseMode {
    Html,
    Markdown,
    Plain,
}

// ── Channel Meta ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMeta {
    pub id: ChannelId,
    pub display_name: String,
    pub description: String,
    pub version: String,
}

// ── Channel Capabilities ─────────────────────────────────────────
// Static feature advertisement per channel (used by UI and approval UX).

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelCapabilities {
    pub chat_types: Vec<ChatType>,
    #[serde(default)]
    pub supports_polls: bool,
    #[serde(default)]
    pub supports_reactions: bool,
    #[serde(default)]
    pub supports_draft: bool,
    #[serde(default)]
    pub supports_edit: bool,
    #[serde(default)]
    pub supports_unsend: bool,
    #[serde(default)]
    pub supports_reply: bool,
    #[serde(default)]
    pub supports_threads: bool,
    #[serde(default)]
    pub supports_media: Vec<MediaType>,
    #[serde(default)]
    pub supports_typing: bool,
    #[serde(default)]
    pub supports_buttons: bool,
    #[serde(default)]
    pub max_message_length: Option<usize>,
}

// ── Inbound Message Context ──────────────────────────────────────
// Normalized inbound message from any channel.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsgContext {
    pub channel_id: ChannelId,
    pub account_id: String,
    pub sender_id: String,
    pub sender_name: Option<String>,
    pub sender_username: Option<String>,
    pub chat_id: String,
    pub chat_type: ChatType,
    pub chat_title: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: String,
    pub text: Option<String>,
    #[serde(default)]
    pub media: Vec<InboundMedia>,
    pub reply_to_message_id: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Whether the bot was @mentioned or replied to in this message.
    #[serde(default)]
    pub was_mentioned: bool,
    /// Raw platform-specific payload for debugging.
    #[serde(default)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundMedia {
    pub media_type: MediaType,
    pub file_id: String,
    pub file_url: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
    pub caption: Option<String>,
}

// ── Outbound Reply Payload ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyPayload {
    pub text: Option<String>,
    #[serde(default)]
    pub media: Vec<OutboundMedia>,
    pub reply_to_message_id: Option<String>,
    pub parse_mode: Option<ParseMode>,
    #[serde(default)]
    pub buttons: Vec<Vec<InlineButton>>,
    pub thread_id: Option<String>,
    /// Draft ID for streaming (e.g. Telegram sendMessageDraft).
    /// Must be non-zero. Drafts with the same ID are animated in the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<i64>,
}

impl ReplyPayload {
    /// Create a simple text reply.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            media: Vec::new(),
            reply_to_message_id: None,
            parse_mode: None,
            buttons: Vec::new(),
            thread_id: None,
            draft_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundMedia {
    pub media_type: MediaType,
    pub data: MediaData,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaData {
    Url(String),
    FilePath(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineButton {
    pub text: String,
    pub callback_data: Option<String>,
    pub url: Option<String>,
}

// ── Security Config ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    #[serde(default)]
    pub dm_policy: DmPolicy,
    /// Legacy group allowlist (by chat_id). Kept for backward compatibility.
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    #[serde(default)]
    pub user_allowlist: Vec<String>,
    #[serde(default)]
    pub admin_ids: Vec<String>,

    // ── Layered group / channel config ────────────────────────────
    /// Account-level group policy (open | allowlist | disabled).
    #[serde(default)]
    pub group_policy: GroupPolicy,
    /// Per-group configuration (key is chat_id string; "*" = wildcard default).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub groups: HashMap<String, TelegramGroupConfig>,
    /// Per-channel (Telegram Channel) configuration (key is chat_id string).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channels: HashMap<String, TelegramChannelConfig>,
}

// ── Channel Account Config ───────────────────────────────────────
// Persisted configuration for a single account on a channel.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAccountConfig {
    pub id: String,
    pub channel_id: ChannelId,
    pub label: String,
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Agent ID bound to this channel account. If None, falls back to global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Opaque per-channel credential blob (e.g. {"token": "..."}).
    #[serde(default)]
    pub credentials: serde_json::Value,
    /// Channel-specific settings (e.g. {"transport": "polling"}).
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub security: SecurityConfig,
    /// When true, all tool calls from this IM channel are automatically approved.
    #[serde(default)]
    pub auto_approve_tools: bool,
    /// When true (default), the channel worker emits a system message into
    /// the IM chat whenever its primary-attach status changes — i.e.
    /// "you are now primary" / "you are now observing". Toggleable per
    /// account so noisy multi-attach setups can stay quiet. Subscribers
    /// listen on the `channel:primary_changed` EventBus topic emitted by
    /// `ChannelDB::{attach,detach,set_primary,update}_session`.
    #[serde(default = "crate::default_true")]
    pub notify_primary_changes: bool,
}

// ── Channel Health ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelHealth {
    pub is_running: bool,
    pub last_probe: Option<String>,
    pub probe_ok: Option<bool>,
    pub error: Option<String>,
    pub uptime_secs: Option<u64>,
    pub bot_name: Option<String>,
}

// ── Delivery Result ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryResult {
    pub success: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}

impl DeliveryResult {
    pub fn ok(message_id: impl Into<String>) -> Self {
        Self {
            success: true,
            message_id: Some(message_id.into()),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            message_id: None,
            error: Some(error.into()),
        }
    }
}
