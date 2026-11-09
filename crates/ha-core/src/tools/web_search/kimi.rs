use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::Value;
use std::time::Instant;

use super::helpers::{
    build_search_client_for_url, check_search_url, read_json_capped, request_error, status_error,
    JSON_RESPONSE_BYTE_CAP,
};
use super::{SearchParams, SearchResult, WebSearchUsageContext};

const SEARCH_URL: &str = "https://api.moonshot.cn/v1/tools/search";

pub(super) async fn search_kimi(
    api_key: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
    usage_ctx: &WebSearchUsageContext,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        bail!("Kimi (Moonshot) API key not configured");
    }
    validate_filters(params)?;
    check_search_url(SEARCH_URL).await?;
    let client = build_search_client_for_url(SEARCH_URL, timeout_secs.max(1))?;
    // Drop records cancellation too: an unreadable/lost response has unknown
    // billing, not an invented free call. No retries inside this adapter.
    let mut usage = UsageRecorder {
        context: usage_ctx,
        started: Instant::now(),
        outcome: UsageOutcome::default(),
    };
    let result = request_search(
        &client,
        SEARCH_URL,
        api_key,
        &search_body(query, count, timeout_secs),
        count,
        &mut usage.outcome,
    )
    .await;
    usage.outcome.success = result.is_ok();
    usage.outcome.error = result.as_ref().err().map(ToString::to_string);
    result
}

fn validate_filters(params: &SearchParams) -> Result<()> {
    if params.country.is_some() || params.language.is_some() || params.freshness.is_some() {
        bail!("Kimi Basic does not support country, language, or freshness filters; choose a provider that supports the requested filters");
    }
    Ok(())
}

fn search_body(query: &str, count: usize, timeout_secs: u64) -> Value {
    serde_json::json!({
        "text_query": query, "limit": count.clamp(1, 20),
        "timeout_seconds": timeout_secs.clamp(1, 60), "include_content": false,
    })
}

#[derive(Default)]
struct UsageOutcome {
    http_status: Option<u16>,
    returned_count: Option<usize>,
    success: bool,
    error: Option<String>,
}

struct UsageRecorder<'a> {
    context: &'a WebSearchUsageContext,
    started: Instant,
    outcome: UsageOutcome,
}

impl Drop for UsageRecorder<'_> {
    fn drop(&mut self) {
        crate::model_usage::record_model_usage_best_effort(usage_event(
            self.context,
            self.started.elapsed().as_millis() as u64,
            &self.outcome,
        ));
    }
}

fn usage_event(
    context: &WebSearchUsageContext,
    duration_ms: u64,
    outcome: &UsageOutcome,
) -> crate::model_usage::ModelUsageEvent {
    let mut event = crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_WEB_SEARCH);
    event.operation = Some("web_search.kimi.basic".into());
    event.source = Some("web_search".into());
    event.provider_id = Some("kimi".into());
    event.provider_name = Some("Kimi".into());
    event.session_id = context.session_id.clone();
    event.agent_id = context.agent_id.clone();
    event.duration_ms = Some(duration_ms);
    event.success = outcome.success;
    event.error = outcome
        .error
        .clone()
        .or_else(|| (!outcome.success).then(|| "Kimi request did not complete".to_string()));
    let billable_call = match outcome.http_status {
        Some(200) => outcome.returned_count.map(|count| count > 0),
        Some(_) => Some(false),
        None => None,
    };
    // Basic has no chat model or token usage. This is published billing
    // eligibility, not a measured price or an account billing receipt.
    event.metadata = Some(serde_json::json!({
        "endpoint": "tools/search", "tier": "basic", "http_status": outcome.http_status,
        "returned_result_count": outcome.returned_count, "billable_call": billable_call,
        "billing_basis": "http_200_nonempty_search_results",
    }));
    event
}

async fn request_search(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
    count: usize,
    outcome: &mut UsageOutcome,
) -> Result<Vec<SearchResult>> {
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .map_err(|error| request_error("Kimi", error))?;
    let status = response.status();
    outcome.http_status = Some(status.as_u16());
    if status != reqwest::StatusCode::OK {
        return Err(status_error("Kimi", status));
    }
    let data = read_json_capped(response, JSON_RESPONSE_BYTE_CAP, "Kimi")
        .await
        .map_err(|_| anyhow::anyhow!("Kimi response is not readable bounded JSON"))?;
    outcome.returned_count = data
        .get("search_results")
        .and_then(Value::as_array)
        .map(Vec::len);
    parse_results(data, count)
}

