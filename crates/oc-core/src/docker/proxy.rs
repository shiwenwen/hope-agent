use super::info;

/// Resolve the proxy URL to inject into the Docker container.
/// - Custom mode: use the configured URL
/// - System mode: env vars → macOS scutil --proxy fallback
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
                // 2. Fallback: read macOS system proxy (Shadowrocket, ClashX, etc.)
                .or_else(detect_macos_system_proxy)
        }
        crate::provider::ProxyMode::None => return None,
    };
    raw_url.map(|u| {
        // Docker containers can't reach host's 127.0.0.1; use special DNS name
        u.replace("127.0.0.1", "host.docker.internal")
            .replace("localhost", "host.docker.internal")
    })
}

/// Read macOS system proxy via `scutil --proxy`.
/// Returns e.g. `Some("http://127.0.0.1:1082")`.
#[cfg(target_os = "macos")]
fn detect_macos_system_proxy() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);

    // Parse "HTTPSEnable : 1", "HTTPSProxy : 127.0.0.1", "HTTPSPort : 1082"
    // Prefer HTTPS proxy, fallback to HTTP proxy
    for prefix in ["HTTPS", "HTTP"] {
        let enabled = text
            .lines()
            .find(|l| l.trim().starts_with(&format!("{}Enable", prefix)))
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        if !enabled {
            continue;
        }

        let host = text
            .lines()
            .find(|l| {
                l.trim().starts_with(&format!("{}Proxy", prefix))
                    && !l.contains("Enable")
                    && !l.contains("Port")
            })
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let port = text
            .lines()
            .find(|l| l.trim().starts_with(&format!("{}Port", prefix)))
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        if let (Some(h), Some(p)) = (host, port) {
            let url = format!("http://{}:{}", h, p);
            info(&format!("Detected macOS system proxy: {}", url));
            return Some(url);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn detect_macos_system_proxy() -> Option<String> {
    // Name kept for symmetry with the macOS branch — platform layer
    // caches per-process so Windows deploys don't re-read the registry.
    let url = crate::platform::detect_system_proxy()?;
    info(&format!("Detected system proxy: {}", url));
    Some(url)
}
