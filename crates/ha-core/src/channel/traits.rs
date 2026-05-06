use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::types::*;

/// Core channel plugin contract.
///
/// Each IM channel (Telegram, Discord, Slack, etc.) implements this trait.
/// Responsibilities are grouped into six sections:
/// lifecycle, outbound, status, security, format conversion, and setup.
#[async_trait]
pub trait ChannelPlugin: Send + Sync + 'static {
    // ── Metadata ──────────────────────────────────────────────────

    /// Static metadata about this channel plugin.
    fn meta(&self) -> ChannelMeta;

    /// Advertised capabilities of this channel.
    fn capabilities(&self) -> ChannelCapabilities;

    // ── Lifecycle ─────────────────────────────────────────────────

    /// Start listening for messages on the given account.
    ///
    /// The plugin should spawn its own background tasks (polling loop, webhook
    /// server, etc.) and send inbound messages through `inbound_tx`.
    /// The `cancel` token signals graceful shutdown.
    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()>;

    /// Stop a running account. Called before app shutdown or account removal.
    async fn stop_account(&self, account_id: &str) -> Result<()>;

    // ── Outbound ──────────────────────────────────────────────────

    /// Send a message to a chat on this channel.
    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult>;

    /// Send a typing indicator. Implementations should handle keepalive
    /// internally if the platform requires periodic refresh.
    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()>;

    /// Send a message draft for streaming (e.g. Telegram's sendMessageDraft).
    ///
    /// Purpose-built for streaming partial messages during generation.
    /// Unlike `edit_message`, drafts have no rate limiting and render progressively.
    /// Call repeatedly with accumulated text, then finalize with `send_message`.
    async fn send_draft(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _payload: &ReplyPayload,
    ) -> Result<()> {
        Err(anyhow::anyhow!("send_draft not supported by this channel"))
    }

    /// Edit an existing message. Not all channels support this.
    async fn edit_message(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _message_id: &str,
        _payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        Err(anyhow::anyhow!(
            "edit_message not supported by this channel"
        ))
    }

    /// Delete an existing message. Not all channels support this.
    async fn delete_message(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _message_id: &str,
    ) -> Result<()> {
        Err(anyhow::anyhow!(
            "delete_message not supported by this channel"
        ))
    }

    // ── Card Streaming ────────────────────────────────────────────
    // Optional path used by streaming previews on channels whose normal
    // `edit_message` would taint the recipient's view (e.g. Feishu's
    // "已编辑" marker). Implementations advertise availability via
    // `capabilities().supports_card_stream`.

    /// Create a streaming-capable card holder, returning the IDs needed for
    /// subsequent `update_card_element` calls. Implementations should
    /// pre-populate the card with `initial_text` so viewers see content as
    /// soon as the card is delivered to chat.
    async fn create_card_stream(
        &self,
        _account_id: &str,
        _initial_text: &str,
    ) -> Result<CardStreamHandle> {
        Err(anyhow::anyhow!(
            "create_card_stream not supported by this channel"
        ))
    }

    /// Push a previously created card to a chat as an interactive message.
    /// Returns the host message ID (used for delete/replace at session end).
    async fn send_card_message(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _card_id: &str,
        _reply_to_message_id: Option<&str>,
        _thread_id: Option<&str>,
    ) -> Result<DeliveryResult> {
        Err(anyhow::anyhow!(
            "send_card_message not supported by this channel"
        ))
    }

    /// Append text to a streaming card element. `sequence` must be strictly
    /// increasing across all calls within one card lifetime.
    async fn update_card_element(
        &self,
        _account_id: &str,
        _card_id: &str,
        _element_id: &str,
        _content: &str,
        _sequence: i64,
    ) -> std::result::Result<(), CardStreamError> {
        Err(CardStreamError::Other(
            "update_card_element not supported by this channel".into(),
        ))
    }

    /// Disable streaming mode on a card so the host message stops showing
    /// the typing indicator. Best-effort: errors are typically logged.
    async fn close_card_stream(
        &self,
        _account_id: &str,
        _card_id: &str,
        _sequence: i64,
    ) -> Result<()> {
        Err(anyhow::anyhow!(
            "close_card_stream not supported by this channel"
        ))
    }

    // ── Status ────────────────────────────────────────────────────

    /// Probe the channel account to check health/connectivity.
    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth>;

    // ── Security ──────────────────────────────────────────────────

    /// Check whether the sender in `msg` is allowed based on `account` security rules.
    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool;

    // ── Format Conversion ─────────────────────────────────────────

    /// Convert Markdown text to the channel's native rich-text format.
    /// For Telegram this is HTML, for Discord it's native Markdown, etc.
    fn markdown_to_native(&self, markdown: &str) -> String;

    /// Split a long message into chunks that fit the channel's message length limit.
    /// The default splits at the channel's max_message_length on paragraph boundaries.
    fn chunk_message(&self, text: &str) -> Vec<String> {
        let max_len = self.capabilities().max_message_length.unwrap_or(4096);
        chunk_text(text, max_len)
    }

    // ── Setup ─────────────────────────────────────────────────────

    /// Validate the given credentials and return the bot name / account label.
    /// Used during account setup to verify the token/API key is valid.
    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String>;

    // ── Slash command menu ────────────────────────────────────────

    /// Re-sync the channel-side slash command menu against the current
    /// `slash_commands::list_slash_commands` snapshot (built-ins + skills,
    /// minus `IM_DISABLED_COMMANDS`).
    ///
    /// Default = no-op for channels without a slash menu (IRC, WhatsApp,
    /// iMessage, etc.). Telegram / Discord override to call their
    /// platform-specific endpoints (setMyCommands / Application Commands).
    /// Triggered both at `start_account` (first-time install) and on demand
    /// from skill / config changes via `ChannelRegistry::sync_commands_*`.
    async fn sync_commands(&self, _account: &ChannelAccountConfig) -> Result<()> {
        Ok(())
    }
}

