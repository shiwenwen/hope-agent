use super::info;

/// Resolve the proxy URL to inject into the Docker container.
/// - Custom mode: use the configured URL
/// - System mode: env vars → desktop system proxy fallback
/// - None mode: no proxy
/// All localhost/127.0.0.1 addresses are rewritten to host.docker.internal.
pub(crate) fn resolve_proxy_for_container() -> Option<String> {
    if !crate::config::cached_config()
        .web_search
        .searxng_docker_use_proxy
    {
        return None;
    }

    let config = crate::provider::load_proxy_config();
    let raw_url = match config.mode {
        crate::provider::ProxyMode::Custom => config.url.filter(|u| !u.is_empty()),
        crate::provider::ProxyMode::System => {
            // 1. Try env vars first
            std::env::var("HTTPS_PROXY")
                .ok()
                .or_else(|| std::env::var("HTTP_PROXY").ok())
                .or_else(|| std::env::var("ALL_PROXY").ok())
                .or_else(|| std::env::var("https_proxy").ok())
                .or_else(|| std::env::var("http_proxy").ok())
                .or_else(|| std::env::var("all_proxy").ok())
                .filter(|u| !u.is_empty())
                // 2. Fallback: read desktop system proxy (macOS Network,
                // GNOME, KDE, Windows registry, etc.).
                .or_else(detect_platform_system_proxy)
        }
        crate::provider::ProxyMode::None => return None,
    };
    raw_url.map(|u| {
        // Docker containers can't reach host's 127.0.0.1; use special DNS name
        u.replace("127.0.0.1", "host.docker.internal")
            .replace("localhost", "host.docker.internal")
    })
}

fn detect_platform_system_proxy() -> Option<String> {
    let url = crate::platform::detect_system_proxy()?;
    info(&format!("Detected system proxy: {}", url));
    Some(url)
}
