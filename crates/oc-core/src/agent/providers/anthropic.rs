//! Anthropic Messages API entry point — thin wrapper around
//! [`AssistantAgent::run_streaming_chat`] with the Anthropic adapter.
//!
//! All body construction, SSE decoding, and history persistence live in
//! [`super::anthropic_adapter`]. The shared tool loop (compaction, tool
//! dispatch, microcompact, event emit) lives in [`super::super::streaming_loop`].

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use super::super::content::build_user_content_anthropic;
use super::super::types::AssistantAgent;
use super::anthropic_adapter::AnthropicStreamingAdapter;

impl AssistantAgent {
    pub(crate) async fn chat_anthropic(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        message: &str,
        attachments: &[super::super::types::Attachment],
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send + Sync),
    ) -> Result<(String, Option<String>)> {
        let adapter = AnthropicStreamingAdapter {
            api_key,
            base_url,
            model,
        };
        let user_content = build_user_content_anthropic(message, attachments);
        self.run_streaming_chat(
            &adapter,
            model,
            message,
            user_content,
            reasoning_effort,
            cancel,
            on_delta,
        )
        .await
    }
}
