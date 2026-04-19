//! OpenAI Responses API entry point — thin wrapper around
//! [`AssistantAgent::run_streaming_chat`] with the Responses adapter.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use super::super::content::build_user_content_responses;
use super::super::types::{AssistantAgent, Attachment};
use super::openai_responses_adapter::OpenAIResponsesStreamingAdapter;

impl AssistantAgent {
    pub(crate) async fn chat_openai_responses(
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
        // resolve_reasoning_config is async (reads live config) — must run
        // before constructing the adapter so the per-turn reasoning level
        // is captured into the adapter struct for SSE include hint.
        let reasoning = self.resolve_reasoning_config(model, reasoning_effort).await;
        let adapter = OpenAIResponsesStreamingAdapter {
            api_key,
            base_url,
            model,
            reasoning,
        };
        let user_content = build_user_content_responses(message, attachments);
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
