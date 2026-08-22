use serde::{Deserialize, Serialize};

// 类型已下沉 ha-config-schema（AppConfig 类型闭包）：配置类类型与其
// inherent impl / Display / 设置键常量原地再导出，既有路径不变。
// 本文件保留运行时类型（消息 / 事件 / 能力 / 投递结果等）。
pub use ha_config_schema::channel::{
    ChannelAccountConfig, ChannelId, DmPolicy, GroupPolicy, ImReplyMode, SecurityConfig,
    TelegramChannelConfig, TelegramGroupConfig, TelegramTopicConfig,
    SETTINGS_KEY_AUTO_TRANSCRIBE_VOICE, SETTINGS_KEY_DISCORD_CHANNEL_OBFUSCATION,
    SETTINGS_KEY_DISCORD_FILE_REQUESTS, SETTINGS_KEY_GOOGLE_CHAT_STANDARD_MARKDOWN,
    SETTINGS_KEY_IMSG_PROTOCOL_V1, SETTINGS_KEY_IM_REPLY_MODE, SETTINGS_KEY_KB_ACCESS_CHATS,
    SETTINGS_KEY_KB_ACCESS_OPT_IN, SETTINGS_KEY_NATIVE_REPLY, SETTINGS_KEY_SHOW_THINKING,
};

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
    /// Streaming-preview byte budget. Used **only** to decide whether the
    /// in-flight `text_delta` accumulator still fits in a single preview
    /// message — when `native_text.len() > streaming_preview_max_bytes`,
    /// the streaming task drops preview rendering and falls back to chunked
    /// `send_text_chunks` for that round.
    ///
    /// Conventionally set ~25% below the platform's true single-message
    /// limit so a still-growing preview doesn't trip the limit at the
    /// last delta. **This is not the chunk-send slice size** — that's
    /// controlled by each plugin's `chunk_message` override (which uses
    /// the platform's true byte ceiling).
    ///
    /// `None` = no preview byte gate (channel either has no streaming
    /// preview, or relies on a different transport like cardkit).
    #[serde(default)]
    pub streaming_preview_max_bytes: Option<usize>,
    /// Channel offers a "card streaming" API that mutates a card element's
    /// content in place without flagging the host message as edited.
    /// Currently only Feishu (cardkit) implements this.
    #[serde(default)]
    pub supports_card_stream: bool,
    /// Native rich-reply streaming contract exposed by this channel.
    /// `None` keeps the legacy draft/card/message preview paths active.
    #[serde(default)]
    pub native_reply: Option<NativeReplyCapabilities>,
}

// ── Native Reply Streaming ────────────────────────────────────

/// Target-scoped native reply capabilities. Unlike the legacy preview flags,
/// these limits describe the platform's native streaming/rich-message API and
/// are intentionally independent from `streaming_preview_max_bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeReplyCapabilities {
    #[serde(default)]
    pub preview_chat_types: Vec<ChatType>,
    #[serde(default)]
    pub final_chat_types: Vec<ChatType>,
    pub update_mode: ReplyStreamUpdateMode,
    /// Whether acknowledged preview revisions are already durable messages or
    /// merely an expiring visualization whose `commit` performs a separate
    /// persistent mutation. A preview-only failure cannot suppress the
    /// ephemeral stream's durable final lane, while an ambiguous persistent
    /// commit still must not be retried.
    #[serde(default)]
    pub preview_persistence: ReplyStreamPreviewPersistence,
    /// Requirements below apply to `open_reply_stream` only. Standalone final
    /// rich replies are gated independently by `final_chat_types` and the
    /// adapter's target preflight.
    #[serde(default)]
    pub requires_reply_anchor: bool,
    /// Whether a non-empty [`ReplyStreamTarget::thread_id`] satisfies
    /// `requires_reply_anchor` for `open_reply_stream`. Providers must opt in:
    /// supporting threads generally does not imply that the native streaming
    /// API accepts a thread id as its reply anchor.
    #[serde(default)]
    pub supports_thread_anchor: bool,
    #[serde(default)]
    pub requires_recipient_user_id: bool,
    #[serde(default)]
    pub requires_recipient_tenant_id: bool,
    #[serde(default)]
    pub supports_task_updates: bool,
    #[serde(default)]
    pub supports_plan_updates: bool,
    /// The adapter can compile canonical Markdown into provider-native
    /// structured blocks. This never authorizes callers to pass arbitrary
    /// provider block JSON through the common contract.
    #[serde(default)]
    pub supports_blocks: bool,
    /// Media kinds that the native rich-reply endpoint can embed into the
    /// canonical final document. Items outside this list stay on the legacy
    /// media delivery path.
    #[serde(default)]
    pub embedded_media_types: Vec<MediaType>,
    /// Maximum number of media items that can be embedded into one native
    /// final reply. `None` means the adapter does not advertise an embedding
    /// budget; callers must not interpret it as unlimited. Missing values from
    /// older serialized capabilities therefore remain conservatively disabled.
    #[serde(default)]
    pub max_embedded_media_items: Option<u16>,
    #[serde(default)]
    pub refresh_after_secs: Option<u64>,
    #[serde(default)]
    pub max_delta_chars: Option<u32>,
}

