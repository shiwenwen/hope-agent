use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::Value;

use super::{ExternalMemoryAdapterSyncFailure, ExternalMemoryAdapterSyncOutcome};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROBE_RESPONSE_BYTES: usize = 64 * 1024;

pub(super) struct ProbeResponse {
    pub body: Vec<u8>,
    pub version_headers: Vec<String>,
}

pub(super) fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("hope-agent/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build external memory provider HTTP client")
}

pub(super) async fn validated_endpoint(raw: &str) -> Result<String> {
    super::ensure_provider_sync_request_budget()?;
    let ssrf = ha_core::config::cached_config().ssrf.clone();
    let url =
        ha_core::security::ssrf::check_url(raw, ssrf.default_policy, &ssrf.trusted_hosts).await?;
    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub(super) fn endpoint_with_path(endpoint: &str, segments: &[&str]) -> Result<String> {
    let mut url = url::Url::parse(endpoint).context("parse external memory provider endpoint")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("external memory provider endpoint cannot be a base URL"))?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub(super) async fn send_json(
    request: RequestBuilder,
    outcome: &mut ExternalMemoryAdapterSyncOutcome,
) -> std::result::Result<Value, ExternalMemoryAdapterSyncFailure> {
    super::ensure_provider_sync_request_budget()
        .map_err(|error| failure(outcome.clone(), error))?;
    outcome.external_io_performed = true;
    let mut response = request
        .send()
        .await
        .map_err(|error| failure(outcome.clone(), error.into()))?;
    if response.status().is_redirection() {
        return Err(failure(
            outcome.clone(),
            anyhow!("external memory provider redirect refused"),
        ));
    }
    let status = response.status();
    let bytes = read_bounded_body(
        &mut response,
        MAX_RESPONSE_BYTES,
        "external memory provider response is too large",
    )
    .await
    .map_err(|error| failure(outcome.clone(), error))?;
    if !status.is_success() {
        let detail = bounded_response_detail(&bytes);
        return Err(failure(
            outcome.clone(),
            anyhow!("external memory provider returned HTTP {status}: {detail}"),
        ));
    }
    if status == StatusCode::NO_CONTENT || bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .context("parse external memory provider response")
        .map_err(|error| failure(outcome.clone(), error))
}

/// Send the small, owner-triggered compatibility probe without exposing a
/// response body to callers. Only bounded bytes and selected version headers
/// are returned for local parsing; redirects and oversized responses remain
/// fail-closed like regular provider traffic.
pub(super) async fn send_probe(
    request: RequestBuilder,
    version_header_names: &[&str],
) -> Result<ProbeResponse> {
    super::ensure_provider_sync_request_budget()?;
    let mut response = request.send().await?;
    if response.status().is_redirection() {
        anyhow::bail!("external memory provider redirect refused");
    }
    let status = response.status();
    let version_headers = version_header_names
        .iter()
        .filter_map(|name| response.headers().get(*name))
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let bytes = read_bounded_body(
        &mut response,
        MAX_PROBE_RESPONSE_BYTES,
        "external memory provider probe response is too large",
    )
    .await?;
    if !status.is_success() {
        anyhow::bail!(
            "external memory provider returned HTTP {status}: {}",
            bounded_response_detail(&bytes)
        );
    }
    Ok(ProbeResponse {
        body: bytes,
        version_headers,
    })
}

async fn read_bounded_body(
    response: &mut reqwest::Response,
    max_bytes: usize,
    too_large_message: &'static str,
) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes as u64)
    {
        anyhow::bail!(too_large_message);
    }
    let capacity = response
        .content_length()
        .unwrap_or_default()
        .min(max_bytes as u64) as usize;
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > max_bytes.saturating_sub(body.len()) {
            anyhow::bail!(too_large_message);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn bounded_response_detail(bytes: &[u8]) -> String {
    static SECRET_ASSIGNMENT_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?i)\b(api[_-]?key|access[_-]?token|refresh[_-]?token|token|secret|password)\s*[:=]\s*[^\s,&;\"']+"#,
        )
        .expect("valid external provider secret regex")
    });
    let redacted = ha_core::logging::redact_sensitive(&String::from_utf8_lossy(bytes));
    SECRET_ASSIGNMENT_RE
        .replace_all(&redacted, "${1}=[redacted]")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(512)
        .collect()
}

