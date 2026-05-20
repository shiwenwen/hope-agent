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

/// Apply proxy settings to a reqwest blocking ClientBuilder based on global config.
pub fn apply_proxy_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    let config = load_proxy_config();
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
}
