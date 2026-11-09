//! Request-local native-history forks for Tier 3 summaries.
//!
//! A template is an opportunity to reuse a prompt prefix, not evidence of a
//! cache hit. It is never persisted, logged, or used as an authorization token.

use std::sync::Arc;

use serde_json::{json, Value};

use super::streaming_adapter::PreparedProviderRequest;
use super::types::{AssistantAgent, LlmProvider, ProviderFormat};
use crate::token_accounting::{TokenCountRequest, TokenCountUnknown};

const MAX_TEMPLATE_BYTES: usize = 8 * 1024 * 1024;

pub struct SummaryForkTemplate {
    body: Value,
    source: Vec<Value>,
    history_start: usize,
    format: ProviderFormat,
    tenant: String,
}

/// A complete, locally counted, text-only summary request. Sensitive fields
/// deliberately have no Debug/Display/Serialize implementation.
pub struct SummaryForkRequest {
    body: Value,
    format: ProviderFormat,
    tenant: String,
    pub(crate) input_upper: u64,
}

fn history_key(format: ProviderFormat) -> &'static str {
    match format {
        ProviderFormat::Anthropic | ProviderFormat::OpenAIChat => "messages",
        ProviderFormat::OpenAIResponses | ProviderFormat::Codex => "input",
    }
}

fn native_history(format: ProviderFormat, source: &[Value]) -> Vec<Value> {
    let source = crate::context_compact::prepare_messages_for_api(source);
    match format {
        ProviderFormat::OpenAIResponses | ProviderFormat::Codex => {
            AssistantAgent::normalize_history_for_responses(&source)
        }
        _ => source,
    }
}

