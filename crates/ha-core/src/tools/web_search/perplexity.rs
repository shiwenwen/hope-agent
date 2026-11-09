use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Instant;

use super::helpers::{
    build_search_client_for_url, check_search_url, read_json_capped, request_error, status_error,
    JSON_RESPONSE_BYTE_CAP,
};
use super::{llm_web_search_usage_event, SearchParams, SearchResult, WebSearchUsageContext};

const MODEL_ID: &str = "sonar";
const SEARCH_URL: &str = "https://api.perplexity.ai/chat/completions";

pub(super) async fn search_perplexity(
    api_key: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
    usage_ctx: &WebSearchUsageContext,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        bail!("Perplexity API key not configured");
    }
    check_search_url(SEARCH_URL).await?;
    let client = build_search_client_for_url(SEARCH_URL, timeout_secs)?;
    let mut body = serde_json::json!({
        "model": MODEL_ID,
        "messages": [{"role": "user", "content": query}],
        "max_tokens": 1024,
        "return_citations": true
    });
    if let Some(ref freshness) = params.freshness {
        body["search_recency_filter"] = Value::String(freshness.clone());
    }
    let mut usage = UsageRecorder {
        context: usage_ctx,
        started: Instant::now(),
        response: None,
        success: false,
        error: None,
    };
    let result = request_search(
        &client,
        SEARCH_URL,
        api_key,
        &body,
        count,
        &mut usage.response,
    )
    .await;
    usage.success = result.is_ok();
    usage.error = result.as_ref().err().map(ToString::to_string);
    result
}

// The guard spans send, bounded read and parsing. A dropped future records one
// unfinished attempt with missing usage, never an invented zero-cost success.
struct UsageRecorder<'a> {
    context: &'a WebSearchUsageContext,
    started: Instant,
    response: Option<Value>,
    success: bool,
    error: Option<String>,
}

impl UsageRecorder<'_> {
    fn event(&self) -> crate::model_usage::ModelUsageEvent {
        llm_web_search_usage_event(
            self.context,
            "web_search.perplexity",
            "perplexity",
            "Perplexity",
            MODEL_ID,
            self.started.elapsed().as_millis() as u64,
            self.success,
            self.error
                .clone()
                .or_else(|| (!self.success).then(|| "Perplexity request did not complete".into())),
            self.response.as_ref(),
        )
    }
}

impl Drop for UsageRecorder<'_> {
    fn drop(&mut self) {
        crate::model_usage::record_model_usage_best_effort(self.event());
    }
}

async fn request_search(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
    count: usize,
    response: &mut Option<Value>,
) -> Result<Vec<SearchResult>> {
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .map_err(|e| request_error("Perplexity", e))?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(status_error("Perplexity", resp.status()));
    }
    let data = read_json_capped(resp, JSON_RESPONSE_BYTE_CAP, "Perplexity")
        .await
        .map_err(|_| anyhow::anyhow!("Perplexity response is not readable bounded JSON"))?;
    let result = parse_results(&data, count);
    *response = Some(data);
    result
}

