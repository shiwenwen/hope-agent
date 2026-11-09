//! Explicit compaction-only policy. Never omit a requested off switch, clamp an
//! unsupported level, increase the output ceiling, or alter other one-shots.

use super::*;
use ha_core::context_compact::SummaryReasoningEffort;
use ha_core::provider::ThinkingStyle;

fn unsupported() -> anyhow::Error {
    ha_core::failover::ProviderRequestContractError(
        "Summary reasoning override is unsupported for this provider/model or output budget. Select the default policy or a supported effort/model; the existing history is preserved."
            .into(),
    )
    .into()
}

pub(super) fn classify_summary_error(
    explicit_summary: bool,
    error: anyhow::Error,
) -> anyhow::Error {
    if explicit_summary
        && error
            .downcast_ref::<ha_core::failover::ProviderApiError>()
            .is_some_and(|error| error.rejects_reasoning_parameter())
    {
        // Do not carry untrusted response text into the public error chain.
        unsupported()
    } else {
        error
    }
}

fn effort_name(effort: SummaryReasoningEffort) -> &'static str {
    use SummaryReasoningEffort as E;
    match effort {
        E::None => "none",
        E::Minimal => "minimal",
        E::Low => "low",
        E::Medium => "medium",
        E::High => "high",
        E::Xhigh => "xhigh",
        E::Max => "max",
    }
}

fn is_direct_host(base_url: &str, hosts: &[&str]) -> bool {
    reqwest::Url::parse(base_url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| hosts.contains(&host))
    })
}

fn manual_thinking(body: &mut Value, effort: &str, max_tokens: u32) -> Result<()> {
    if effort == "none" {
        body["thinking"] = json!({"type": "disabled"});
        return Ok(());
    }
    let budget = match effort {
        "low" => 1024,
        "medium" => 4096,
        "high" => 8192,
        "xhigh" | "max" => 16384,
        _ => return Err(unsupported()),
    };
    // A small ceiling must fail explicitly, never grow silently or be reduced
    // to a budget that consumes almost the entire answer allocation.
    if max_tokens <= budget {
        return Err(unsupported());
    }
    body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    Ok(())
}

