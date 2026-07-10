use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::Value;

use super::{ExternalMemoryAdapterSyncFailure, ExternalMemoryAdapterSyncOutcome};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("hope-agent/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build external memory provider HTTP client")
}

pub(super) async fn validated_endpoint(raw: &str) -> Result<String> {
    let ssrf = crate::config::cached_config().ssrf.clone();
    let url =
        crate::security::ssrf::check_url(raw, ssrf.default_policy, &ssrf.trusted_hosts).await?;
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
    outcome.external_io_performed = true;
    let response = request
        .send()
        .await
        .map_err(|error| failure(outcome.clone(), error.into()))?;
    if response.status().is_redirection() {
        return Err(failure(
            outcome.clone(),
            anyhow!("external memory provider redirect refused"),
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size as usize > MAX_RESPONSE_BYTES)
    {
        return Err(failure(
            outcome.clone(),
            anyhow!("external memory provider response is too large"),
        ));
    }
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| failure(outcome.clone(), error.into()))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(failure(
            outcome.clone(),
            anyhow!("external memory provider response is too large"),
        ));
    }
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

fn bounded_response_detail(bytes: &[u8]) -> String {
    static SECRET_ASSIGNMENT_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?i)\b(api[_-]?key|access[_-]?token|refresh[_-]?token|token|secret|password)\s*[:=]\s*[^\s,&;\"']+"#,
        )
        .expect("valid external provider secret regex")
    });
    let redacted = crate::logging::redact_sensitive(&String::from_utf8_lossy(bytes));
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
}
