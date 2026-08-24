use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::streaming_loop::RuntimeAgentExt as _;

pub(crate) async fn run_agent_chat(
    agent: &ha_core::agent::AssistantAgent,
    message: &str,
    attachments: &[ha_core::agent::Attachment],
    current_user_message_state: super::streaming_loop::CurrentUserMessageState,
    reasoning_effort: Option<&str>,
    cancel: Arc<AtomicBool>,
    on_delta: impl Fn(&str) + Send + Sync + 'static,
) -> anyhow::Result<(String, Option<String>)> {
    let (provider_type, model_name) = match agent.runtime_provider() {
        ha_core::agent::LlmProvider::Anthropic { model, .. } => ("Anthropic", model.as_str()),
        ha_core::agent::LlmProvider::OpenAIChat { model, .. } => ("OpenAIChat", model.as_str()),
        ha_core::agent::LlmProvider::OpenAIResponses { model, .. } => {
            ("OpenAIResponses", model.as_str())
        }
        ha_core::agent::LlmProvider::Codex { model, .. } => ("Codex", model.as_str()),
    };
    if let Some(logger) = ha_core::get_logger() {
        logger.log(
            "info",
            "agent",
            "agent::chat",
            &format!(
                "Agent chat dispatching: provider={}, model={}",
                provider_type, model_name
            ),
            Some(
                serde_json::json!({
                    "provider_type": provider_type,
                    "model": model_name,
                    "reasoning_effort": reasoning_effort,
                    "attachments": attachments.len(),
                    "history_messages": agent.runtime_history_len(),
                    "message_bytes": message.len(),
                    "message_fingerprint": ha_core::audit_fingerprint(
                        "agent-chat-message",
                        message.as_bytes(),
                    ),
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    match agent.runtime_provider() {
        ha_core::agent::LlmProvider::Anthropic {
            api_key,
            base_url,
            model,
        } => {
            let adapter = super::provider_adapters::anthropic_adapter::AnthropicStreamingAdapter {
                api_key,
                base_url,
                model,
            };
            let user_content = ha_core::agent::content::build_user_content_anthropic(
                message,
                attachments,
                agent.get_context_window(),
                agent.runtime_context_resource_refs(),
            );
            agent
                .run_streaming_chat(
                    &adapter,
                    model,
                    message,
                    user_content,
                    current_user_message_state,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
        }
        ha_core::agent::LlmProvider::OpenAIChat {
            api_key,
            base_url,
            model,
        } => {
            let adapter =
                super::provider_adapters::openai_chat_adapter::OpenAIChatStreamingAdapter {
                    api_key,
                    base_url,
                    model,
                    thinking_style: agent.runtime_thinking_style(),
                    provider_config: agent.runtime_provider_config(),
                    vision_runtime_disabled: Arc::new(AtomicBool::new(false)),
                    vision_notice_emitted: Arc::new(AtomicBool::new(false)),
                    prepared_history_had_images: AtomicBool::new(false),
                };
            let user_content = ha_core::agent::content::build_user_content_openai_chat(
                message,
                attachments,
                agent.get_context_window(),
                agent.runtime_context_resource_refs(),
            );
            agent
                .run_streaming_chat(
                    &adapter,
                    model,
                    message,
                    user_content,
                    current_user_message_state,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
        }
        ha_core::agent::LlmProvider::OpenAIResponses {
            api_key,
            base_url,
            model,
        } => {
            let reasoning = agent
                .resolve_reasoning_config(model, reasoning_effort)
                .await;
            let adapter =
                super::provider_adapters::openai_responses_adapter::OpenAIResponsesStreamingAdapter {
                    api_key,
                    base_url,
                    model,
                    reasoning,
                };
            let user_content = ha_core::agent::content::build_user_content_responses(
                message,
                attachments,
                agent.get_context_window(),
                agent.runtime_context_resource_refs(),
            );
            agent
                .run_streaming_chat(
                    &adapter,
                    model,
                    message,
                    user_content,
                    current_user_message_state,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
        }
        ha_core::agent::LlmProvider::Codex {
            access_token,
            account_id,
            model,
        } => {
            let reasoning = agent
                .resolve_reasoning_config(model, reasoning_effort)
                .await;
            let adapter = super::provider_adapters::codex_adapter::CodexStreamingAdapter {
                access_token,
                account_id,
                model,
                reasoning,
            };
            let user_content = ha_core::agent::content::build_user_content_responses(
                message,
                attachments,
                agent.get_context_window(),
                agent.runtime_context_resource_refs(),
            );
            agent
                .run_streaming_chat(
                    &adapter,
                    model,
                    message,
                    user_content,
                    current_user_message_state,
                    reasoning_effort,
                    &cancel,
                    &on_delta,
                )
                .await
        }
    }
}
