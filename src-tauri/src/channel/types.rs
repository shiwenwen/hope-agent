use serde::{Deserialize, Serialize};

// ── Channel ID ───────────────────────────────────────────────────
// Matches OpenClaw CHAT_CHANNEL_ORDER exactly.

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelId {
    Telegram,
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
    /// Extension channels not in the built-in list.
    #[serde(untagged)]
    Custom(String),
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelId::Telegram => write!(f, "telegram"),
            ChannelId::WhatsApp => write!(f, "whatsapp"),
            ChannelId::Discord => write!(f, "discord"),
            ChannelId::Irc => write!(f, "irc"),
            ChannelId::GoogleChat => write!(f, "googlechat"),
            ChannelId::Slack => write!(f, "slack"),
            ChannelId::Signal => write!(f, "signal"),
            ChannelId::IMessage => write!(f, "imessage"),
            ChannelId::Line => write!(f, "line"),
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
// Compatible with OpenClaw's dmPolicy config.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DmPolicy {
    Open,
    Allowlist,
    Pairing,
}

impl Default for DmPolicy {
    fn default() -> Self {
        DmPolicy::Open
    }
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
// Static feature advertisement per channel, matching OpenClaw's schema.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelCapabilities {
    pub chat_types: Vec<ChatType>,
    #[serde(default)]
    pub supports_polls: bool,
    #[serde(default)]
    pub supports_reactions: bool,
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
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    #[serde(default)]
    pub user_allowlist: Vec<String>,
    #[serde(default)]
    pub admin_ids: Vec<String>,
}

// ── Channel Account Config ───────────────────────────────────────
// Persisted configuration for a single account on a channel.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAccountConfig {
    pub id: String,
    pub channel_id: ChannelId,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Opaque per-channel credential blob (e.g. {"token": "..."}).
    #[serde(default)]
    pub credentials: serde_json::Value,
    /// Channel-specific settings (e.g. {"transport": "polling"}).
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub security: SecurityConfig,
}

fn default_true() -> bool {
    true
}

// ── Channel Health ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelHealth {
    pub is_running: bool,
    pub last_probe: Option<String>,
    pub probe_ok: Option<bool>,
    pub error: Option<String>,
    pub uptime_secs: Option<u64>,
    pub bot_name: Option<String>,
}

impl Default for ChannelHealth {
    fn default() -> Self {
        Self {
            is_running: false,
            last_probe: None,
            probe_ok: None,
            error: None,
            uptime_secs: None,
            bot_name: None,
        }
    }
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