fn source_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn parse_results(data: &Value, count: usize) -> Result<Vec<SearchResult>> {
    if data.get("error").is_some() {
        bail!("Perplexity response contains an error");
    }
    let citations = data.get("citations").and_then(Value::as_array);
    let content = data
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str);
    if citations.is_none() && content.is_none() {
        bail!("Perplexity response has no recognized search result fields");
    }
    let count = count.min(10);
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let urls = citations.into_iter().flatten().filter_map(Value::as_str);
    for raw_url in urls {
        let Some(url) = source_url(raw_url) else {
            continue;
        };
        if results.len() == count {
            break;
        }
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = reqwest::Url::parse(&url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        results.push(SearchResult {
            title,
            url,
            snippet: String::new(),
            source: "Perplexity".into(),
        });
    }
    // Preserve the legacy explicitly unsourced summary fallback. Never turn
    // answer text or an Agent API output item into a fabricated citation.
    if count > 0 && results.is_empty() {
        if let Some(content) = content.filter(|s| !s.is_empty()) {
            results.push(SearchResult {
                title: "Perplexity Summary".into(),
                url: String::new(),
                snippet: crate::util::truncate_utf8(content, 500).to_owned(),
                source: "Perplexity".into(),
            });
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_citation_order_and_unsourced_summary_remain_compatible() {
        let results = parse_results(
            &json!({"citations":["https://b.example/","https://a.example/"],
            "choices":[{"message":{"content":"answer"}}]}),
            2,
        )
        .unwrap();
        assert_eq!(results[0].url, "https://b.example/");
        assert_eq!(results[0].title, "b.example");
        assert!(results[0].snippet.is_empty());
        assert_eq!(results[1].url, "https://a.example/");
    }

    #[test]
    fn invalid_or_duplicate_sources_do_not_consume_the_limit() {
        let results = parse_results(
            &json!({"citations":[
                "javascript:alert(1)", "https://user:secret@a.example/", "https://a.example/",
                "https://a.example/", "https://b.example/", "https://c.example/"
            ]}),
            2,
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://a.example/");
        assert_eq!(results[1].url, "https://b.example/");
    }

    #[test]
    fn malformed_protocol_is_not_an_empty_search() {
        for data in [
            json!({}),
            json!({"error":{"message":"PRIVATE_CANARY"}}),
            json!({"citations":42}),
        ] {
            let error = parse_results(&data, 2)
                .err()
                .expect("protocol error")
                .to_string();
            assert!(!error.contains("PRIVATE_CANARY"));
        }
        assert!(parse_results(&json!({"citations":[]}), 2)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_summary_is_bounded_and_explicitly_unsourced() {
        let results = parse_results(
            &json!({"choices":[{"message":{"content":"x".repeat(800)}}]}),
            1,
        )
        .unwrap();
        assert_eq!(results[0].snippet.len(), 500);
        assert!(results[0].url.is_empty());
    }
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn fixture(
        status: u16,
        body: String,
        headers: String,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0u8; 1024];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(request.len() < 8192);
            }
            let _=socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len()).as_bytes()).await;
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/search"), task)
    }

    fn recorder(ctx: &WebSearchUsageContext) -> UsageRecorder<'_> {
        UsageRecorder {
            context: ctx,
            started: Instant::now(),
            response: None,
            success: false,
            error: None,
        }
    }

    #[tokio::test]
    async fn redirects_and_http_errors_never_reach_the_target_or_reflect_private_data() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        for status in [201, 302, 307, 308, 400, 401, 403, 429, 500] {
            let (url, task) = fixture(
                status,
                "PRIVATE_CANARY".into(),
                format!(
                    "Location: http://{}/PRIVATE_CANARY\r\n",
                    target.local_addr().unwrap()
                ),
            )
            .await;
            let client = build_search_client_for_url(&url, 2).unwrap();
            let mut response = None;
            let error = request_search(
                &client,
                &url,
                "SYNTHETIC_KEY",
                &json!({"query":"PRIVATE_QUERY"}),
                2,
                &mut response,
            )
            .await
            .err()
            .unwrap()
            .to_string();
            assert!(error.contains(&status.to_string()));
            assert!(!error.contains("PRIVATE_") && !error.contains("SYNTHETIC_KEY"));
            assert!(response.is_none());
            task.await.unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), target.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn malformed_truncated_and_oversized_json_are_private_failures() {
        for (body, headers) in [
            ("{PRIVATE_CANARY".to_owned(), String::new()),
            (
                "PRIVATE_CANARY".to_owned(),
                "Content-Type: application/json\r\n".into(),
            ),
            ("x".repeat(JSON_RESPONSE_BYTE_CAP + 1), String::new()),
        ] {
            let (url, task) = fixture(200, body, headers).await;
            let mut response = None;
            let error = request_search(
                &build_search_client_for_url(&url, 2).unwrap(),
                &url,
                "SYNTHETIC_KEY",
                &json!({}),
                2,
                &mut response,
            )
            .await
            .err()
            .unwrap()
            .to_string();
            assert_eq!(error, "Perplexity response is not readable bounded JSON");
            assert!(response.is_none());
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn protocol_failure_keeps_actual_usage_and_session_attribution() {
        let ctx = WebSearchUsageContext {
            session_id: Some("synthetic-session".into()),
            agent_id: Some("synthetic-agent".into()),
        };
        let mut usage = recorder(&ctx);
        let (url,task)=fixture(200,json!({"error":{"message":"PRIVATE_CANARY"},"usage":{"prompt_tokens":7,"completion_tokens":0}}).to_string(),String::new()).await;
        let result = request_search(
            &build_search_client_for_url(&url, 2).unwrap(),
            &url,
            "SYNTHETIC_KEY",
            &json!({}),
            2,
            &mut usage.response,
        )
        .await;
        usage.error = result.err().map(|e| e.to_string());
        let event = usage.event();
        assert!(!event.success);
        assert_eq!(event.input_tokens, Some(7));
        assert_eq!(event.output_tokens, Some(0));
        assert_eq!(event.session_id, ctx.session_id);
        assert_eq!(event.agent_id, ctx.agent_id);
        assert!(!event.error.unwrap().contains("PRIVATE_CANARY"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_closes_the_socket_and_keeps_usage_unknown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/search", listener.local_addr().unwrap());
        let client = build_search_client_for_url(&url, 2).unwrap();
        let ctx = WebSearchUsageContext::default();
        let mut usage = recorder(&ctx);
        let body = json!({});
        let mut request = Box::pin(request_search(
            &client,
            &url,
            "SYNTHETIC_KEY",
            &body,
            2,
            &mut usage.response,
        ));
        let mut socket = tokio::select! {
            accepted=listener.accept()=>accepted.unwrap().0,
            _=&mut request=>panic!("server has not returned a response")
        };
        let mut bytes = [0u8; 1024];
        tokio::select! {
            n=socket.read(&mut bytes)=>assert!(n.unwrap()>0),
            _=&mut request=>panic!("server has not returned a response")
        }
        drop(request);
        let event = usage.event();
        assert!(!event.success);
        assert!(event.input_tokens.is_none() && event.output_tokens.is_none());
        assert_eq!(
            event.error.as_deref(),
            Some("Perplexity request did not complete")
        );
        let mut remaining = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut remaining))
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn missing_usage_and_real_zero_are_distinct() {
        let ctx = WebSearchUsageContext::default();
        let mut usage = recorder(&ctx);
        let event = usage.event();
        assert!(!event.success);
        assert!(event.input_tokens.is_none());
        assert!(event.output_tokens.is_none());
        usage.response = Some(
            json!({"usage":{"input_tokens":0,"output_tokens":9,"input_tokens_details":{"cached_tokens":0}}}),
        );
        usage.success = true;
        let event = usage.event();
        assert!(event.success);
        assert_eq!(event.input_tokens, Some(0));
        assert_eq!(event.output_tokens, Some(9));
        assert_eq!(event.cache_read_input_tokens, Some(0));
    }
}
