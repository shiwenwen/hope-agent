use super::types::{ProxyConfig, ProxyMode};

/// Load global proxy config once.
pub fn load_proxy_config() -> ProxyConfig {
    crate::config::cached_config().proxy.clone()
}

/// Resolve the user-configured **custom** proxy URL (ignoring system-proxy
/// autodetection). Returns `None` unless the user explicitly set
/// [`ProxyMode::Custom`] with a non-empty URL. Used by bot SDKs (Telegram,
/// Discord, Slack, LINE) that should only honor explicit proxies and never
/// pick up an unexpected env-var / system proxy.
pub fn active_custom_proxy_url() -> Option<String> {
    let cfg = crate::config::cached_config();
    if matches!(cfg.proxy.mode, ProxyMode::Custom) {
        cfg.proxy
            .url
            .as_ref()
            .filter(|u: &&String| !u.is_empty())
            .cloned()
    } else {
        None
    }
}

/// Apply proxy settings to a reqwest async ClientBuilder based on global config.
pub fn apply_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    apply_proxy_from_config(builder, &load_proxy_config())
}

/// Model requests carry credentials and private inputs. A redirect is a
/// response to the configured endpoint, never permission to dispatch again.
pub fn apply_model_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    model_client_from_config(builder, &load_proxy_config())
}

fn model_client_from_config(
    builder: reqwest::ClientBuilder,
    config: &ProxyConfig,
) -> reqwest::ClientBuilder {
    apply_proxy_from_config(builder.redirect(reqwest::redirect::Policy::none()), config)
}

/// Apply proxy settings for a specific target URL.
/// Loopback destinations should always bypass the global proxy, otherwise local
/// services like Docker-managed SearXNG or Chrome CDP can be routed into the
/// system proxy and fail unexpectedly.
pub fn apply_proxy_for_url(
    builder: reqwest::ClientBuilder,
    target_url: &str,
) -> reqwest::ClientBuilder {
    if should_bypass_proxy(target_url) {
        builder.no_proxy()
    } else {
        apply_proxy(builder)
    }
}

/// Apply proxy settings from a specific ProxyConfig (async builder).
pub fn apply_proxy_from_config(
    mut builder: reqwest::ClientBuilder,
    config: &ProxyConfig,
) -> reqwest::ClientBuilder {
    match config.mode {
        ProxyMode::System => {
            // reqwest default: reads HTTP_PROXY / HTTPS_PROXY / ALL_PROXY env vars.
            // Desktop proxy settings (macOS Network, GNOME, KDE, Windows registry)
            // often are not exported as env vars. Detect and apply them only
            // when env vars are empty so explicit shell config still wins.
            let has_env_proxy = [
                "HTTPS_PROXY",
                "HTTP_PROXY",
                "ALL_PROXY",
                "https_proxy",
                "http_proxy",
                "all_proxy",
            ]
            .iter()
            .any(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).is_some());
            if !has_env_proxy {
                if let Some(url) = crate::platform::detect_system_proxy() {
                    if let Ok(proxy) = reqwest::Proxy::all(&url) {
                        builder = builder.proxy(proxy);
                    }
                }
            }
        }
        ProxyMode::None => {
            builder = builder.no_proxy();
        }
        ProxyMode::Custom => {
            if let Some(ref url) = config.url {
                if !url.is_empty() {
                    if let Ok(proxy) = reqwest::Proxy::all(url) {
                        builder = builder.proxy(proxy);
                    }
                }
            }
        }
    }
    builder
}

fn should_bypass_proxy(target_url: &str) -> bool {
    let Ok(url) = url::Url::parse(target_url) else {
        return false;
    };

    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        None => false,
    }
}

/// Blocking model requests follow the same no-redirect credential boundary.
pub fn apply_model_proxy_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    model_blocking_client_from_config(builder, &load_proxy_config())
}

/// Apply proxy settings to a reqwest blocking ClientBuilder based on global config.
pub fn apply_proxy_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    apply_proxy_blocking_from_config(builder, &load_proxy_config())
}

fn model_blocking_client_from_config(
    builder: reqwest::blocking::ClientBuilder,
    config: &ProxyConfig,
) -> reqwest::blocking::ClientBuilder {
    apply_proxy_blocking_from_config(builder.redirect(reqwest::redirect::Policy::none()), config)
}

