//! IM channel configuration (`AppConfig.channels`).
//!
//! Wire types for the channel subsystem's persisted configuration: the
//! top-level [`ChannelStoreConfig`], per-account [`ChannelAccountConfig`]
//! (credentials blob included — redaction wiring stays in `ha-core`), and
//! the layered security / Telegram group-and-channel policy types.
//!
//! Runtime channel types (messages, inbound events, capabilities, delivery
//! results …) deliberately stay in `ha-core::channel::types` — only the
//! `AppConfig` type closure lives here.

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

impl ChannelId {
    /// Parse the canonical lowercase form (the value stored in SQLite
    /// `channel_conversations.channel_id` and emitted by `Display`) back
    /// to a `ChannelId`, falling back to `Custom(s)` for extension
    /// channels via the existing `#[serde(untagged)]` variant. Use this
    /// from EventBus / DB callbacks where you only have the string form
    /// — both `eviction_watcher` and `startup_watcher` go through here.
    pub fn from_storage_str(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
    }
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

// ── IM Reply Mode ────────────────────────────────────────────────
// Controls how the dispatcher delivers multi-round assistant output (text +
// tool-produced media) over an IM channel. Three modes, all channels honor
// the same setting — streaming vs non-streaming only changes whether each
// round's text is rendered with a typewriter preview or as a single shot.
//
// **Round** here = one LLM `process_round` (an assistant message that may
// contain narration + tool_calls). `RoundTextAccumulator` watches the
// `text_delta` / `tool_call` / `tool_result` event stream and groups events
// into per-round buckets; the dispatcher fans them out per `ImReplyMode`.
//
// Lives here (not with the runtime types in `ha-core`) because
// `ChannelAccountConfig::im_reply_mode` reads it from the account's settings
// blob, and an inherent impl must sit in the type's defining crate.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImReplyMode {
    /// (Default) Each round's text + media is delivered in time order, as
    /// independent messages — narration → tool media → next narration → ...
    /// Streaming channels still get a typewriter effect *per round*, just
    /// not "one growing message"; non-streaming channels send each round in
    /// one shot. Mirrors how the model actually narrated the work.
    #[default]
    Split,
    /// Drop pre-tool narration; deliver only the final round's text plus all
    /// tool media in one outbound burst. No streaming preview.
    Final,
    /// Streaming-only: render the full merged response in a single growing
    /// preview message (Telegram edit / Feishu cardkit / Telegram DM draft),
    /// finalize at the end, then send all media. Non-streaming channels
    /// degrade to `Final` since they have no preview transport to speak of.
    Preview,
}

impl ImReplyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Final => "final",
            Self::Preview => "preview",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "split" | "s" => Some(Self::Split),
            "final" | "f" => Some(Self::Final),
            "preview" | "p" => Some(Self::Preview),
            _ => None,
        }
    }
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
    /// When true (default), the eviction watcher emits a system message
    /// into the IM chat when it gets evicted from a session because
    /// another chat took it over (1:1 attach invariant). Toggleable per
    /// account. Subscribers listen on the `channel:session_evicted`
    /// EventBus topic emitted by `ChannelDB::{attach,update}_session`.
    #[serde(default = "crate::default_true")]
    pub notify_session_eviction: bool,
    /// When true (default), `channel::worker::startup_watcher` posts a
    /// short "back online" notice into every chat on this account that
    /// was active within `AppConfig.startup_notification.window_secs`
    /// after a fresh process boot. Toggleable per account.
    #[serde(default = "crate::default_true")]
    pub notify_startup: bool,
}

/// Settings JSON key controlling IM reply mode (see [`ImReplyMode`]).
pub const SETTINGS_KEY_IM_REPLY_MODE: &str = "imReplyMode";

/// Settings JSON key controlling whether the model's thinking/reasoning
/// content is included in outbound IM messages (toggled via the `/reason`
/// slash command). Default `false` — reasoning stays out of IM messages.
pub const SETTINGS_KEY_SHOW_THINKING: &str = "showThinking";

/// Settings JSON key controlling whether incoming voice / audio messages
/// are auto-transcribed by the STT subsystem before reaching the chat
/// engine. Default `false` — transcription costs API quota per message,
/// so the user has to opt in per account.
pub const SETTINGS_KEY_AUTO_TRANSCRIBE_VOICE: &str = "autoTranscribeVoice";

/// Settings JSON key — account-level opt-in to knowledge-base access from this
/// IM channel (WS8). Default `false`: IM turns get zero KB access (design D10)
/// unless the owner explicitly enables it per account. For group / non-DM chats
/// this opt-in is necessary but **not** sufficient — each group chat must also be
/// confirmed in [`SETTINGS_KEY_KB_ACCESS_CHATS`].
pub const SETTINGS_KEY_KB_ACCESS_OPT_IN: &str = "kbAccessOptIn";

