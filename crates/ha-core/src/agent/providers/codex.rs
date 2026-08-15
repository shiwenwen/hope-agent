//! Codex (ChatGPT subscription via OAuth) entry point — thin wrapper around
//! [`AssistantAgent::run_streaming_chat`] with the Codex adapter.
//!
//! Codex uses the same wire protocol as OpenAI Responses but with a fixed
//! endpoint, OAuth-based auth, and an internal retry loop for transient
//! network / 5xx errors. See [`super::codex_adapter`] for details.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use super::super::content::build_user_content_responses;
use super::super::streaming_loop::CurrentUserMessageState;
use super::super::types::{AssistantAgent, Attachment};
use super::codex_adapter::CodexStreamingAdapter;

impl AssistantAgent {
    pub(crate) async fn chat_openai(
        &self,
        access_token: &str,
        account_id: &str,
        model: &str,
        message: &str,
        attachments: &[Attachment],
        current_user_message_state: CurrentUserMessageState,
        reasoning_effort: Option<&str>,
        cancel: &Arc<AtomicBool>,
        on_delta: &(impl Fn(&str) + Send + Sync),
    ) -> Result<(String, Option<String>)> {
        let reasoning = self.resolve_reasoning_config(model, reasoning_effort).await;
        let adapter = CodexStreamingAdapter {
            access_token,
            account_id,
            model,
            reasoning,
        };
        let user_content = build_user_content_responses(
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
