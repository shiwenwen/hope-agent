//! OpenAI Chat Completions API entry point — thin wrapper around
//! [`AssistantAgent::run_streaming_chat`] with the OpenAI Chat adapter.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use super::super::content::build_user_content_openai_chat;
use super::super::types::{AssistantAgent, Attachment};
use super::openai_chat_adapter::OpenAIChatStreamingAdapter;

impl AssistantAgent {
    pub(crate) async fn chat_openai_chat(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        message: &str,
        attachments: &[Attachment],
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send + Sync),
    ) -> Result<(String, Option<String>)> {
        let adapter = OpenAIChatStreamingAdapter {
            api_key,
            base_url,
            model,
            thinking_style: &self.thinking_style,
        };
        let user_content = build_user_content_openai_chat(message, attachments);
        self.run_streaming_chat(
            &adapter,
            model,
            "OpenAIChat",
            message,
            user_content,
            reasoning_effort,
            cancel,
            on_delta,
        )
        .await
    }
}
