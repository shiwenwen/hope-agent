//! Redirect-safe HTTP GET primitive.
//!
//! reqwest's redirect callback is synchronous, so it cannot perform the DNS
//! resolution required by [`super::ssrf::check_url`].  Callers that let
//! reqwest follow redirects therefore have a hostname-sized SSRF gap.  This
//! module keeps redirects disabled in the client and validates every hop in
//! async code before the next request is sent.

use std::collections::HashSet;
use std::future::Future;

use reqwest::header::{HeaderMap, LOCATION};

use super::ssrf::{check_url, SsrfPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectHop {
    pub url: String,
    pub to_url: String,
    pub status: u16,
}

pub struct CheckedGetResponse {
    pub response: reqwest::Response,
    pub redirects: Vec<RedirectHop>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckedGetErrorKind {
    UrlCheck,
    Request,
    RedirectProtocol,
}

#[derive(Debug)]
pub struct CheckedGetError {
    kind: CheckedGetErrorKind,
    message: String,
}

impl CheckedGetError {
    fn new(kind: CheckedGetErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CheckedGetErrorKind {
        self.kind
    }
}

impl std::fmt::Display for CheckedGetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CheckedGetError {}

fn follows_location(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

async fn checked_target(
    url: &str,
    policy: SsrfPolicy,
    allowlist: &[String],
    redirect: bool,
) -> std::result::Result<url::Url, CheckedGetError> {
    let mut target = check_url(url, policy, allowlist).await.map_err(|_| {
        CheckedGetError::new(
            CheckedGetErrorKind::UrlCheck,
            if redirect {
                "redirect URL is blocked by policy or could not be resolved"
            } else {
                "URL is blocked by policy or could not be resolved"
            },
        )
    })?;
    if !target.username().is_empty() || target.password().is_some() {
        return Err(CheckedGetError::new(
            CheckedGetErrorKind::UrlCheck,
            "URL credentials are not allowed",
        ));
    }
    target.set_fragment(None);
    Ok(target)
}

/// Perform a GET with redirects disabled at the client layer and validated at
/// every hop through the repository's single SSRF decision point.
///
/// `headers` are re-applied to each hop.  This helper deliberately does not
/// accept credentials; a future credential-aware caller must strip them on a
/// cross-origin redirect before calling this primitive.
pub async fn checked_get(
    client: &reqwest::Client,
    url_str: &str,
    policy: SsrfPolicy,
    allowlist: &[String],
    max_redirects: usize,
    headers: Option<&HeaderMap>,
) -> std::result::Result<CheckedGetResponse, CheckedGetError> {
    checked_get_with_admission(
        client,
        url_str,
        policy,
        allowlist,
        max_redirects,
        headers,
        |_| std::future::ready(()),
    )
    .await
    .map(|(response, ())| response)
}

/// Variant of [`checked_get`] that reserves a caller-defined admission guard
/// immediately before each actual network request. Redirect responses drop
/// their guard before the next hop is admitted; the final guard is returned
/// with the response so callers can hold a per-host concurrency slot while
/// streaming the response body.
pub async fn checked_get_with_admission<F, Fut, G>(
    client: &reqwest::Client,
    url_str: &str,
    policy: SsrfPolicy,
    allowlist: &[String],
    max_redirects: usize,
    headers: Option<&HeaderMap>,
    mut before_request: F,
) -> std::result::Result<(CheckedGetResponse, G), CheckedGetError>
where
    F: FnMut(&url::Url) -> Fut,
    Fut: Future<Output = G>,
{
    let mut next = checked_target(url_str, policy, allowlist, false).await?;
    let mut seen = HashSet::new();
    let mut redirects = Vec::new();

    loop {
        let identity = next.as_str().to_string();
        if !seen.insert(identity) {
            return Err(CheckedGetError::new(
                CheckedGetErrorKind::RedirectProtocol,
                "redirect loop detected",
            ));
        }

        let admission = before_request(&next).await;
        let mut request = client.get(next.clone());
        if let Some(headers) = headers {
            request = request.headers(headers.clone());
        }
        let response = request.send().await.map_err(|error| {
            CheckedGetError::new(
                CheckedGetErrorKind::Request,
                format!("request failed: {}", error.without_url()),
            )
        })?;
        let status = response.status();
        if !follows_location(status) {
            return Ok((
                CheckedGetResponse {
                    response,
                    redirects,
                },
                admission,
            ));
        }

        if redirects.len() >= max_redirects {
            return Err(CheckedGetError::new(
                CheckedGetErrorKind::RedirectProtocol,
                "too many redirects",
            ));
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                CheckedGetError::new(
                    CheckedGetErrorKind::RedirectProtocol,
                    "redirect missing a valid Location header",
                )
            })?;
        let redirected = response.url().join(location).map_err(|_| {
            CheckedGetError::new(
                CheckedGetErrorKind::RedirectProtocol,
                "invalid redirect Location",
            )
        })?;
        redirects.push(RedirectHop {
            url: response.url().to_string(),
            to_url: redirected.to_string(),
            status: status.as_u16(),
        });
        next = checked_target(redirected.as_str(), policy, allowlist, true).await?;
        drop(response);
        drop(admission);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_redirect_statuses_follow_location() {
        for status in [301, 302, 303, 307, 308] {
            assert!(follows_location(
                reqwest::StatusCode::from_u16(status).expect("status")
            ));
        }
        for status in [200, 300, 304, 305, 306, 400] {
            assert!(!follows_location(
                reqwest::StatusCode::from_u16(status).expect("status")
            ));
        }
    }

    #[tokio::test]
    async fn redirect_target_is_checked_before_it_is_contacted() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let origin = TcpListener::bind("127.0.0.1:0").await.expect("origin");
        let target = TcpListener::bind("127.0.0.1:0").await.expect("target");
        let origin_addr = origin.local_addr().expect("origin address");
        let target_addr = target.local_addr().expect("target address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = origin.accept().await.expect("origin accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{target_addr}/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write redirect");
        });
        let target_probe = tokio::spawn(async move {
            tokio::time::timeout(std::time::Duration::from_millis(300), target.accept()).await
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client");
        let result = checked_get(
            &client,
            &format!("http://{origin_addr}/start"),
            SsrfPolicy::Strict,
            &[origin_addr.to_string()],
            5,
            None,
        )
        .await;

        assert!(result.is_err(), "private redirect should be rejected");
        server.await.expect("origin task");
        assert!(
            target_probe.await.expect("target probe").is_err(),
            "blocked redirect target must not receive a connection"
        );
    }

    #[tokio::test]
    async fn admission_runs_for_the_original_and_redirect_destination() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let origin = TcpListener::bind("127.0.0.1:0").await.expect("origin");
        let target = TcpListener::bind("127.0.0.1:0").await.expect("target");
        let origin_addr = origin.local_addr().expect("origin address");
        let target_addr = target.local_addr().expect("target address");
        let origin_task = tokio::spawn(async move {
            let (mut stream, _) = origin.accept().await.expect("origin accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://{target_addr}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .expect("write redirect");
        });
        let target_task = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.expect("target accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("write final response");
        });
        let admissions = Arc::new(Mutex::new(Vec::new()));
        let observed = admissions.clone();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client");
        let (checked, final_guard) = checked_get_with_admission(
            &client,
            &format!("http://{origin_addr}/start"),
            SsrfPolicy::Default,
            &[],
            5,
            None,
            move |url| {
                let observed = observed.clone();
                let origin = url.origin().ascii_serialization();
                async move {
                    observed.lock().expect("admissions").push(origin.clone());
                    origin
                }
            },
        )
        .await
        .expect("redirect fetch");

        assert_eq!(checked.response.status(), reqwest::StatusCode::OK);
        assert!(final_guard.contains(&target_addr.port().to_string()));
        let observed = admissions.lock().expect("admissions");
        assert_eq!(observed.len(), 2);
        assert!(observed[0].contains(&origin_addr.port().to_string()));
        assert!(observed[1].contains(&target_addr.port().to_string()));
        origin_task.await.expect("origin task");
        target_task.await.expect("target task");
    }
}