fn apply_proxy_blocking_from_config(
    builder: reqwest::blocking::ClientBuilder,
    config: &ProxyConfig,
) -> reqwest::blocking::ClientBuilder {
    match config.mode {
        ProxyMode::System => {
            let has_env_proxy = [
                "HTTPS_PROXY",
                "HTTP_PROXY",
                "ALL_PROXY",
                "https_proxy",
                "http_proxy",
                "all_proxy",
            ]
            .iter()
            .any(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).is_some());
            if !has_env_proxy {
                if let Some(url) = crate::platform::detect_system_proxy() {
                    if let Ok(proxy) = reqwest::Proxy::all(&url) {
                        return builder.proxy(proxy);
                    }
                }
            }
            builder
        }
        ProxyMode::None => builder.no_proxy(),
        ProxyMode::Custom => {
            if let Some(ref url) = config.url {
                if !url.is_empty() {
                    if let Ok(proxy) = reqwest::Proxy::all(url) {
                        return builder.proxy(proxy);
                    }
                }
            }
            builder
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_bypass_proxy;

    #[test]
    fn loopback_hosts_bypass_proxy() {
        assert!(should_bypass_proxy("http://localhost:8080/search?q=test"));
        assert!(should_bypass_proxy("http://127.0.0.1:8080/search?q=test"));
        assert!(should_bypass_proxy("http://[::1]:9222/json/version"));
    }

    #[test]
    fn remote_hosts_keep_proxy() {
        assert!(!should_bypass_proxy("https://duckduckgo.com/?q=test"));
        assert!(!should_bypass_proxy("http://192.168.1.10:8080"));
        assert!(!should_bypass_proxy("not-a-url"));
    }

    #[test]
    fn blocking_model_requests_do_not_follow_redirects() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        let destination = TcpListener::bind("127.0.0.1:0").unwrap();
        destination.set_nonblocking(true).unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_url = format!("http://{}/embedding", origin.local_addr().unwrap());
        let destination_url = format!("http://{}/redirect", destination.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = origin.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() < 8192);
            }
            let header_end = bytes
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap()
                + 4;
            let expected_body = b"synthetic-embedding-input";
            while bytes.len() < header_end + expected_body.len() {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() < 8192);
            }
            assert_eq!(&bytes[header_end..], expected_body);
            assert!(String::from_utf8_lossy(&bytes)
                .to_lowercase()
                .contains("authorization: bearer synthetic-key"));
            socket.write_all(format!("HTTP/1.1 307 Redirect\r\nLocation: {destination_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        });
        let config = super::ProxyConfig {
            mode: super::ProxyMode::None,
            ..Default::default()
        };
        let client = super::model_blocking_client_from_config(
            reqwest::blocking::Client::builder().timeout(Duration::from_secs(2)),
            &config,
        )
        .build()
        .unwrap();
        let response = client
            .post(origin_url)
            .bearer_auth("synthetic-key")
            .body("synthetic-embedding-input")
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 307);
        server.join().unwrap();
        assert_eq!(
            destination.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[tokio::test]
    async fn model_requests_do_not_replay_credentials_or_body_on_redirect() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::time::{timeout, Duration};

        for status in [302, 307, 308] {
            let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let destination_url =
                format!("http://{}/new-endpoint", destination.local_addr().unwrap());
            let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin_url = format!("http://{}/model", origin.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = origin.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 8192);
                }
                let header_end = request
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                let expected_body = b"synthetic-private-input";
                while request.len() < header_end + expected_body.len() {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 8192);
                }
                assert_eq!(&request[header_end..], expected_body);
                let headers = String::from_utf8_lossy(&request).to_lowercase();
                assert!(headers.contains("authorization: bearer synthetic-token"));
                assert!(headers.contains("x-api-key: synthetic-key"));
                socket.write_all(format!(
                    "HTTP/1.1 {status} Redirect\r\nLocation: {destination_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ).as_bytes()).await.unwrap();
            });
            let config = super::ProxyConfig {
                mode: super::ProxyMode::None,
                ..Default::default()
            };
            let client = super::model_client_from_config(
                reqwest::Client::builder().timeout(Duration::from_secs(2)),
                &config,
            )
            .build()
            .unwrap();
            let response = client
                .post(origin_url)
                .bearer_auth("synthetic-token")
                .header("x-api-key", "synthetic-key")
                .body("synthetic-private-input")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status);
            server.await.unwrap();
            assert!(timeout(Duration::from_millis(50), destination.accept())
                .await
                .is_err());
        }
    }
}
