use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::types::*;

/// Core channel plugin contract.
///
/// Each IM channel (Telegram, Discord, Slack, etc.) implements this trait.
/// Designed to be compatible with OpenClaw's ChannelPlugin adapter pattern,
/// but flattened into a single trait for Rust ergonomics.
///
/// OpenClaw adapter mapping:
/// - GatewayAdapter  → start_account / stop_account
/// - OutboundAdapter → send_message / send_typing / edit_message / delete_message
/// - StatusAdapter   → probe
/// - SecurityAdapter → check_access
/// - SetupAdapter    → validate_credentials
/// - Format/Chunking → markdown_to_native / chunk_message
#[async_trait]
pub trait ChannelPlugin: Send + Sync + 'static {
    // ── Metadata ────���────────────────────────────────────────────

    /// Static metadata about this channel plugin.
    fn meta(&self) -> ChannelMeta;

    /// Advertised capabilities of this channel.
    fn capabilities(&self) -> ChannelCapabilities;

    // ── Lifecycle (OpenClaw GatewayAdapter) ──────────────────────

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

    // ── Outbound (OpenClaw OutboundAdapter) ──────────────────────

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

    /// Edit an existing message. Not all channels support this.
    async fn edit_message(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _message_id: &str,
        _payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        Err(anyhow::anyhow!("edit_message not supported by this channel"))
    }

    /// Delete an existing message. Not all channels support this.
    async fn delete_message(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _message_id: &str,
    ) -> Result<()> {
        Err(anyhow::anyhow!("delete_message not supported by this channel"))
    }

    // ── Status (OpenClaw StatusAdapter) ──────────────────────────

    /// Probe the channel account to check health/connectivity.
    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth>;

    // ── Security (OpenClaw SecurityAdapter) ──────────────────────

    /// Check whether the sender in `msg` is allowed based on `account` security rules.
    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool;

    // ── Format Conversion ��───────────────────────────────────────

    /// Convert Markdown text to the channel's native rich-text format.
    /// For Telegram this is HTML, for Discord it's native Markdown, etc.
    fn markdown_to_native(&self, markdown: &str) -> String;

    /// Split a long message into chunks that fit the channel's message length limit.
    /// The default splits at the channel's max_message_length on paragraph boundaries.
    fn chunk_message(&self, text: &str) -> Vec<String> {
        let max_len = self.capabilities().max_message_length.unwrap_or(4096);
        chunk_text(text, max_len)
    }

    // ── Setup (OpenClaw SetupAdapter) ────────────────────────────

    /// Validate the given credentials and return the bot name / account label.
    /// Used during account setup to verify the token/API key is valid.
    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String>;
}

/// Split text into chunks of at most `max_len` bytes, preferring paragraph boundaries.
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
        let search_range = &remaining[..max_len];
        let split_pos = search_range
            .rfind("\n\n")
            .or_else(|| search_range.rfind('\n'))
            .or_else(|| search_range.rfind(". "))
            .or_else(|| search_range.rfind(' '))
            .unwrap_or(max_len);

        // Ensure we don't split in the middle of a UTF-8 character
        let split_pos = crate::truncate_utf8(&remaining[..split_pos], split_pos).len();
        if split_pos == 0 {
            // Edge case: single character wider than max_len (shouldn't happen with 4096)
            break;
        }

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
}