/// How a native stream consumes text updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplyStreamUpdateMode {
    /// Each push contains only text not accepted by earlier pushes.
    Append,
    /// Each push replaces the currently visible draft with a full snapshot.
    Snapshot,
}

/// Delivery semantics of the provider-owned preview before `commit`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplyStreamPreviewPersistence {
    /// Accepted preview revisions may already be user-visible durable content.
    /// Falling back after acceptance could duplicate the reply.
    #[default]
    Persistent,
    /// Preview revisions expire automatically and `commit` is an independent
    /// persistent send. A preview failure may abandon further refreshes, but
    /// it never makes the durable final ineligible.
    Ephemeral,
}

/// Fully resolved destination for a native stream or rich final reply.
#[derive(Clone, PartialEq, Eq)]
pub struct ReplyStreamTarget {
    pub account_id: String,
    pub chat_id: String,
    pub chat_type: ChatType,
    pub thread_id: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub recipient_user_id: Option<String>,
    /// Platform-neutral tenant/workspace identity for the recipient. Slack
    /// Connect maps this to `recipient_team_id`; single-tenant channels leave
    /// it unset.
    pub recipient_tenant_id: Option<String>,
}

/// One revision of an in-flight native reply.
#[derive(Clone, PartialEq, Eq)]
pub struct ReplyStreamFrame {
    /// Strictly increasing within one stream, including keepalive refreshes.
    /// A successful `open_reply_stream` consumes its first revision; later
    /// `push` calls must use larger values. A keepalive repeats the acknowledged
    /// snapshot/task state with an empty delta under a fresh revision.
    pub revision: u64,
    /// Canonical full Markdown at this revision. Snapshot adapters consume
    /// this field; append adapters ignore it. This value is never clipped:
    /// provider-specific visible-window limits belong inside the adapter so a
    /// snapshot implementation never has to depend on delta reconstruction.
    pub markdown_snapshot: String,
    /// Canonical Markdown accepted since the preceding acknowledged revision.
    /// Append adapters consume this field; snapshot adapters ignore it.
    pub markdown_delta: String,
    pub phase: ReplyStreamPhase,
    /// Complete, locally generated, user-safe task snapshot for this revision.
    /// Adapters diff it against their last ACK. It must never contain tool
    /// arguments, raw results, credentials, file contents, or private paths.
    pub tasks: Vec<ReplyStreamTask>,
    /// Complete user-safe plan heading; `None` means no plan is displayed.
    pub plan_title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyStreamPhase {
    Generating,
    RunningTools,
    Finalizing,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReplyStreamTask {
    pub id: String,
    pub title: String,
    pub status: ReplyStreamTaskStatus,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyStreamTaskStatus {
    Pending,
    InProgress,
    Complete,
    Error,
}

/// Canonical final reply passed to a channel's native rich-message endpoint.
#[derive(Clone)]
pub struct RichReply {
    pub markdown: String,
    /// Authorized outbound media offered to the native rich-message adapter.
    /// The adapter reports consumed indices in [`RichReplyReceipt`]; all
    /// remaining items are delivered through the legacy media path.
    pub media: Vec<OutboundMedia>,
    /// A successful native receipt confirms that all actions were delivered;
    /// adapters that cannot preserve them must return `InvalidContent` before
    /// the terminal mutation and must never silently truncate actions.
    pub buttons: Vec<Vec<InlineButton>>,
}

/// Successful result of committing a native rich reply. Provider rejection,
/// transport failure, and delivery ambiguity are always returned as a typed
/// [`ReplyStreamError`], never encoded inside this receipt.
#[derive(Clone)]
pub struct RichReplyReceipt {
    /// Identifier of the last persisted native message. Adapters that split a
    /// long rich document return the final segment's identifier.
    pub message_id: String,
    /// Successful native embeds, expressed as a unique contiguous prefix of
    /// indices into `RichReply.media` (`[0, 1, .., n]`). This keeps attachment
    /// order stable when the worker delivers the remaining suffix through the
    /// legacy lane. A malformed/non-prefix receipt is terminally fail-closed:
    /// the worker will not risk duplicating provider-accepted media.
    pub consumed_media: Vec<usize>,
}

impl RichReplyReceipt {
    pub fn text_only(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            consumed_media: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyAbortReason {
    Cancelled,
    Failed,
    Detached,
}

/// Classification used by the worker to make retry and fallback decisions
/// without depending on provider-specific error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyStreamErrorKind {
    Unsupported,
    InvalidTarget,
    InvalidContent,
    Rejected,
    RateLimited,
    /// A temporary failure that occurred before any provider mutation was
    /// issued, so a caller may safely choose another delivery lane. Timeouts,
    /// disconnects, 5xx responses, or unreadable acknowledgements are always
    /// [`Self::Ambiguous`], never `Transient`.
    Transient,
    Expired,
    Ambiguous,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReplyStreamError {
    pub kind: ReplyStreamErrorKind,
    pub message: String,
}

impl ReplyStreamError {
    pub fn new(kind: ReplyStreamErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ReplyStreamErrorKind::Unsupported, message)
    }
}

impl std::fmt::Display for ReplyStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::fmt::Debug for ReplyStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplyStreamError")
            .field("kind", &self.kind)
            .field("message", &"[REDACTED]")
            .finish()
    }
}

impl std::error::Error for ReplyStreamError {}

// ── Card Stream Handle ───────────────────────────────────────────
// Resource identifiers returned from a `create_card_stream` call.

#[derive(Debug, Clone)]
pub struct CardStreamHandle {
    pub card_id: String,
    pub element_id: String,
}

// ── Card Stream Error ────────────────────────────────────────────
// Classified error from card streaming endpoints. Lets the streaming task
// decide between local recovery, immediate degrade, or session abort
// without hard-coding platform error codes.

#[derive(Debug, Clone)]
pub enum CardStreamError {
    /// Sequence number not strictly increasing (Feishu 300317).
    SequenceOutOfOrder,
    /// Card past its 14-day TTL (Feishu 200750).
    Expired,
    /// Streaming session past its 10-minute auto-close window (Feishu 200850).
    TimedOut,
    /// Card was created without `streaming_mode=true` (Feishu 300309).
    NotEnabled,
    /// App scope or tenant token missing the card stream permission
    /// (Feishu 300311).
    NoPermission,
    /// Anything else — network errors, parse failures, unknown codes.
    Other(String),
}

impl std::fmt::Display for CardStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SequenceOutOfOrder => write!(f, "card stream sequence out of order"),
            Self::Expired => write!(f, "card expired"),
            Self::TimedOut => write!(f, "card stream timed out"),
            Self::NotEnabled => write!(f, "card stream mode not enabled"),
            Self::NoPermission => write!(f, "card stream permission denied"),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for CardStreamError {}

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
    /// Sender's tenant/workspace identity when the platform exposes one
    /// (notably Slack Connect). Kept separate from `account_id`, which names
    /// the local bot configuration rather than the remote user's workspace.
    #[serde(default)]
    pub sender_tenant_id: Option<String>,
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

// ── Inbound Event ────────────────────────────────────────────────
// Top-level event delivered from a channel plugin to the dispatcher.
// `Message` is the canonical payload (a user wrote something for the bot to
// respond to). All other variants are out-of-band signals — they may or may
// not trigger an agent round depending on the dispatcher's policy for each
// variant. v0.2.0 keeps non-Message variants log-only at the dispatcher;
// business behavior (sync edits, recall removal, welcome templates) is
// deferred to v0.3+.

/// Top-level event from a channel plugin to the dispatcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InboundEvent {
    /// A new user message — full chat round trigger.
    Message(MsgContext),
    /// User added or removed an emoji reaction on an existing message.
    Reaction(ReactionEvent),
    /// User edited the text/content of a previously sent message.
    /// Feishu does not currently expose this; Telegram/Discord do.
    MessageEdited(EditedMessageEvent),
    /// Message was withdrawn by sender. Channel-specific recall windows
    /// (e.g. Feishu 24h, Telegram 48h) determine availability.
    MessageRecalled(RecalledMessageEvent),
    /// Membership change in a chat — user/bot joined or left.
    Membership(MembershipEvent),
    /// User read the bot's last sent message. Spammy on busy chats — the
    /// dispatcher's default policy is to log+drop unless explicitly enabled.
    ReadReceipt(ReadReceiptEvent),
}

/// Common envelope shared by all non-Message inbound events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCommon {
    pub channel_id: ChannelId,
    pub account_id: String,
    pub chat_id: String,
    pub chat_type: ChatType,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Raw platform-specific payload for diagnostics / debugging.
    /// Wrapped in `Arc` so per-source fan-out (e.g. read-receipt batches with
    /// 100 message_ids → 100 events) shares one buffer instead of deep-cloning.
    #[serde(default = "default_raw_arc")]
    pub raw: std::sync::Arc<serde_json::Value>,
}