#[derive(Deserialize)]
struct BasicResponse {
    search_results: Vec<BasicResult>,
}
#[derive(Deserialize)]
struct BasicResult {
    title: String,
    url: String,
    snippet: String,
}

fn parse_results(data: Value, count: usize) -> Result<Vec<SearchResult>> {
    let data: BasicResponse = serde_json::from_value(data)
        .map_err(|_| anyhow::anyhow!("Kimi invalid search_results response"))?;
    Ok(data
        .search_results
        .into_iter()
        .filter(|item| {
            reqwest::Url::parse(&item.url).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            })
        })
        .take(count.clamp(1, 20))
        .map(|item| SearchResult {
            title: item.title,
            url: item.url,
            snippet: item.snippet,
            source: "Kimi".into(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn fixture(
        status: u16,
        body: String,
        headers: String,
    ) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let length = socket.read(&mut chunk).await.unwrap();
                assert!(length > 0);
                request.extend_from_slice(&chunk[..length]);
                assert!(request.len() < 8192);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let content_length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + content_length {
                        break;
                    }
                }
            }
            let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes()).await;
            request
        });
        (format!("http://{address}/v1/tools/search"), server)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap()
    }

    #[test]
    fn structured_results_never_invent_citations_or_parse_model_text() {
        let results = parse_results(
            serde_json::json!({"search_results": [
                {"title":"", "url":"", "snippet":"PRIVATE_CANARY"},
                {"title":"bad", "url":"javascript:alert(1)", "snippet":""},
                {"title":"First", "url":"https://example.com/a", "snippet":"摘要"},
                {"title":"Second", "url":"https://example.com/b", "snippet":""}
            ]}),
            1,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "First");
        assert_eq!(results[0].snippet, "摘要");
        assert!(parse_results(serde_json::json!({"search_results": []}), 10)
            .unwrap()
            .is_empty());
        for data in [
            serde_json::json!({"choices":[{"message":{"content":"[]"}}]}),
            serde_json::json!({"search_results": "PRIVATE_CANARY"}),
            serde_json::json!({"search_results":[{"title":42,"url":"https://example.com","snippet":"PRIVATE_CANARY"}]}),
        ] {
            assert!(!parse_results(data, 5)
                .err()
                .unwrap()
                .to_string()
                .contains("PRIVATE_CANARY"));
        }
    }

    #[test]
    fn unsupported_filters_are_rejected_before_dispatch() {
        for params in [
            SearchParams {
                country: Some("CN".into()),
                ..Default::default()
            },
            SearchParams {
                language: Some("zh".into()),
                ..Default::default()
            },
            SearchParams {
                freshness: Some("day".into()),
                ..Default::default()
            },
        ] {
            assert!(validate_filters(&params)
                .err()
                .unwrap()
                .to_string()
                .contains("does not support"));
        }
        assert!(validate_filters(&SearchParams::default()).is_ok());
    }

    #[tokio::test]
    async fn basic_request_is_single_bounded_post_without_a_chat_model() {
        let (url, server) = fixture(200, r#"{"search_results":[{"title":"Title","url":"https://example.com","snippet":"摘要"}]}"#.into(), String::new()).await;
        let mut outcome = UsageOutcome::default();
        let results = request_search(
            &client(),
            &url,
            "SYNTHETIC_KEY",
            &search_body("查询", 100, 120),
            10,
            &mut outcome,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(outcome.returned_count, Some(1));
        let request = server.await.unwrap();
        let end = request
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap();
        assert!(String::from_utf8_lossy(&request).starts_with("POST /v1/tools/search HTTP/1.1"));
        assert_eq!(
            serde_json::from_slice::<Value>(&request[end + 4..]).unwrap(),
            serde_json::json!({"text_query":"查询","limit":20,"timeout_seconds":60,"include_content":false})
        );
        assert_eq!(search_body("q", 0, 0)["timeout_seconds"], 1);
    }

    #[tokio::test]
    async fn errors_do_not_read_private_bodies_or_follow_redirects() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        for status in [201, 302, 307, 308, 400, 401, 403, 408, 429, 500, 502, 504] {
            let (url, server) = fixture(
                status,
                "PRIVATE_CANARY".into(),
                format!(
                    "Location: http://{}/?secret=PRIVATE_CANARY\r\n",
                    target.local_addr().unwrap()
                ),
            )
            .await;
            let mut outcome = UsageOutcome::default();
            let error = request_search(
                &client(),
                &url,
                "SYNTHETIC_KEY",
                &search_body("PRIVATE_QUERY", 5, 2),
                5,
                &mut outcome,
            )
            .await
            .err()
            .unwrap()
            .to_string();
            assert!(error.contains(&status.to_string()));
            assert!(!error.contains("PRIVATE_"));
            assert!(!error.contains("SYNTHETIC_KEY"));
            assert_eq!(outcome.http_status, Some(status));
            server.await.unwrap();
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), target.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unreadable_responses_fail_with_unknown_billing() {
        for body in [
            "PRIVATE_CANARY".to_string(),
            " ".repeat(JSON_RESPONSE_BYTE_CAP + 1),
        ] {
            let (url, server) = fixture(200, body, String::new()).await;
            let mut outcome = UsageOutcome::default();
            let error = request_search(
                &client(),
                &url,
                "SYNTHETIC_KEY",
                &search_body("PRIVATE_QUERY", 5, 2),
                5,
                &mut outcome,
            )
            .await
            .err()
            .unwrap()
            .to_string();
            assert!(!error.contains("PRIVATE_"));
            assert_eq!(outcome.http_status, Some(200));
            assert_eq!(outcome.returned_count, None);
            server.await.unwrap();
        }
    }

    #[test]
    fn usage_keeps_tokens_absent_and_billing_uncertainty_explicit() {
        for (status, returned_count, expected) in [
            (Some(200), Some(1), serde_json::json!(true)),
            (Some(200), Some(0), serde_json::json!(false)),
            (Some(200), None, Value::Null),
            (Some(429), None, serde_json::json!(false)),
            (None, None, Value::Null),
        ] {
            let event = usage_event(
                &WebSearchUsageContext {
                    session_id: Some("synthetic-session".into()),
                    agent_id: Some("synthetic-agent".into()),
                },
                1,
                &UsageOutcome {
                    http_status: status,
                    returned_count,
                    ..Default::default()
                },
            );
            assert_eq!(event.metadata.unwrap()["billable_call"], expected);
            assert!(event.model_id.is_none());
            assert!(event.input_tokens.is_none());
            assert!(event.output_tokens.is_none());
            assert_eq!(event.session_id.as_deref(), Some("synthetic-session"));
            assert_eq!(event.agent_id.as_deref(), Some("synthetic-agent"));
        }
    }

    #[tokio::test]
    async fn upstream_nonempty_results_remain_billable_when_unusable() {
        for (body, billable, failed) in [
            (serde_json::json!({"search_results": []}), false, false),
            (
                serde_json::json!({"search_results": [{"title":"", "url":"", "snippet":""}]}),
                true,
                false,
            ),
            (
                serde_json::json!({"search_results": [{"title":42, "url":"", "snippet":"PRIVATE_CANARY"}]}),
                true,
                true,
            ),
        ] {
            let (url, server) = fixture(200, body.to_string(), String::new()).await;
            let mut outcome = UsageOutcome::default();
            let result = request_search(
                &client(),
                &url,
                "SYNTHETIC_KEY",
                &search_body("q", 5, 2),
                5,
                &mut outcome,
            )
            .await;
            assert_eq!(result.is_err(), failed);
            assert_eq!(
                usage_event(&WebSearchUsageContext::default(), 1, &outcome)
                    .metadata
                    .unwrap()["billable_call"],
                billable
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn canceled_request_keeps_billing_unknown_and_closes_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/tools/search", listener.local_addr().unwrap());
        let (started, received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 8192];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            started.send(()).unwrap();
            while socket.read(&mut bytes).await.unwrap() > 0 {}
        });
        let client = client();
        let body = search_body("q", 5, 2);
        let mut outcome = UsageOutcome::default();
        let mut request = Box::pin(request_search(
            &client,
            &url,
            "SYNTHETIC_KEY",
            &body,
            5,
            &mut outcome,
        ));
        tokio::select! {
            _ = &mut request => panic!("fixture must wait for cancellation"),
            _ = received => {}
        }
        drop(request);
        let event = usage_event(&WebSearchUsageContext::default(), 1, &outcome);
        assert!(!event.success);
        assert_eq!(event.metadata.unwrap()["billable_call"], Value::Null);
        tokio::time::timeout(std::time::Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }
}
