//! OpenAI Chat Completions API entry point — thin wrapper around
//! [`AssistantAgent::run_streaming_chat`] with the OpenAI Chat adapter.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use super::super::content::build_user_content_openai_chat;
use super::super::streaming_loop::CurrentUserMessageState;
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
        current_user_message_state: CurrentUserMessageState,
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send + Sync),
    ) -> Result<(String, Option<String>)> {
        let adapter = OpenAIChatStreamingAdapter {
            api_key,
            base_url,
            model,
            thinking_style: &self.thinking_style,
            provider_config: self.provider_config.as_deref(),
            vision_runtime_disabled: Arc::new(AtomicBool::new(false)),
            vision_notice_emitted: Arc::new(AtomicBool::new(false)),
            prepared_history_had_images: AtomicBool::new(false),
        };
        let user_content = build_user_content_openai_chat(
            message,
            attachments,
            self.get_context_window(),
            &self.context_resource_refs,
        );
        self.run_streaming_chat(
            &adapter,
            model,
            message,
            user_content,
            current_user_message_state,
            reasoning_effort,
            cancel,
            on_delta,
        )
        .await
    }
}