/// Default access-control logic shared by all channel plugins.
///
/// Implements DM policy, group policy (allowlist / disabled / open),
/// per-group and per-topic allow_from lists, legacy group_allowlist compat,
/// and account-level user allowlist. Unsupported chat types are denied.
pub fn default_check_access(
    account: &ChannelAccountConfig,
    msg: &MsgContext,
    supported_chat_types: &[ChatType],
) -> bool {
    if !supported_chat_types.contains(&msg.chat_type) {
        return false;
    }

    let security = &account.security;

    match msg.chat_type {
        ChatType::Dm => match security.dm_policy {
            DmPolicy::Open => true,
            DmPolicy::Allowlist | DmPolicy::Pairing => {
                security.user_allowlist.contains(&msg.sender_id)
                    || security.admin_ids.contains(&msg.sender_id)
            }
        },
        ChatType::Group | ChatType::Forum | ChatType::Channel => {
            if security.group_policy == GroupPolicy::Disabled {
                return false;
            }

            let group_config = security.groups.get(&msg.chat_id);
            let wildcard_config = security.groups.get("*");
            let effective_group_config = group_config.or(wildcard_config);

            if security.group_policy == GroupPolicy::Allowlist {
                if security.groups.is_empty() {
                    if !security.group_allowlist.is_empty()
                        && !security.group_allowlist.contains(&msg.chat_id)
                    {
                        return false;
                    }
                } else if effective_group_config.is_none() {
                    return false;
                }
            }

            // Legacy group_allowlist backward compat
            if !security.group_allowlist.is_empty()
                && security.groups.is_empty()
                && !security.group_allowlist.contains(&msg.chat_id)
            {
                return false;
            }

            if let Some(cfg) = effective_group_config {
                if cfg.enabled == Some(false) {
                    return false;
                }

                // Topic-level check
                if let Some(ref thread_id) = msg.thread_id {
                    if let Some(topic_cfg) = cfg.topics.get(thread_id) {
                        if topic_cfg.enabled == Some(false) {
                            return false;
                        }
                        if !topic_cfg.allow_from.is_empty()
                            && !topic_cfg.allow_from.contains(&msg.sender_id)
                            && !security.admin_ids.contains(&msg.sender_id)
                        {
                            return false;
                        }
                    }
                }

                // Group-level sender allowlist
                if !cfg.allow_from.is_empty()
                    && !cfg.allow_from.contains(&msg.sender_id)
                    && !security.admin_ids.contains(&msg.sender_id)
                {
                    return false;
                }
            }

            // Account-level user allowlist
            if !security.user_allowlist.is_empty()
                && !security.user_allowlist.contains(&msg.sender_id)
                && !security.admin_ids.contains(&msg.sender_id)
            {
                return false;
            }

            true
        }
    }
}

/// Split text into chunks of at most `max_len` **bytes** (UTF-8), preferring
/// paragraph boundaries.
///
/// **Note**: `max_len` is byte-conservative. Most IM platforms publish their
/// limit in characters (or UTF-16 code units for Telegram); a single CJK
/// character is 3 bytes UTF-8, so 4096 bytes ≈ 1365 CJK chars. Channels
/// whose official spec is in characters MUST set `max_message_length` below
/// the spec value to leave UTF-8 headroom. See per-channel `capabilities()`.
pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at a paragraph boundary (double newline)
        let safe_limit = crate::truncate_utf8(remaining, max_len).len();
        if safe_limit == 0 {
            // max_len is smaller than the next scalar value. Keep making
            // progress instead of dropping the rest of the message.
            if let Some(ch) = remaining.chars().next() {
                chunks.push(ch.to_string());
                remaining = &remaining[ch.len_utf8()..];
                continue;
            }
            break;
        }

        let search_range = &remaining[..safe_limit];
        let split_pos = search_range
            .rfind("\n\n")
            .or_else(|| search_range.rfind('\n'))
            .or_else(|| search_range.rfind(". "))
            .or_else(|| search_range.rfind(' '))
            .unwrap_or(safe_limit);
        let split_pos = if split_pos == 0 {
            safe_limit
        } else {
            split_pos
        };

        chunks.push(remaining[..split_pos].to_string());
        remaining = remaining[split_pos..].trim_start();
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_short_text() {
        let chunks = chunk_text("Hello world", 4096);
        assert_eq!(chunks, vec!["Hello world"]);
    }

    #[test]
    fn test_chunk_at_paragraph() {
        let text = format!("{}\n\n{}", "A".repeat(100), "B".repeat(100));
        let chunks = chunk_text(&text, 150);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with("AAAA"));
        assert!(chunks[1].starts_with("BBBB"));
    }

    #[test]
    fn test_chunk_at_newline() {
        let text = format!("{}\n{}", "A".repeat(100), "B".repeat(100));
        let chunks = chunk_text(&text, 150);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn test_chunk_text_does_not_slice_inside_utf8() {
        let chunks = chunk_text("你好世界", 5);
        assert_eq!(chunks, vec!["你", "好", "世", "界"]);
    }

    #[test]
    fn test_chunk_text_progresses_when_limit_is_smaller_than_char() {
        let chunks = chunk_text("🔑🔒", 1);
        assert_eq!(chunks, vec!["🔑", "🔒"]);
    }

    #[test]
    fn test_chunk_text_does_not_emit_empty_chunk_for_leading_space() {
        let chunks = chunk_text(" 你好世界", 5);
        assert!(!chunks.iter().any(|chunk| chunk.is_empty()));
    }
}
