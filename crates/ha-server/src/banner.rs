//! Human-readable launch banner printed to stderr after the HTTP server
//! binds to its configured address.
//!
//! Lives in `ha-server` (not `src-tauri`) because the server is the
//! component that actually knows when the port is listening. Both the
//! desktop GUI and the CLI `server start` path call into
//! [`print_launch_banner`]; the Tauri command layer re-exports
//! [`local_ipv4_addresses`] for the onboarding Summary step so the Web
//! URL QR code can use a LAN-reachable IP.

use std::net::IpAddr;

/// Enumerate non-loopback, non-link-local IPv4 addresses across all
/// interfaces, capped at 3 entries. Returns an empty Vec on error (e.g.
/// kernel APIs unavailable in a minimal container).
pub fn local_ipv4_addresses() -> Vec<String> {
    match local_ip_address::list_afinet_netifas() {
        Ok(list) => list
            .into_iter()
            .filter_map(|(_name, ip)| match ip {
                IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(v4.to_string()),
                _ => None,
            })
            .take(3)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Expand a bind string (e.g. `0.0.0.0:8420`) into user-friendly base URLs.
/// For wildcard binds we emit `http://localhost:PORT` plus up to three
/// LAN IPs so the user can pick whichever matches their network path.
pub fn display_host_urls(bind_addr: &str) -> Vec<String> {
    let (host, port) = match bind_addr.rsplit_once(':') {
        Some((h, p)) => (h, p),
        None => return vec![format!("http://{}", bind_addr)],
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');

    let hosts: Vec<String> = match host {
        "0.0.0.0" | "::" => {
            let mut v = vec!["localhost".to_string()];
            v.extend(local_ipv4_addresses());
            v
        }
        "127.0.0.1" | "::1" => vec!["localhost".to_string()],
        other => vec![other.to_string()],
    };

    hosts
        .into_iter()
        .map(|h| format!("http://{}:{}", h, port))
        .collect()
}

/// Print the "Hope Agent is running" banner. `api_key` is substituted into
/// a `?token=` query param so the copyable URL logs the user in
/// automatically when clicked; passing `None` hides the key row.
///
/// In headless deployments (`HA_DEPLOYMENT` env var set, or stderr not a
/// TTY — systemd, launchd, Docker, CI) the API key is masked and the
/// `?token=` URL suffix is dropped: stderr in those contexts flows into
/// `docker logs` / journalctl / log collectors and an unmasked Bearer
/// token there is effectively a credential leak.
pub fn print_launch_banner(bind_addr: &str, api_key: Option<&str>) {
    let bases = display_host_urls(bind_addr);
    let mask = should_mask_secrets();
    let token_suffix = match (api_key, mask) {
        (Some(k), false) => format!("/?token={}", k),
        _ => "/".to_string(),
    };

    eprintln!();
    eprintln!("╔═══════════════════════════════════════════════════════════════╗");
    eprintln!("║  Hope Agent is running                                        ║");
    eprintln!("╟───────────────────────────────────────────────────────────────╢");
    for (i, base) in bases.iter().enumerate() {
        let label = if i == 0 { "🌐 Web GUI" } else { "          " };
        eprintln!("║  {} : {}{}", label, base, token_suffix);
        if i == 0 {
            eprintln!("║  🔌 API     : {}/api", base);
        }
    }
    if let Some(key) = api_key {
        if mask {
            eprintln!(
                "║  🔑 API Key : {} (set; hidden — read it from your env / secrets store)",
                mask_secret(key)
            );
        } else {
            eprintln!("║  🔑 API Key : {}", key);
        }
    }
    eprintln!("║                                                               ║");
    eprintln!("║  💡 Open the Web GUI link in any browser for the full         ║");
    eprintln!("║     experience. Press Ctrl+C to stop the service.             ║");
    eprintln!("╚═══════════════════════════════════════════════════════════════╝");
    eprintln!();
}

/// Containers / systemd / launchd / Windows services route stderr to a
/// log collector, so unmasked Bearer tokens in the banner become a
/// credential leak. Honor `HA_DEPLOYMENT=docker` (or anything non-empty
/// — the variable is set by the Docker image's `ENV` and by service
/// installers) and also fall back to a TTY probe so locally-launched
/// `hope-agent server start` under a redirected shell mask too.
fn should_mask_secrets() -> bool {
    if std::env::var("HA_DEPLOYMENT")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    use std::io::IsTerminal;
    !std::io::stderr().is_terminal()
}

/// Render a secret as `xxxx…yyyy` so log scrapers can tell the API key
/// is configured without learning its value. Short strings collapse to
/// pure asterisks to avoid leaking high-entropy prefixes.
fn mask_secret(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let prefix: String = chars.iter().take(4).collect();
    let suffix: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{}…{}", prefix, suffix)
}

/// Printed when `server start` runs without a completed onboarding on a
/// non-TTY stdin (systemd, Docker, CI). Tells the operator the service
/// is starting with defaults and points at the Web GUI for finishing
/// setup.
pub fn print_unconfigured_notice(bind_addr: &str) {
    let base = display_host_urls(bind_addr)
        .into_iter()
        .next()
        .unwrap_or_else(|| format!("http://{}", bind_addr));
    eprintln!();
    eprintln!("⚠  Hope Agent has not completed first-run setup.");
    eprintln!("   Non-interactive stdin detected — starting with defaults.");
    eprintln!("   Finish configuration in the Web GUI: {}/", base);
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_secret_keeps_endpoints_for_long_inputs() {
        assert_eq!(mask_secret("abcdefghijklmnop"), "abcd…mnop");
    }

    #[test]
    fn mask_secret_redacts_short_inputs_completely() {
        assert_eq!(mask_secret("short"), "*****");
        assert_eq!(mask_secret("12345678"), "********");
    }
}