pub(super) fn apply_summary_reasoning(
    base_url: &str,
    model: &str,
    format: ProviderFormat,
    req: &OneShotRequest<'_>,
    body: &mut Value,
) -> Result<()> {
    let OneShotMode::Summary {
        effort,
        thinking_style,
        ..
    } = &req.mode
    else {
        return Ok(());
    };
    let effort = effort_name(*effort);
    if effort != "none" && matches!(thinking_style, ThinkingStyle::None) {
        return Err(unsupported());
    }
    if crate::agent::config::is_direct_openai_gpt6(base_url, model)
        && crate::agent::config::gpt6_api_reasoning_effort(model, Some(effort)) != Some(effort)
    {
        return Err(unsupported());
    }
    match format {
        ProviderFormat::OpenAIChat => match thinking_style {
            ThinkingStyle::Openai => {
                if crate::agent::config::is_direct_deepseek(base_url, model) {
                    body["thinking"] =
                        json!({"type": if effort == "none" { "disabled" } else { "enabled" }});
                    if effort != "none" {
                        let mapped = match effort {
                            "minimal" | "low" => "low",
                            "max" => "max",
                            _ => "high",
                        };
                        body["reasoning_effort"] = json!(mapped);
                    } else {
                        body.as_object_mut()
                            .expect("chat body")
                            .remove("reasoning_effort");
                    }
                } else {
                    // Compatible gateways use the explicitly selected wire style.
                    // Do not run the generic clamp that turns none into absence.
                    body["reasoning_effort"] = json!(effort);
                }
            }
            ThinkingStyle::Anthropic => manual_thinking(body, effort, req.max_tokens)?,
            ThinkingStyle::Zai => {
                let direct = is_direct_host(base_url, &["api.z.ai", "open.bigmodel.cn"]);
                if direct
                    && matches!(model, "glm-5.3" | "glm-5.3-flash")
                    && !matches!(effort, "low" | "high" | "max")
                {
                    return Err(unsupported());
                }
                body["thinking"] =
                    json!({"type": if effort == "none" { "disabled" } else { "enabled" }});
                if effort != "none" {
                    body["reasoning_effort"] = json!(effort);
                }
            }
            ThinkingStyle::Qwen if effort == "none" => {
                body["enable_thinking"] = json!(false);
            }
            // The configured Qwen style is an on/off contract, not a graded
            // effort contract. A None style must never silently discard an override.
            ThinkingStyle::Qwen | ThinkingStyle::None => return Err(unsupported()),
        },
        ProviderFormat::OpenAIResponses => {
            body["reasoning"] = json!({"effort": effort});
        }
        ProviderFormat::Codex => {
            // The consumer OAuth backend has its own contract. Do not infer
            // an off switch from the public Responses API's model capabilities.
            let supported = if crate::agent::config::is_gpt6_model(model) {
                crate::agent::config::astra_reasoning_effort(Some(effort)) == effort
            } else {
                crate::agent::config::clamp_reasoning_effort(model, effort).as_deref()
                    == Some(effort)
            };
            if effort == "none" || !supported {
                return Err(unsupported());
            }
            body["reasoning"] = json!({"effort": effort});
        }
        ProviderFormat::Anthropic => {
            let direct = crate::provider::is_direct_anthropic(base_url);
            let rejects_disabled = model.starts_with("claude-fable-5")
                || model.starts_with("claude-mythos-5")
                || model.starts_with("claude-opus-5-5")
                || model.starts_with("claude-sonnet-5-5");
            if direct && effort == "none" && rejects_disabled {
                return Err(unsupported());
            }
            if direct && effort != "none" && crate::agent::config::is_claude_5_model(model) {
                if crate::agent::config::clamp_reasoning_effort(model, effort).as_deref()
                    != Some(effort)
                {
                    return Err(unsupported());
                }
                body["thinking"] = json!({"type": "adaptive"});
                body["output_config"] = json!({"effort": effort});
            } else {
                manual_thinking(body, effort, req.max_tokens)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(effort: SummaryReasoningEffort, style: &ThinkingStyle) -> OneShotRequest<'_> {
        OneShotRequest {
            instruction: "summarize the supplied history",
            max_tokens: 4096,
            mode: OneShotMode::Summary {
                system: "summary system",
                effort,
                thinking_style: style,
            },
            user_content: None,
        }
    }

    #[test]
    fn ordinary_one_shots_keep_their_existing_policy() {
        let req = OneShotRequest {
            instruction: "title",
            max_tokens: 256,
            mode: OneShotMode::Independent {
                system: "title system",
            },
            user_content: None,
        };
        for format in [
            ProviderFormat::Anthropic,
            ProviderFormat::OpenAIChat,
            ProviderFormat::OpenAIResponses,
            ProviderFormat::Codex,
        ] {
            let mut body = json!({"reasoning":{"effort":"low"},"max_tokens":256});
            let original = body.clone();
            apply_summary_reasoning("https://synthetic.example", "test", format, &req, &mut body)
                .unwrap();
            assert_eq!(body, original);
        }
    }

    #[test]
    fn explicit_none_is_transmitted_for_chat_and_responses_without_budget_growth() {
        let style = ThinkingStyle::Openai;
        let req = request(SummaryReasoningEffort::None, &style);
        let mut chat = build_openai_chat_body("deepseek-v4.1-flash", &req);
        apply_summary_reasoning(
            "https://synthetic-gateway.example",
            "deepseek-v4.1-flash",
            ProviderFormat::OpenAIChat,
            &req,
            &mut chat,
        )
        .unwrap();
        assert_eq!(chat["reasoning_effort"], "none");
        assert_eq!(chat["max_tokens"], 4096);
        assert_eq!(chat["messages"][0]["content"], "summary system");
        assert_eq!(chat["messages"].as_array().unwrap().len(), 2);
        assert!(chat.get("tools").is_none());
        let mut responses = build_responses_body("test", &req, ProviderFormat::OpenAIResponses);
        apply_summary_reasoning(
            "https://synthetic-gateway.example",
            "test",
            ProviderFormat::OpenAIResponses,
            &req,
            &mut responses,
        )
        .unwrap();
        assert_eq!(responses["reasoning"]["effort"], "none");
        assert_eq!(responses["max_output_tokens"], 4096);
        assert_eq!(responses["instructions"], "summary system");
        assert_eq!(responses["input"].as_array().unwrap().len(), 1);
        assert!(responses.get("tools").is_none());
    }

    #[test]
    fn official_deepseek_uses_its_explicit_toggle_only_on_the_direct_endpoint() {
        let style = ThinkingStyle::Openai;
        let req = request(SummaryReasoningEffort::None, &style);
        let mut body = json!({"reasoning_effort":"low","max_tokens":4096});
        apply_summary_reasoning(
            "https://api.deepseek.com",
            "deepseek-v4-pro",
            ProviderFormat::OpenAIChat,
            &req,
            &mut body,
        )
        .unwrap();
        assert_eq!(body["thinking"]["type"], "disabled");
        assert!(body.get("reasoning_effort").is_none());
        let req = request(SummaryReasoningEffort::Medium, &style);
        apply_summary_reasoning(
            "https://api.deepseek.com",
            "deepseek-v4-pro",
            ProviderFormat::OpenAIChat,
            &req,
            &mut body,
        )
        .unwrap();
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["max_tokens"], 4096);
    }

    #[test]
    fn unsupported_modes_are_terminal_contract_errors() {
        let openai = ThinkingStyle::Openai;
        let none = ThinkingStyle::None;
        let qwen = ThinkingStyle::Qwen;
        let zai = ThinkingStyle::Zai;
        let anthropic = ThinkingStyle::Anthropic;
        for (base, model, format, effort, style) in [
            (
                "https://api.openai.com",
                "gpt-6-astra",
                ProviderFormat::OpenAIResponses,
                SummaryReasoningEffort::None,
                &openai,
            ),
            (
                "https://api.openai.com",
                "gpt-6.1-sol",
                ProviderFormat::OpenAIChat,
                SummaryReasoningEffort::Minimal,
                &openai,
            ),
            (
                CODEX_API_URL,
                "gpt-6-sol",
                ProviderFormat::Codex,
                SummaryReasoningEffort::None,
                &openai,
            ),
            (
                "https://synthetic.example",
                "test",
                ProviderFormat::OpenAIChat,
                SummaryReasoningEffort::Low,
                &none,
            ),
            (
                "https://synthetic.example",
                "test",
                ProviderFormat::OpenAIChat,
                SummaryReasoningEffort::Low,
                &qwen,
            ),
            (
                "https://api.z.ai/api/paas/v4",
                "glm-5.3",
                ProviderFormat::OpenAIChat,
                SummaryReasoningEffort::None,
                &zai,
            ),
            (
                "https://api.anthropic.com",
                "claude-fable-5-1",
                ProviderFormat::Anthropic,
                SummaryReasoningEffort::None,
                &anthropic,
            ),
        ] {
            let req = request(effort, style);
            let mut body = json!({"max_tokens":4096});
            let original = body.clone();
            let error = apply_summary_reasoning(base, model, format, &req, &mut body).unwrap_err();
            assert!(error
                .downcast_ref::<ha_core::failover::ProviderRequestContractError>()
                .is_some());
            assert_eq!(
                ha_core::failover::classify_error_with_evidence(&error).0,
                ha_core::failover::FailoverReason::RequestContract
            );
            assert_eq!(body, original);
        }
    }

    #[test]
    fn native_styles_send_off_switches_and_manual_budgets_must_fit() {
        for (format, style, field, value) in [
            (
                ProviderFormat::OpenAIChat,
                ThinkingStyle::Qwen,
                "enable_thinking",
                json!(false),
            ),
            (
                ProviderFormat::OpenAIChat,
                ThinkingStyle::Zai,
                "thinking",
                json!({"type":"disabled"}),
            ),
            (
                ProviderFormat::Anthropic,
                ThinkingStyle::Anthropic,
                "thinking",
                json!({"type":"disabled"}),
            ),
        ] {
            let req = request(SummaryReasoningEffort::None, &style);
            let mut body = json!({"max_tokens":4096});
            apply_summary_reasoning("https://synthetic.example", "test", format, &req, &mut body)
                .unwrap();
            assert_eq!(body[field], value);
        }
        let style = ThinkingStyle::Anthropic;
        let req = request(SummaryReasoningEffort::Low, &style);
        let mut body = json!({"max_tokens":4096});
        apply_summary_reasoning(
            "https://synthetic.example",
            "test",
            ProviderFormat::Anthropic,
            &req,
            &mut body,
        )
        .unwrap();
        assert_eq!(body["thinking"]["budget_tokens"], 1024);
        let req = request(SummaryReasoningEffort::High, &style);
        let before = body.clone();
        assert!(apply_summary_reasoning(
            "https://synthetic.example",
            "test",
            ProviderFormat::Anthropic,
            &req,
            &mut body
        )
        .is_err());
        assert_eq!(body, before);
    }

    #[test]
    fn native_adaptive_anthropic_and_codex_positive_efforts_are_preserved() {
        let style = ThinkingStyle::Anthropic;
        let req = request(SummaryReasoningEffort::High, &style);
        let mut body = json!({"max_tokens":4096});
        apply_summary_reasoning(
            "https://api.anthropic.com",
            "claude-fable-5-1",
            ProviderFormat::Anthropic,
            &req,
            &mut body,
        )
        .unwrap();
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["max_tokens"], 4096);
        let req = request(SummaryReasoningEffort::Max, &style);
        let mut body = json!({});
        apply_summary_reasoning(
            CODEX_API_URL,
            "gpt-6-astra",
            ProviderFormat::Codex,
            &req,
            &mut body,
        )
        .unwrap();
        assert_eq!(body["reasoning"]["effort"], "max");
    }

    #[tokio::test]
    async fn gateway_rejection_is_terminal_only_for_an_explicit_summary_override() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let body_start;
            loop {
                let mut chunk = [0; 2048];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0 && bytes.len() + n <= 65536);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..index]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= index + 4 + length {
                        body_start = index + 4;
                        break;
                    }
                }
            }
            let request: Value = serde_json::from_slice(&bytes[body_start..]).unwrap();
            let error = r#"{"error":{"message":"Unsupported value: synthetic-private-echo","param":"reasoning_effort","code":"unsupported_value"}}"#;
            socket.write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",error.len(),error).as_bytes()).await.unwrap();
            request
        });
        let provider = LlmProvider::OpenAIChat {
            api_key: "synthetic-local-key".into(),
            base_url: format!("http://{address}"),
            model: "synthetic-model".into(),
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let style = ThinkingStyle::Openai;
        let req = request(SummaryReasoningEffort::None, &style);
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            Runtime.one_shot(&provider, &client, req),
        )
        .await
        .unwrap()
        .err()
        .unwrap();
        assert_eq!(
            ha_core::failover::classify_error_with_evidence(&error).0,
            ha_core::failover::FailoverReason::RequestContract
        );
        assert!(!error.to_string().contains("synthetic-private-echo"));
        let captured = server.await.unwrap();
        assert_eq!(captured["reasoning_effort"], "none");
        assert_eq!(captured["max_tokens"], 4096);
        let plain = ha_core::failover::ProviderApiError::from_http_response(
            "synthetic",
            400,
            r#"{"error":{"param":"reasoning_effort","message":"Unsupported value"}}"#,
        );
        let error = classify_summary_error(false, plain.into());
        assert!(error
            .downcast_ref::<ha_core::failover::ProviderApiError>()
            .is_some());
        let retryable = ha_core::failover::ProviderApiError::from_http_response(
            "synthetic",
            429,
            r#"{"error":{"param":"reasoning_effort","message":"rate limited"}}"#,
        );
        let error = classify_summary_error(true, retryable.into());
        assert!(error
            .downcast_ref::<ha_core::failover::ProviderApiError>()
            .is_some());
    }
}