fn failure(
    outcome: ExternalMemoryAdapterSyncOutcome,
    error: anyhow::Error,
) -> ExternalMemoryAdapterSyncFailure {
    ExternalMemoryAdapterSyncFailure { outcome, error }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn local_response_url(headers: &str, body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let headers = headers.to_string();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(format!("HTTP/1.1 200 OK\r\n{headers}\r\n").as_bytes())
                .await
                .unwrap();
            stream.write_all(&body).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        format!("http://{address}")
    }

    #[test]
    fn response_detail_is_bounded_redacted_and_single_line() {
        let detail = bounded_response_detail(b"bad\nrequest\ttoken=secret");
        assert_eq!(detail, "bad request token=[redacted]");
    }

    #[test]
    fn endpoint_path_segments_are_percent_encoded() {
        assert_eq!(
            endpoint_with_path("https://example.com/base", &["groups", "alice/bob"]).unwrap(),
            "https://example.com/base/groups/alice%2Fbob"
        );
    }

    #[tokio::test]
    async fn provider_http_never_follows_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/target", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/target"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(0)
            .mount(&server)
            .await;

        let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
        let error = send_json(
            client().unwrap().get(format!("{}/start", server.uri())),
            &mut outcome,
        )
        .await
        .unwrap_err();

        assert!(outcome.external_io_performed);
        assert!(error.error.to_string().contains("redirect refused"));
    }

    #[tokio::test]
    async fn provider_http_rejects_oversized_responses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/large"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_RESPONSE_BYTES + 1]),
            )
            .mount(&server)
            .await;

        let mut outcome = ExternalMemoryAdapterSyncOutcome::default();
        let error = send_json(
            client().unwrap().get(format!("{}/large", server.uri())),
            &mut outcome,
        )
        .await
        .unwrap_err();

        assert!(outcome.external_io_performed);
        assert!(error.error.to_string().contains("response is too large"));
    }

    #[tokio::test]
    async fn provider_probe_uses_only_explicit_product_version_headers() {
        let url = local_response_url(
            "Server: nginx/1.24.0\r\nX-Graphiti-Version: 0.29.3\r\nContent-Length: 2\r\nConnection: close\r\n",
            b"{}".to_vec(),
        )
        .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = send_probe(client.get(url), &["x-graphiti-version"])
            .await
            .unwrap();
        assert_eq!(response.version_headers, vec!["0.29.3"]);
    }

    #[tokio::test]
    async fn provider_probe_stops_streaming_an_oversized_chunked_body() {
        let chunk = vec![b'x'; MAX_PROBE_RESPONSE_BYTES + 1];
        let mut body = format!("{:X}\r\n", chunk.len()).into_bytes();
        body.extend_from_slice(&chunk);
        body.extend_from_slice(b"\r\n0\r\n\r\n");
        let url =
            local_response_url("Transfer-Encoding: chunked\r\nConnection: close\r\n", body).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = match send_probe(client.get(url), &[]).await {
            Err(error) => error,
            Ok(_) => panic!("oversized chunked probe should fail"),
        };
        assert!(error.to_string().contains("probe response is too large"));
    }

    #[tokio::test]
    async fn expired_sync_budget_stops_before_remote_io_and_preserves_outcome() {
        let server = MockServer::start().await;
        let mut outcome = ExternalMemoryAdapterSyncOutcome {
            imported_memory_count: 3,
            ..Default::default()
        };
        let request = client()
            .unwrap()
            .get(format!("{}/never-sent", server.uri()));

        let error = super::super::PROVIDER_SYNC_DEADLINE
            .scope(std::time::Instant::now(), send_json(request, &mut outcome))
            .await
            .unwrap_err();

        assert!(!outcome.external_io_performed);
        assert_eq!(error.outcome.imported_memory_count, 3);
        assert!(error.error.to_string().contains("request budget"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
