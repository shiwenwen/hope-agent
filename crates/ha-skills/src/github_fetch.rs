//! Bounded public GitHub reads for the bundled installer. This CLI runs inside
//! the caller's ordinary exec/sandbox boundary; it never writes package files,
//! loads credentials, starts a chat runtime, or delegates to the host.

use std::io::Write;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use ha_core::security::{http_stream::read_bytes_capped, ssrf};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const OUTPUT_PREFIX: &[u8] = b"hope-skill-fetch-v1\n";

fn request_accept(url: &url::Url) -> Result<&'static str> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        bail!("skill_source_url_rejected");
    }
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    match (url.host_str(), parts.as_slice()) {
        (Some("api.github.com"), ["repos", owner, repo, "commits", reference])
            if !owner.is_empty()
                && !repo.is_empty()
                && !reference.is_empty()
                && url.query().is_none() =>
        {
            Ok("application/vnd.github.sha")
        }
        (Some("api.github.com"), ["repos", owner, repo, "git", "trees", oid])
            if !owner.is_empty()
                && !repo.is_empty()
                && is_oid(oid)
                && matches!(url.query(), None | Some("recursive=1")) =>
        {
            Ok("application/vnd.github+json")
        }
        (Some("raw.githubusercontent.com"), [owner, repo, commit, path @ ..])
            if !owner.is_empty()
                && !repo.is_empty()
                && is_oid(commit)
                && !path.is_empty()
                && path.iter().all(|part| !part.is_empty())
                && url.query().is_none() =>
        {
            Ok("application/octet-stream")
        }
        _ => bail!("skill_source_url_rejected"),
    }
}

fn is_oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn response_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response.status() != reqwest::StatusCode::OK {
        // No response body, URL, or credentials in errors, including redirects.
        bail!("skill_source_http_status_{}", response.status().as_u16());
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        bail!("skill_source_response_too_large");
    }
    let bytes = read_bytes_capped(response, limit + 1)
        .await
        .map_err(|_| anyhow::anyhow!("skill_source_response_failed"))?;
    if bytes.len() > limit {
        bail!("skill_source_response_too_large");
    }
    Ok(bytes)
}

async fn fetch(raw: &str, limit: usize) -> Result<Vec<u8>> {
    let parsed = url::Url::parse(raw).context("skill_source_url_invalid")?;
    let accept = request_accept(&parsed)?;
    let url = ssrf::check_url(raw, ssrf::SsrfPolicy::Strict, &[])
        .await
        .map_err(|_| anyhow::anyhow!("skill_source_destination_rejected"))?;
    let host = url.host_str().context("skill_source_host_missing")?;
    // Pin the checked addresses; reqwest must not do a second unchecked lookup.
    let addresses = ssrf::resolve_checked_destination(host, 443, ssrf::SsrfPolicy::Strict, &[])
        .await
        .map_err(|_| anyhow::anyhow!("skill_source_destination_rejected"))?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .context("skill_source_client_failed")?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "Hope-Agent-Skill-Installer")
        .header(reqwest::header::ACCEPT, accept)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("skill_source_request_failed"))?;
    response_bytes(response, limit).await
}

/// Shared by desktop and headless binaries, before their runtime initialization.
pub fn run_cli(args: &[String]) -> Result<()> {
    let [url_flag, url, size_flag, size, timeout_flag, timeout] = args else {
        bail!("Usage: hope-agent skill-source-fetch --url URL --max-bytes N --timeout-ms N");
    };
    if url_flag != "--url" || size_flag != "--max-bytes" || timeout_flag != "--timeout-ms" {
        bail!("skill_source_arguments_invalid");
    }
    let limit: usize = size.parse().context("skill_source_limit_invalid")?;
    let timeout: u64 = timeout.parse().context("skill_source_timeout_invalid")?;
    if limit > MAX_BYTES || !(1..=180_000).contains(&timeout) || url.len() > 8192 {
        bail!("skill_source_limits_invalid");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let bytes = runtime.block_on(async {
        tokio::time::timeout(Duration::from_millis(timeout), fetch(url, limit))
            .await
            .context("skill_source_timeout")?
    })?;
    let mut output = std::io::stdout().lock();
    output.write_all(OUTPUT_PREFIX)?;
    output.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_limited_to_public_github_metadata_and_pinned_files() {
        let oid = "a".repeat(40);
        for raw in [
            "https://api.github.com/repos/owner/repo/commits/feature%2Fexample".to_owned(),
            format!("https://api.github.com/repos/owner/repo/git/trees/{oid}?recursive=1"),
            format!("https://raw.githubusercontent.com/owner/repo/{oid}/SKILL.md"),
        ] {
            assert!(request_accept(&url::Url::parse(&raw).unwrap()).is_ok());
        }
        for raw in [
            "http://api.github.com/repos/owner/repo/commits/main",
            "https://token@api.github.com/repos/owner/repo/commits/main",
            "https://api.github.com:444/repos/owner/repo/commits/main",
            "https://github.com.evil.example/repos/owner/repo/commits/main",
            "https://api.github.com/user",
            "https://api.github.com/repos/owner/repo/commits/main?token=secret",
            "https://raw.githubusercontent.com/owner/repo/main/SKILL.md",
        ] {
            assert!(request_accept(&url::Url::parse(raw).unwrap()).is_err());
        }
    }

    async fn fixture_response(bytes: &'static [u8]) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let mut received = 0;
            loop {
                let read = socket.read(&mut request[received..]).await.unwrap();
                assert_ne!(read, 0, "fixture request ended before its headers");
                received += read;
                if request[..received].ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(
                    received < request.len(),
                    "fixture request headers too large"
                );
            }
            socket.write_all(bytes).await.unwrap();
        });
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn both_declared_and_streamed_oversized_responses_fail() {
        for bytes in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n123456789"
                .as_slice(),
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n123456789".as_slice(),
        ] {
            let error = response_bytes(fixture_response(bytes).await, 8)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("too_large"));
        }
        let response = fixture_response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc").await;
        assert_eq!(response_bytes(response, 3).await.unwrap(), b"abc");
    }

    #[tokio::test]
    async fn redirects_are_rejected_without_following_the_location() {
        let response = fixture_response(
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\n\r\n",
        ).await;
        let error = response_bytes(response, 1024).await.unwrap_err();
        assert_eq!(error.to_string(), "skill_source_http_status_302");
    }
}
