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
pub fn print_launch_banner(bind_addr: &str, api_key: Option<&str>) {
    let bases = display_host_urls(bind_addr);
    let token_suffix = api_key
        .map(|k| format!("/?token={}", k))
        .unwrap_or_else(|| "/".to_string());

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
        eprintln!("║  🔑 API Key : {}", key);
    }
    eprintln!("║                                                               ║");
    eprintln!("║  💡 Open the Web GUI link in any browser for the full         ║");
    eprintln!("║     experience. Press Ctrl+C to stop the service.             ║");
    eprintln!("╚═══════════════════════════════════════════════════════════════╝");
    eprintln!();
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