/// Settings JSON key — array of confirmed group/non-DM chat ids allowed KB
/// access (WS8). A DM only needs the account-level opt-in; a group additionally
/// needs its chat id listed here (confirmed via the in-chat `/kb on` command or
/// the account dialog).
pub const SETTINGS_KEY_KB_ACCESS_CHATS: &str = "kbAccessChats";

impl ChannelAccountConfig {
    /// Read `settings.imReplyMode`, falling back to `ImReplyMode::default()`
    /// when missing or unparseable.
    pub fn im_reply_mode(&self) -> ImReplyMode {
        self.settings
            .get(SETTINGS_KEY_IM_REPLY_MODE)
            .and_then(|v| v.as_str())
            .and_then(ImReplyMode::parse)
            .unwrap_or_default()
    }

    /// Write `settings.imReplyMode = mode` in place. Creates the settings
    /// object if it was previously `null` / non-object.
    pub fn set_im_reply_mode(&mut self, mode: ImReplyMode) {
        if !self.settings.is_object() {
            self.settings = serde_json::json!({});
        }
        if let Some(obj) = self.settings.as_object_mut() {
            obj.insert(
                SETTINGS_KEY_IM_REPLY_MODE.to_string(),
                serde_json::Value::String(mode.as_str().to_string()),
            );
        }
    }