/// Compare provider-visible content while ignoring only transport cache
/// annotations at supported content-block positions. Tool inputs/arguments and
/// nested user JSON remain exact; no arbitrary recursive field removal.
fn comparison_items(format: ProviderFormat, items: &[Value]) -> Vec<Value> {
    let mut items = items.to_vec();
    for item in &mut items {
        let field = if item.get("type").and_then(Value::as_str) == Some("function_call_output") {
            "output"
        } else {
            "content"
        };
        let text_type = match format {
            ProviderFormat::Anthropic => Some("text"),
            ProviderFormat::OpenAIResponses
                if field == "output"
                    || item.get("role").and_then(Value::as_str) == Some("user") =>
            {
                Some("input_text")
            }
            _ => None,
        };
        if let (Some(kind), Some(text)) = (text_type, item.get(field).and_then(Value::as_str)) {
            item[field] = json!([{"type":kind,"text":text}]);
        }
        if let Some(parts) = item.get_mut(field).and_then(Value::as_array_mut) {
            for part in parts {
                if let Some(part) = part.as_object_mut() {
                    match format {
                        ProviderFormat::Anthropic => {
                            part.remove("cache_control");
                        }
                        ProviderFormat::OpenAIResponses => {
                            part.remove("prompt_cache_breakpoint");
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    items
}

fn local_function_tools_only(body: &Value, format: ProviderFormat) -> bool {
    body.get("tools").is_none_or(|tools| {
        tools.as_array().is_some_and(|tools| {
            tools.iter().all(|tool| match format {
                ProviderFormat::Anthropic => {
                    tool.get("type").is_none()
                        && tool.get("input_schema").is_some_and(Value::is_object)
                }
                _ => tool.get("type").and_then(Value::as_str) == Some("function"),
            })
        })
    })
}

impl SummaryForkTemplate {
    #[doc(hidden)]
    pub fn from_prepared(
        provider: &LlmProvider,
        prepared: &PreparedProviderRequest,
        source: &[Value],
    ) -> Option<Self> {
        if matches!(provider, LlmProvider::OpenAIChat { base_url, model, .. } if super::config::is_direct_openai_astra(base_url, model))
        {
            // The independent Astra Chat adapter intentionally pins a different
            // background effort. Keep that compatibility policy out of forks.
            return None;
        }
        if prepared.body().len() > MAX_TEMPLATE_BYTES {
            return None;
        }
        let format = ProviderFormat::from(provider);
        let mut body: Value = serde_json::from_slice(&prepared.body()).ok()?;
        if body.get("model").and_then(Value::as_str) != Some(provider.model())
            || !local_function_tools_only(&body, format)
            || ["previous_response_id", "conversation", "context_management"]
                .iter()
                .any(|field| body.get(field).is_some_and(|value| !value.is_null()))
            || body
                .get("tool_choice")
                .is_some_and(|choice| !matches!(choice.as_str(), Some("auto" | "none")))
            || ["/response_format/type", "/text/format/type"]
                .iter()
                .any(|path| {
                    body.pointer(path)
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind != "text")
                })
        {
            return None;
        }
        let mut source = comparison_items(format, &native_history(format, source));
        let first = source.first()?;
        let history = body.get_mut(history_key(format))?.as_array_mut()?;
        let comparison = comparison_items(format, history);
        // Keep only items actually unchanged by the provider renderer. A
        // dynamic user-data envelope can extend the last user item; that item
        // is intentionally ineligible rather than reconstructed approximately.
        let (history_start, matched) = comparison
            .iter()
            .enumerate()
            .filter(|(_, item)| *item == first)
            .map(|(start, _)| {
                let matched = comparison[start..]
                    .iter()
                    .zip(&source)
                    .take_while(|(rendered, source)| rendered == source)
                    .count();
                (start, matched)
            })
            .max_by_key(|(_, matched)| *matched)?;
        source.truncate(matched);
        history.truncate(history_start + matched);
        Some(Self {
            body,
            source,
            history_start,
            format,
            tenant: provider.cache_tenant_partition(),
        })
    }

    #[doc(hidden)]
    pub fn build(
        &self,
        source: &[Value],
        instruction: &str,
        max_tokens: u32,
        context_window: u32,
    ) -> Option<SummaryForkRequest> {
        let source = comparison_items(self.format, &native_history(self.format, source));
        if source.is_empty() || !self.source.starts_with(&source) || context_window == 0 {
            return None;
        }
        // Keep manual thinking budgets unchanged for prefix affinity. If the
        // requested summary output cannot contain that budget, use the existing
        // independent summarizer instead of silently changing either limit.
        if self
            .body
            .pointer("/thinking/budget_tokens")
            .and_then(Value::as_u64)
            .is_some_and(|budget| budget >= u64::from(max_tokens))
        {
            return None;
        }
        let mut body = self.body.clone();
        let explicit_responses = self.format == ProviderFormat::OpenAIResponses
            && body
                .pointer("/prompt_cache_options/mode")
                .and_then(Value::as_str)
                == Some("explicit");
        let history = body.get_mut(history_key(self.format))?.as_array_mut()?;
        history.truncate(self.history_start + source.len());
        if self.format == ProviderFormat::Anthropic {
            super::streaming_adapter::mark_anthropic_history_cache_boundary(history);
        } else if explicit_responses {
            super::streaming_adapter::mark_responses_history_cache_boundaries(history);
        }
        // Never merge the directive into the last cached user/tool item.
        history.push(json!({"role": "user", "content": instruction}));
        let history = history.clone();
        let object = body.as_object_mut()?;
        match self.format {
            ProviderFormat::Anthropic | ProviderFormat::OpenAIChat => {
                object.insert("max_tokens".into(), json!(max_tokens));
            }
            ProviderFormat::OpenAIResponses => {
                object.insert("max_output_tokens".into(), json!(max_tokens));
            }
            ProviderFormat::Codex => {}
        }
        object.insert("stream".into(), json!(self.format == ProviderFormat::Codex));
        if self.format != ProviderFormat::Codex {
            object.remove("stream_options");
        }
        // Count all fixed fields, history and actual native tool schemas once.
        // Media/unknown protocol content is not an optimization candidate.
        let mut fixed = body.clone();
        let fixed_object = fixed.as_object_mut()?;
        fixed_object.remove(history_key(self.format));
        let tools = fixed_object
            .remove("tools")
            .and_then(|tools| tools.as_array().cloned())
            .unwrap_or_default();
        let fixed = serde_json::to_string(&fixed).ok()?;
        let model = body.get("model")?.as_str()?;
        let count = crate::token_accounting::service().count_local(&TokenCountRequest {
            provider: self.format.token_provider_family(),
            model,
            request_shape: self.format.token_request_shape(),
            stable_prompt: &fixed,
            dynamic_prompt: "",
            history: &history,
            eager_tool_schemas: &tools,
            activated_tool_schemas: &[],
        });
        if count
            .unknowns
            .iter()
            .any(|unknown| !matches!(unknown, TokenCountUnknown::TokenizerUnavailable))
        {
            return None;
        }
        let safety = u64::from(context_window / 50).max(1_024);
        if count
            .upper_bound
            .saturating_add(u64::from(max_tokens))
            .saturating_add(safety)
            > u64::from(context_window)
        {
            return None;
        }
        Some(SummaryForkRequest {
            body,
            format: self.format,
            tenant: self.tenant.clone(),
            input_upper: count.upper_bound,
        })
    }
}

impl SummaryForkRequest {
    pub fn matches_provider(&self, provider: &LlmProvider) -> bool {
        self.format == ProviderFormat::from(provider)
            && self.body.get("model").and_then(Value::as_str) == Some(provider.model())
            && self.tenant == provider.cache_tenant_partition()
    }

    pub fn body_for(&self, format: ProviderFormat, model: &str) -> Option<Value> {
        (self.format == format && self.body.get("model").and_then(Value::as_str) == Some(model))
            .then(|| self.body.clone())
    }
}

impl AssistantAgent {
    #[doc(hidden)]
    pub fn summary_fork_enabled(&self) -> bool {
        self.compact_config.enabled
            && self.context_window > 0
            && self.compaction_provider.is_none()
            && self
                .compact_config
                .effective_summarization_model_ref()
                .is_none()
            && self.compact_config.summary_reasoning_effort.is_none()
            && !matches!(&self.provider, LlmProvider::Codex { account_id, .. } if account_id.is_empty())
    }

    #[doc(hidden)]
    pub fn clear_summary_fork_template(&self) {
        *self
            .summary_fork_template
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Called only with a newly prepared main-request template. The stored
    /// opportunity stays on this agent and cannot cross sessions or failover.
    #[doc(hidden)]
    pub fn save_summary_fork_template(&self, prepared: &PreparedProviderRequest, source: &[Value]) {
        let template = self
            .summary_fork_enabled()
            .then(|| SummaryForkTemplate::from_prepared(&self.provider, prepared, source))
            .flatten()
            .map(Arc::new);
        *self
            .summary_fork_template
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = template;
    }

    pub(crate) fn prepare_summary_fork(&self, source: &[Value]) -> Option<Arc<SummaryForkRequest>> {
        if !self.summary_fork_enabled() {
            return None;
        }
        let template = self
            .summary_fork_template
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        template
            .build(
                source,
                &crate::context_compact::build_summary_fork_instruction(&self.compact_config),
                self.compact_config.summary_max_tokens,
                self.context_window,
            )
            .map(Arc::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::streaming_adapter::{
        PreparedRequestVariant, ProviderEndpointKind, ProviderRequestShape,
    };

    static_assertions::assert_not_impl_any!(SummaryForkTemplate: std::fmt::Debug, serde::Serialize, std::fmt::Display);
    static_assertions::assert_not_impl_any!(SummaryForkRequest: std::fmt::Debug, serde::Serialize, std::fmt::Display);

    fn provider(format: ProviderFormat) -> LlmProvider {
        match format {
            ProviderFormat::Anthropic => LlmProvider::Anthropic {
                api_key: "synthetic-key".into(),
                base_url: "https://synthetic.example".into(),
                model: "test-model".into(),
                workspace_id: Some("synthetic-workspace".into()),
            },
            ProviderFormat::OpenAIChat => LlmProvider::OpenAIChat {
                api_key: "synthetic-key".into(),
                base_url: "https://synthetic.example".into(),
                model: "test-model".into(),
            },
            ProviderFormat::OpenAIResponses => LlmProvider::OpenAIResponses {
                api_key: "synthetic-key".into(),
                base_url: "https://synthetic.example".into(),
                model: "test-model".into(),
            },
            ProviderFormat::Codex => LlmProvider::Codex {
                access_token: "synthetic-token".into(),
                account_id: "synthetic-account".into(),
                model: "test-model".into(),
            },
        }
    }

    fn prepared(body: &Value, format: ProviderFormat) -> PreparedProviderRequest {
        let (endpoint, shape, variant) = match format {
            ProviderFormat::Anthropic => (
                ProviderEndpointKind::AnthropicMessages,
                ProviderRequestShape::AnthropicMessages,
                PreparedRequestVariant::Anthropic,
            ),
            ProviderFormat::OpenAIChat => (
                ProviderEndpointKind::OpenAIChatCompletions,
                ProviderRequestShape::OpenAIChatCompletions,
                PreparedRequestVariant::OpenAIChat {
                    thinking_parameters_disabled: false,
                    reasoning_output_disabled: false,
                    model_supports_vision: false,
                    prompt_cache_key_included: true,
                    proactive_vision_notice: false,
                },
            ),
            ProviderFormat::OpenAIResponses => (
                ProviderEndpointKind::OpenAIResponses,
                ProviderRequestShape::OpenAIResponses,
                PreparedRequestVariant::OpenAIResponses,
            ),
            ProviderFormat::Codex => (
                ProviderEndpointKind::CodexResponses,
                ProviderRequestShape::CodexResponses,
                PreparedRequestVariant::Codex,
            ),
        };
        PreparedProviderRequest::from_json(
            endpoint,
            shape,
            "test-model",
            0,
            None,
            Some("medium"),
            false,
            variant,
            body,
        )
        .unwrap()
    }

    fn fixture(format: ProviderFormat) -> (LlmProvider, Vec<Value>, Value) {
        let mut source =
            vec![json!({"role":"user","content":"original request /workspace/exact.rs"})];
        match format {
            ProviderFormat::Anthropic => source.extend([
                json!({"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"read","input":{"path":"/workspace/exact.rs"}}]}),
                json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"untrusted tool result"}]}),
            ]),
            ProviderFormat::OpenAIChat => source.extend([
                json!({"role":"assistant","tool_calls":[{"id":"call-1","type":"function","function":{"name":"read","arguments":"{\"path\":\"/workspace/exact.rs\"}"}}]}),
                json!({"role":"tool","tool_call_id":"call-1","content":"untrusted tool result"}),
            ]),
            _ => source.extend([
                json!({"type":"function_call","call_id":"call-1","name":"read","arguments":"{\"path\":\"/workspace/exact.rs\"}"}),
                json!({"type":"function_call_output","call_id":"call-1","output":"untrusted tool result"}),
            ]),
        }
        source.push(json!({"role":"assistant","content":"completed old turn"}));
        source.push(json!({"role":"user","content":"protected recent turn"}));
        let mut history = native_history(format, &source);
        history.last_mut().unwrap()["content"] =
            json!("protected recent turn plus changing dynamic data");
        let mut body = json!({"model":"test-model","stream":true,"prompt_cache_key":"same-route","temperature":0.2});
        match format {
            ProviderFormat::Anthropic => {
                body["system"] = json!([{"type":"text","text":"stable system","cache_control":{"type":"ephemeral"}},{"type":"text","text":"trusted run framing"}]);
                body["tools"] = json!([{"name":"read","input_schema":{"type":"object"},"cache_control":{"type":"ephemeral"}}]);
                body["thinking"] = json!({"type":"adaptive"});
                body["output_config"] = json!({"effort":"medium"});
                body["max_tokens"] = json!(8192);
            }
            ProviderFormat::OpenAIChat => {
                history.insert(0, json!({"role":"system","content":"stable system"}));
                body["tools"] = json!([{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}]);
                body["reasoning_effort"] = json!("medium");
                body["max_tokens"] = json!(8192);
            }
            _ => {
                body["instructions"] = json!("stable system");
                body["tools"] =
                    json!([{"type":"function","name":"read","parameters":{"type":"object"}}]);
                body["reasoning"] = json!({"effort":"medium","summary":"auto"});
                body["store"] = json!(false);
            }
        }
        body[history_key(format)] = json!(history);
        (provider(format), source, body)
    }

    #[test]
    fn summary_fork_preserves_native_prefix_and_omits_protected_tail_for_all_protocols() {
        for format in [
            ProviderFormat::Anthropic,
            ProviderFormat::OpenAIChat,
            ProviderFormat::OpenAIResponses,
            ProviderFormat::Codex,
        ] {
            let (provider, source, body) = fixture(format);
            let prepared = prepared(&body, format);
            let template =
                SummaryForkTemplate::from_prepared(&provider, &prepared, &source).unwrap();
            let fork = template
                .build(&source[..4], "checkpoint directive", 4096, 100_000)
                .unwrap();
            let result = fork.body_for(format, "test-model").unwrap();
            let offset = usize::from(format == ProviderFormat::OpenAIChat);
            assert_eq!(
                comparison_items(
                    format,
                    &result[history_key(format)].as_array().unwrap()[..offset + 4]
                ),
                comparison_items(
                    format,
                    &body[history_key(format)].as_array().unwrap()[..offset + 4]
                )
            );
            assert_eq!(
                result[history_key(format)].as_array().unwrap().len(),
                offset + 5
            );
            assert_eq!(
                result[history_key(format)][offset + 4]["content"],
                "checkpoint directive"
            );
            for field in [
                "system",
                "instructions",
                "tools",
                "thinking",
                "output_config",
                "reasoning",
                "reasoning_effort",
                "temperature",
                "prompt_cache_key",
            ] {
                assert_eq!(
                    result.get(field),
                    body.get(field),
                    "changed {field} for {format:?}"
                );
            }
            assert!(!result.to_string().contains("protected recent turn"));
            assert_eq!(
                result.to_string().matches("untrusted tool result").count(),
                1
            );
            assert!(fork.matches_provider(&provider));
            assert_eq!(prepared.body().as_ref(), serde_json::to_vec(&body).unwrap());
        }
    }

    #[test]
    fn summary_fork_refuses_stale_source_and_modified_dynamic_tail() {
        let (provider, source, body) = fixture(ProviderFormat::Anthropic);
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        let mut changed = source[..4].to_vec();
        changed[0]["content"] = json!("changed request");
        assert!(template
            .build(&changed, "checkpoint", 4096, 100_000)
            .is_none());
        assert!(template
            .build(&source, "checkpoint", 4096, 100_000)
            .is_none());
        assert!(template.build(&[], "checkpoint", 4096, 100_000).is_none());
    }

    #[test]
    fn summary_fork_refuses_capacity_overflow_and_incompatible_thinking_budget() {
        let (provider, source, mut body) = fixture(ProviderFormat::Anthropic);
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        assert!(template
            .build(&source[..4], "checkpoint", 4096, 4096)
            .is_none());
        assert!(template
            .build(&source[..4], "checkpoint", 4096, 0)
            .is_none());
        body["thinking"] = json!({"type":"enabled","budget_tokens":4096});
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        assert!(template
            .build(&source[..4], "checkpoint", 4096, 100_000)
            .is_none());
        assert!(template
            .build(&source[..4], "checkpoint", 8192, 100_000)
            .is_some());
    }

    #[test]
    fn summary_fork_cannot_cross_account_workspace_model_or_protocol() {
        let (provider, source, body) = fixture(ProviderFormat::Anthropic);
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        let fork = template
            .build(&source[..4], "checkpoint", 4096, 100_000)
            .unwrap();
        for (key, workspace, model) in [
            ("other-key", "synthetic-workspace", "test-model"),
            ("synthetic-key", "other-workspace", "test-model"),
            ("synthetic-key", "synthetic-workspace", "other-model"),
        ] {
            assert!(!fork.matches_provider(&LlmProvider::Anthropic {
                api_key: key.into(),
                base_url: "https://synthetic.example".into(),
                model: model.into(),
                workspace_id: Some(workspace.into())
            }));
        }
        assert!(fork
            .body_for(ProviderFormat::OpenAIChat, "test-model")
            .is_none());
        assert!(fork
            .body_for(ProviderFormat::Anthropic, "other-model")
            .is_none());
    }

    #[test]
    fn summary_fork_rejects_provider_tools_server_state_and_forced_output_contracts() {
        let (provider, source, body) = fixture(ProviderFormat::OpenAIResponses);
        for (field, value) in [
            ("tools", json!([{"type":"web_search"}])),
            ("tools", json!([{"type":"tool_search"}])),
            (
                "previous_response_id",
                json!("response-from-another-prefix"),
            ),
            ("tool_choice", json!("required")),
            ("text", json!({"format":{"type":"json_schema"}})),
        ] {
            let mut changed = body.clone();
            changed[field] = value;
            assert!(
                SummaryForkTemplate::from_prepared(
                    &provider,
                    &prepared(&changed, ProviderFormat::OpenAIResponses),
                    &source
                )
                .is_none(),
                "accepted {field}"
            );
        }
    }

    #[test]
    fn summary_fork_keeps_previous_summary_once_and_refuses_media() {
        let (provider, mut source, _) = fixture(ProviderFormat::Anthropic);
        source[0]["content"] = json!("[Previous conversation summary]\n\nprior decisions");
        let mut body = json!({"model":"test-model","system":"stable","messages":source});
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        let fork = template
            .build(&source[..4], "checkpoint", 4096, 100_000)
            .unwrap();
        assert_eq!(fork.body.to_string().matches("prior decisions").count(), 1);
        source[0]["content"] = json!([{"type":"image","source":{"type":"base64","media_type":"image/png","data":"synthetic"}}]);
        body["messages"] = json!(source);
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        assert!(template
            .build(&source[..4], "checkpoint", 4096, 100_000)
            .is_none());
    }

    #[test]
    fn summary_fork_honors_dedicated_model_and_explicit_reasoning_settings() {
        let mut agent = AssistantAgent::new_anthropic("synthetic");
        assert!(agent.summary_fork_enabled());
        agent.compact_config.summary_reasoning_effort =
            Some(crate::context_compact::SummaryReasoningEffort::Low);
        assert!(!agent.summary_fork_enabled());
        agent.compact_config.summary_reasoning_effort = None;
        agent.compact_config.summarization_model = Some("dedicated:model".into());
        assert!(!agent.summary_fork_enabled());
    }

    #[test]
    fn summary_fork_cache_annotations_do_not_hide_changes_to_untrusted_tool_arguments() {
        let (provider, source, mut body) = fixture(ProviderFormat::Anthropic);
        body["messages"][1]["content"][0]["input"]["cache_control"] =
            json!({"untrusted_value":"original"});
        let mut source = source;
        source[1]["content"][0]["input"]["cache_control"] = json!({"untrusted_value":"original"});
        let template = SummaryForkTemplate::from_prepared(
            &provider,
            &prepared(&body, ProviderFormat::Anthropic),
            &source,
        )
        .unwrap();
        source[1]["content"][0]["input"]["cache_control"] = json!({"untrusted_value":"changed"});
        assert!(template
            .build(&source[..4], "checkpoint", 4096, 100_000)
            .is_none());
    }

    #[test]
    fn summary_fork_template_is_cleared_when_session_or_history_is_replaced() {
        let (provider, source, body) = fixture(ProviderFormat::Anthropic);
        let mut agent = AssistantAgent::new_anthropic("synthetic");
        agent.provider = provider;
        let prepared = prepared(&body, ProviderFormat::Anthropic);
        agent.save_summary_fork_template(&prepared, &source);
        assert!(agent.prepare_summary_fork(&source[..4]).is_some());
        agent.set_session_id("new-synthetic-session");
        assert!(agent.prepare_summary_fork(&source[..4]).is_none());
        agent.save_summary_fork_template(&prepared, &source);
        assert!(agent.prepare_summary_fork(&source[..4]).is_some());
        agent.set_conversation_history(Vec::new());
        assert!(agent.prepare_summary_fork(&source[..4]).is_none());
    }
}