fn default_raw_arc() -> std::sync::Arc<serde_json::Value> {
    std::sync::Arc::new(serde_json::Value::Null)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    pub sender_id: String,
    pub emoji: String,
    /// `true` = reaction added; `false` = removed.
    pub added: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedMessageEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    pub sender_id: String,
    pub new_text: Option<String>,
    pub edited_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecalledMessageEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    /// Some channels (Telegram) report who recalled; others don't.
    pub recalled_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MembershipAction {
    UserJoined {
        user_id: String,
        inviter_id: Option<String>,
    },
    UserLeft {
        user_id: String,
        kicked_by: Option<String>,
    },
    BotJoined {
        added_by: Option<String>,
    },
    BotLeft {
        removed_by: Option<String>,
    },
    ChatCreated,
    ChatDisbanded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembershipEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    #[serde(flatten)]
    pub action: MembershipAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReceiptEvent {
    #[serde(flatten)]
    pub common: EventCommon,
    pub message_id: String,
    pub reader_id: String,
}

impl From<MsgContext> for InboundEvent {
    fn from(msg: MsgContext) -> Self {
        InboundEvent::Message(msg)
    }
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

// ── Channel Health ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCapabilitySnapshot {
    /// Adapter-owned, fixed identifier such as `signal-cli` or
    /// `whatsapp-bridge`; never copied from an arbitrary Bridge response.
    pub source: String,
    /// Sanitized, length-bounded runtime version when it can be detected.
    pub version: Option<String>,
    pub observed_at: String,
    /// Adapter allowlist only. Unrecognized external strings cannot grant a
    /// capability or become UI/log output.
    pub capabilities: Vec<String>,
    /// `compatible`, `warning`, `blocked`, or `unknown`.
    pub compatibility: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelHealth {
    pub is_running: bool,
    pub last_probe: Option<String>,
    pub probe_ok: Option<bool>,
    pub error: Option<String>,
    pub uptime_secs: Option<u64>,
    pub bot_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot: Option<AccountCapabilitySnapshot>,
}

// ── Delivery Result ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Legacy outbound result. `success=true` is a confirmed acknowledgement;
/// `success=false` is only a generic failure and does **not** prove that the
/// provider performed no mutation. Callers must not automatically retry a
/// non-idempotent send from this value alone.
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

// `InlineButton` 的固有 impl 必须待在定义它的 crate 里（Rust 孤儿规则）。
// IM 侧的 approval / ask_user 卡片按这个值匹配回调，故随类型留 kernel。
impl InlineButton {
    /// Returns the effective callback identifier: `callback_data` if set, otherwise `text`.
    pub fn callback_id(&self) -> &str {
        self.callback_data.as_deref().unwrap_or(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_account(settings: serde_json::Value) -> ChannelAccountConfig {
        ChannelAccountConfig {
            id: "x".into(),
            channel_id: ChannelId::WeChat,
            label: "x".into(),
            enabled: true,
            agent_id: None,
            credentials: serde_json::Value::Null,
            settings,
            security: SecurityConfig::default(),
            auto_approve_tools: false,
            notify_session_eviction: true,
            notify_startup: true,
        }
    }

    #[test]
    fn im_reply_mode_parses_canonical_and_short_forms() {
        assert_eq!(ImReplyMode::parse("split"), Some(ImReplyMode::Split));
        assert_eq!(ImReplyMode::parse("final"), Some(ImReplyMode::Final));
        assert_eq!(ImReplyMode::parse("preview"), Some(ImReplyMode::Preview));
        // Single-letter shortcuts.
        assert_eq!(ImReplyMode::parse("S"), Some(ImReplyMode::Split));
        assert_eq!(ImReplyMode::parse("f"), Some(ImReplyMode::Final));
        assert_eq!(ImReplyMode::parse("P"), Some(ImReplyMode::Preview));
        assert_eq!(ImReplyMode::parse("  SPLIT  "), Some(ImReplyMode::Split));
        assert_eq!(ImReplyMode::parse("merged"), None);
        assert_eq!(ImReplyMode::parse(""), None);
    }

    #[test]
    fn im_reply_mode_falls_back_to_default_when_settings_missing() {
        // Default is Split — ungrouped accounts get the time-ordered behavior.
        assert_eq!(
            mk_account(serde_json::Value::Null).im_reply_mode(),
            ImReplyMode::Split
        );
        assert_eq!(
            mk_account(serde_json::json!({})).im_reply_mode(),
            ImReplyMode::Split
        );
        assert_eq!(
            mk_account(serde_json::json!({"imReplyMode": "garbage"})).im_reply_mode(),
            ImReplyMode::Split
        );
    }

    #[test]
    fn set_im_reply_mode_initializes_and_overwrites_settings() {
        // Null settings → object created.
        let mut acc = mk_account(serde_json::Value::Null);
        acc.set_im_reply_mode(ImReplyMode::Split);
        assert_eq!(acc.settings["imReplyMode"], "split");
        assert_eq!(acc.im_reply_mode(), ImReplyMode::Split);

        // Existing keys preserved on update.
        let mut acc = mk_account(serde_json::json!({"transport": "polling"}));
        acc.set_im_reply_mode(ImReplyMode::Split);
        assert_eq!(acc.settings["transport"], "polling");
        assert_eq!(acc.settings["imReplyMode"], "split");

        // Overwrite.
        acc.set_im_reply_mode(ImReplyMode::Final);
        assert_eq!(acc.settings["imReplyMode"], "final");
    }

    #[test]
    fn show_thinking_defaults_to_false_when_missing_or_invalid() {
        assert!(!mk_account(serde_json::Value::Null).show_thinking());
        assert!(!mk_account(serde_json::json!({})).show_thinking());
        // Non-bool values fall back to the default.
        assert!(!mk_account(serde_json::json!({"showThinking": "yes"})).show_thinking());
        assert!(!mk_account(serde_json::json!({"showThinking": 1})).show_thinking());
        assert!(mk_account(serde_json::json!({"showThinking": true})).show_thinking());
    }

    #[test]
    fn set_show_thinking_initializes_and_overwrites_settings() {
        // Null settings → object created.
        let mut acc = mk_account(serde_json::Value::Null);
        acc.set_show_thinking(true);
        assert_eq!(acc.settings["showThinking"], true);
        assert!(acc.show_thinking());

        // Sibling keys preserved.
        let mut acc = mk_account(serde_json::json!({"imReplyMode": "split"}));
        acc.set_show_thinking(true);
        assert_eq!(acc.settings["imReplyMode"], "split");
        assert_eq!(acc.settings["showThinking"], true);

        // Overwrite back to false.
        acc.set_show_thinking(false);
        assert_eq!(acc.settings["showThinking"], false);
        assert!(!acc.show_thinking());
    }

    #[test]
    fn auto_transcribe_voice_defaults_to_false() {
        assert!(!mk_account(serde_json::Value::Null).auto_transcribe_voice());
        assert!(!mk_account(serde_json::json!({})).auto_transcribe_voice());
        // Non-bool values fall back to default.
        assert!(
            !mk_account(serde_json::json!({"autoTranscribeVoice": "yes"})).auto_transcribe_voice()
        );
        assert!(
            mk_account(serde_json::json!({"autoTranscribeVoice": true})).auto_transcribe_voice()
        );
    }

    #[test]
    fn set_auto_transcribe_voice_round_trip() {
        let mut acc = mk_account(serde_json::Value::Null);
        acc.set_auto_transcribe_voice(true);
        assert!(acc.auto_transcribe_voice());

        // Sibling keys preserved.
        let mut acc = mk_account(serde_json::json!({"imReplyMode": "split"}));
        acc.set_auto_transcribe_voice(true);
        assert_eq!(acc.settings["imReplyMode"], "split");
        assert!(acc.auto_transcribe_voice());

        // Toggle back off.
        acc.set_auto_transcribe_voice(false);
        assert!(!acc.auto_transcribe_voice());
    }

    #[test]
    fn kb_access_opt_in_defaults_to_false() {
        assert!(!mk_account(serde_json::Value::Null).kb_access_opt_in());
        assert!(!mk_account(serde_json::json!({})).kb_access_opt_in());
        // Non-bool falls back to false (fail closed).
        assert!(!mk_account(serde_json::json!({"kbAccessOptIn": "yes"})).kb_access_opt_in());
        assert!(mk_account(serde_json::json!({"kbAccessOptIn": true})).kb_access_opt_in());
    }

    #[test]
    fn set_kb_access_opt_in_round_trip() {
        let mut acc = mk_account(serde_json::Value::Null);
        acc.set_kb_access_opt_in(true);
        assert!(acc.kb_access_opt_in());

        // Sibling keys preserved.
        let mut acc = mk_account(serde_json::json!({"imReplyMode": "split"}));
        acc.set_kb_access_opt_in(true);
        assert_eq!(acc.settings["imReplyMode"], "split");
        assert!(acc.kb_access_opt_in());

        acc.set_kb_access_opt_in(false);
        assert!(!acc.kb_access_opt_in());
    }

    #[test]
    fn kb_access_chat_confirm_add_remove() {
        let mut acc = mk_account(serde_json::Value::Null);
        assert!(!acc.kb_access_chat_confirmed("g1"));

        acc.set_kb_access_chat("g1", true);
        assert!(acc.kb_access_chat_confirmed("g1"));
        assert!(!acc.kb_access_chat_confirmed("g2"));

        // Idempotent add — no duplicate entry.
        acc.set_kb_access_chat("g1", true);
        assert_eq!(
            acc.settings[SETTINGS_KEY_KB_ACCESS_CHATS]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // Remove.
        acc.set_kb_access_chat("g1", false);
        assert!(!acc.kb_access_chat_confirmed("g1"));

        // Sibling opt-in flag is untouched by chat-list edits.
        let mut acc = mk_account(serde_json::json!({"kbAccessOptIn": true}));
        acc.set_kb_access_chat("g9", true);
        assert!(acc.kb_access_opt_in());
        assert!(acc.kb_access_chat_confirmed("g9"));
    }
}