    /// Read `settings.showThinking`. Default `false` — reasoning is not
    /// included in IM messages unless the user opts in via `/reason on` or
    /// the channel-account dialog toggle.
    pub fn show_thinking(&self) -> bool {
        self.settings
            .get(SETTINGS_KEY_SHOW_THINKING)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Write `settings.showThinking = on`. Creates the settings object if
    /// it was previously `null` / non-object.
    pub fn set_show_thinking(&mut self, on: bool) {
        if !self.settings.is_object() {
            self.settings = serde_json::json!({});
        }
        if let Some(obj) = self.settings.as_object_mut() {
            obj.insert(
                SETTINGS_KEY_SHOW_THINKING.to_string(),
                serde_json::Value::Bool(on),
            );
        }
    }

    /// Read `settings.autoTranscribeVoice`. Default `false` — opt-in
    /// because each transcription consumes STT API quota.
    pub fn auto_transcribe_voice(&self) -> bool {
        self.settings
            .get(SETTINGS_KEY_AUTO_TRANSCRIBE_VOICE)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Write `settings.autoTranscribeVoice = on`. Creates the settings
    /// object if it was previously `null` / non-object.
    pub fn set_auto_transcribe_voice(&mut self, on: bool) {
        if !self.settings.is_object() {
            self.settings = serde_json::json!({});
        }
        if let Some(obj) = self.settings.as_object_mut() {
            obj.insert(
                SETTINGS_KEY_AUTO_TRANSCRIBE_VOICE.to_string(),
                serde_json::Value::Bool(on),
            );
        }
    }

    /// Read `settings.kbAccessOptIn` (WS8). Default `false` — IM channels have
    /// zero KB access unless the owner opts the account in.
    pub fn kb_access_opt_in(&self) -> bool {
        self.settings
            .get(SETTINGS_KEY_KB_ACCESS_OPT_IN)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Write `settings.kbAccessOptIn = on`. Creates the settings object if it
    /// was previously `null` / non-object.
    pub fn set_kb_access_opt_in(&mut self, on: bool) {
        if !self.settings.is_object() {
            self.settings = serde_json::json!({});
        }
        if let Some(obj) = self.settings.as_object_mut() {
            obj.insert(
                SETTINGS_KEY_KB_ACCESS_OPT_IN.to_string(),
                serde_json::Value::Bool(on),
            );
        }
    }

    /// WS8 全判定：本账号是否允许 `(channel_id, chat_id)` 这一路 IM 会话访问
    /// 知识库。**Fails closed** —— 携带的 channel id 与账号自身不符（伪造 /
    /// 陈旧身份）直接拒；群聊还须该 chat 单独确认，DM 只看账号级 opt-in。
    ///
    /// 自包含 impl（只读自身字段），故住在 schema；读全局 config 解析账号的
    /// 那一层在 ha-core 的 `knowledge::access`（KB 门的判定处）。
    pub fn kb_access_allowed_for(&self, channel_id: &str, chat_id: &str, is_group: bool) -> bool {
        // Defense in depth: the carried channel id must match the account's channel.
        if self.channel_id.to_string() != channel_id {
            return false;
        }
        if !self.kb_access_opt_in() {
            return false;
        }
        if is_group {
            self.kb_access_chat_confirmed(chat_id)
        } else {
            true
        }
    }

    /// Whether a specific group/non-DM `chat_id` is confirmed for KB access
    /// (WS8). DMs ignore this list (the account opt-in alone suffices).
    pub fn kb_access_chat_confirmed(&self, chat_id: &str) -> bool {
        self.settings
            .get(SETTINGS_KEY_KB_ACCESS_CHATS)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().any(|v| v.as_str() == Some(chat_id)))
            .unwrap_or(false)
    }

    /// Add / remove a group `chat_id` from the confirmed list (WS8). Returns the
    /// resulting confirmed state. Idempotent.
    pub fn set_kb_access_chat(&mut self, chat_id: &str, on: bool) -> bool {
        if !self.settings.is_object() {
            self.settings = serde_json::json!({});
        }
        let obj = match self.settings.as_object_mut() {
            Some(o) => o,
            None => return false,
        };
        let arr = obj
            .entry(SETTINGS_KEY_KB_ACCESS_CHATS.to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if !arr.is_array() {
            *arr = serde_json::Value::Array(Vec::new());
        }
        let list = arr.as_array_mut().expect("just ensured array");
        let present = list.iter().any(|v| v.as_str() == Some(chat_id));
        if on && !present {
            list.push(serde_json::Value::String(chat_id.to_string()));
        } else if !on && present {
            list.retain(|v| v.as_str() != Some(chat_id));
        }
        on
    }
}

// ── Channel Store Config ─────────────────────────────────────────

/// Top-level channel configuration stored in AppConfig (config.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStoreConfig {
    /// All configured channel accounts (across all channels).
    #[serde(default)]
    pub accounts: Vec<ChannelAccountConfig>,
    /// Legacy channel-specific default Agent ID. Runtime dispatch now lets
    /// unbound channel conversations inherit `AppConfig.default_agent_id`;
    /// keep this field for backward-compatible config deserialization.
    #[serde(default)]
    pub default_agent_id: Option<String>,
    /// Provider/model override for channel conversations.
    /// If None, uses the global active_model from AppConfig.
    #[serde(default)]
    pub default_model: Option<crate::provider::ActiveModel>,
}

impl ChannelStoreConfig {
    /// Find an account by its ID.
    pub fn find_account(&self, account_id: &str) -> Option<&ChannelAccountConfig> {
        self.accounts.iter().find(|a| a.id == account_id)
    }

    /// Find a mutable account by its ID.
    pub fn find_account_mut(&mut self, account_id: &str) -> Option<&mut ChannelAccountConfig> {
        self.accounts.iter_mut().find(|a| a.id == account_id)
    }

    /// List all enabled accounts.
    pub fn enabled_accounts(&self) -> Vec<&ChannelAccountConfig> {
        self.accounts.iter().filter(|a| a.enabled).collect()
    }
}

#[cfg(test)]
mod kb_access_tests {
    use super::{ChannelAccountConfig, ChannelId};

    fn account(settings: serde_json::Value) -> ChannelAccountConfig {
        ChannelAccountConfig {
            id: "acc1".into(),
            channel_id: ChannelId::WeChat,
            label: "Test".into(),
            enabled: true,
            agent_id: None,
            credentials: serde_json::Value::Null,
            settings,
            security: Default::default(),
            auto_approve_tools: false,
            notify_session_eviction: true,
            notify_startup: true,
        }
    }

    #[test]
    fn deny_without_opt_in() {
        let acc = account(serde_json::Value::Null);
        assert!(!acc.kb_access_allowed_for("wechat", "dm1", false));
        assert!(!acc.kb_access_allowed_for("wechat", "g1", true));
    }

    #[test]
    fn dm_granted_with_opt_in() {
        let acc = account(serde_json::json!({"kbAccessOptIn": true}));
        // DM: account opt-in alone suffices.
        assert!(acc.kb_access_allowed_for("wechat", "dm1", false));
    }

    #[test]
    fn group_needs_per_chat_confirm() {
        let acc = account(serde_json::json!({"kbAccessOptIn": true}));
        // Group: opt-in alone is NOT enough.
        assert!(!acc.kb_access_allowed_for("wechat", "g1", true));

        let acc = account(serde_json::json!({
            "kbAccessOptIn": true,
            "kbAccessChats": ["g1"],
        }));
        assert!(acc.kb_access_allowed_for("wechat", "g1", true));
        // A different, unconfirmed group stays denied.
        assert!(!acc.kb_access_allowed_for("wechat", "g2", true));
    }

    #[test]
    fn channel_id_mismatch_fails_closed() {
        let acc = account(serde_json::json!({"kbAccessOptIn": true}));
        // Even fully opted in, a mismatched channel id denies (fail closed).
        assert!(!acc.kb_access_allowed_for("telegram", "dm1", false));
    }
}
