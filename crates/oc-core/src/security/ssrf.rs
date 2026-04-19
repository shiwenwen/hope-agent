use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

// ── Types ────────────────────────────────────────────────────────

/// SSRF policy governing which destination hosts are allowed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SsrfPolicy {
    /// Block loopback + private + link-local + metadata + unspecified + broadcast.
    Strict,
    /// Block private + link-local + metadata + unspecified + broadcast; allow loopback.
    #[default]
    Default,
    /// Allow loopback + private; still block link-local + metadata + unspecified + broadcast.
    AllowPrivate,
}

/// Classification of a resolved IP address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    Public,
    Loopback,
    Private,
    LinkLocal,
    Metadata,
    Unspecified,
    Broadcast,
}

/// Global SSRF configuration. Lives in [`crate::config::AppConfig::ssrf`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SsrfConfig {
    /// Fallback policy for outbound contexts that do not set an explicit policy.
    #[serde(default)]
    pub default_policy: SsrfPolicy,
    /// Hosts trusted by the user — matched before any policy check runs.
    #[serde(default)]
    pub trusted_hosts: Vec<String>,
    /// Override for the browser tool (default: inherit `default_policy`).
    #[serde(default)]
    pub browser_policy: Option<SsrfPolicy>,
    /// Override for the web_fetch tool.
    #[serde(default)]
    pub web_fetch_policy: Option<SsrfPolicy>,
    /// Override for the image_generate tool's URL download path.
    #[serde(default)]
    pub image_generate_policy: Option<SsrfPolicy>,
    /// Override for url_preview.
    #[serde(default)]
    pub url_preview_policy: Option<SsrfPolicy>,
}

impl Default for SsrfConfig {
    fn default() -> Self {
        Self {
            default_policy: SsrfPolicy::Default,
            trusted_hosts: Vec::new(),
            browser_policy: None,
            web_fetch_policy: None,
            image_generate_policy: None,
            url_preview_policy: None,
        }
    }
}

impl SsrfConfig {
    pub fn browser(&self) -> SsrfPolicy {
        self.browser_policy.unwrap_or(self.default_policy)
    }
    pub fn web_fetch(&self) -> SsrfPolicy {
        self.web_fetch_policy.unwrap_or(self.default_policy)
    }
    pub fn image_generate(&self) -> SsrfPolicy {
        self.image_generate_policy.unwrap_or(self.default_policy)
    }
    pub fn url_preview(&self) -> SsrfPolicy {
        self.url_preview_policy.unwrap_or(self.default_policy)
    }
}

// ── Metadata hard blocklist ──────────────────────────────────────

/// Cloud metadata IPs that are blocked regardless of policy.
const METADATA_IPS: &[&str] = &[
    "169.254.169.254", // AWS / GCP / Azure IMDS
    "169.254.170.2",   // ECS Task Metadata
    "100.100.100.200", // Aliyun
    "fd00:ec2::254",   // EC2 IMDSv6
];

fn is_metadata_ip(ip: &IpAddr) -> bool {
    for raw in METADATA_IPS {
        if let Ok(parsed) = raw.parse::<IpAddr>() {
            if parsed == *ip {
                return true;
            }
        }
    }
    // IPv4-mapped IPv6 (::ffff:a.b.c.d) — unwrap and re-check against v4 metadata.
    if let IpAddr::V6(v6) = ip {
        if let Some(v4) = v6.to_ipv4_mapped() {
            return is_metadata_ip(&IpAddr::V4(v4));
        }
    }
    false
}

// ── Classification ──────────────────────────────────────────────

/// Classify an IP address into a [`HostKind`].
pub fn classify_ip(ip: &IpAddr) -> HostKind {
    if is_metadata_ip(ip) {
        return HostKind::Metadata;
    }
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_unspecified() || v4.octets()[0] == 0 {
                HostKind::Unspecified
            } else if v4.is_loopback() {
                HostKind::Loopback
            } else if v4.is_broadcast() {
                HostKind::Broadcast
            } else if v4.is_link_local() {
                HostKind::LinkLocal
            } else if v4.is_private() {
                HostKind::Private
            } else {
                HostKind::Public
            }
        }
        IpAddr::V6(v6) => {
            // Resolve IPv4-mapped to v4 classification first (except metadata handled above).
            if let Some(v4) = v6.to_ipv4_mapped() {
                return classify_ip(&IpAddr::V4(v4));
            }
            if v6.is_unspecified() {
                HostKind::Unspecified
            } else if v6.is_loopback() {
                HostKind::Loopback
            } else if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                HostKind::LinkLocal
            } else if (v6.segments()[0] & 0xfe00) == 0xfc00 {
                HostKind::Private
            } else {
                HostKind::Public
            }
        }
    }
}

/// Resolve `host` via DNS and return each address with its classification.
pub async fn resolve_and_classify(host: &str, port: u16) -> Result<Vec<(IpAddr, HostKind)>> {
    let addr_str = format!("{}:{}", host, port);
    let addrs = tokio::net::lookup_host(&addr_str)
        .await
        .map_err(|e| anyhow!("DNS resolution failed for {}: {}", host, e))?;

    let mut out = Vec::new();
    for addr in addrs {
        let ip = addr.ip();
        out.push((ip, classify_ip(&ip)));
    }
    if out.is_empty() {
        return Err(anyhow!("DNS returned no records for {}", host));
    }
    Ok(out)
}

// ── Allowlist ────────────────────────────────────────────────────

/// Check whether `host` (optionally including port) matches a user-trusted entry.
/// Supports exact match and a single leading wildcard like `*.example.com`.
pub fn is_in_allowlist(host: &str, allowlist: &[String]) -> bool {
    let host_lower = host.to_ascii_lowercase();
    for entry in allowlist {
        let entry_lower = entry.trim().to_ascii_lowercase();
        if entry_lower.is_empty() {
            continue;
        }
        if entry_lower == host_lower {
            return true;
        }
        if let Some(suffix) = entry_lower.strip_prefix("*.") {
            if host_lower == suffix || host_lower.ends_with(&format!(".{}", suffix)) {
                return true;
            }
        }
    }
    false
}

// ── Policy decision ──────────────────────────────────────────────

fn policy_allows(policy: SsrfPolicy, kind: HostKind) -> bool {
    match kind {
        HostKind::Metadata | HostKind::Unspecified | HostKind::Broadcast | HostKind::LinkLocal => {
            false
        }
        HostKind::Public => true,
        HostKind::Loopback => matches!(policy, SsrfPolicy::Default | SsrfPolicy::AllowPrivate),
        HostKind::Private => matches!(policy, SsrfPolicy::AllowPrivate),
    }
}

// ── Public entry points ──────────────────────────────────────────

/// Parse + DNS-resolve + classify + apply policy. Returns the parsed URL on success.
/// Allowlist matches on both the bare host and `host:port` form.
pub async fn check_url(
    url_str: &str,
    policy: SsrfPolicy,
    allowlist: &[String],
) -> Result<url::Url> {
    let parsed = url::Url::parse(url_str).map_err(|e| anyhow!("Invalid URL: {}", e))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(anyhow!("Blocked URL scheme: {}", other)),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("URL has no host"))?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let host_port = format!("{}:{}", host, port);

    if is_in_allowlist(host, allowlist) || is_in_allowlist(&host_port, allowlist) {
        return Ok(parsed);
    }

    // Literal IP hosts: skip DNS, classify directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        let kind = classify_ip(&ip);
        if !policy_allows(policy, kind) {
            return Err(anyhow!(
                "SSRF policy {:?} blocked {} ({:?})",
                policy,
                ip,
                kind
            ));
        }
        return Ok(parsed);
    }

    // Hostname: resolve and require every returned IP to pass. Guards against
    // DNS rebinding where a single hostname resolves to mixed public/private.
    let resolved = resolve_and_classify(host, port).await?;
    for (ip, kind) in &resolved {
        if !policy_allows(policy, *kind) {
            return Err(anyhow!(
                "SSRF policy {:?} blocked {} → {} ({:?})",
                policy,
                host,
                ip,
                kind
            ));
        }
    }
    Ok(parsed)
}

/// Synchronous host check for use inside reqwest redirect policy callbacks.
/// Does not perform DNS — only classifies literal IPs and rejects known
/// blocked hostnames (`localhost`, `*.localhost`). Returns true when the host
/// should be blocked.
pub fn check_host_blocking_sync(host: &str, policy: SsrfPolicy, allowlist: &[String]) -> bool {
    if is_in_allowlist(host, allowlist) {
        return false;
    }

    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        let kind = HostKind::Loopback;
        return !policy_allows(policy, kind);
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        let kind = classify_ip(&ip);
        return !policy_allows(policy, kind);
    }

    // Unknown hostname — cannot classify without DNS; allow and rely on the
    // next request's `check_url` to re-resolve if reqwest follows the redirect.
    false
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn classify_ipv4() {
        assert_eq!(classify_ip(&ip("127.0.0.1")), HostKind::Loopback);
        assert_eq!(classify_ip(&ip("10.0.0.1")), HostKind::Private);
        assert_eq!(classify_ip(&ip("172.16.0.1")), HostKind::Private);
        assert_eq!(classify_ip(&ip("192.168.1.1")), HostKind::Private);
        assert_eq!(classify_ip(&ip("169.254.1.1")), HostKind::LinkLocal);
        assert_eq!(classify_ip(&ip("169.254.169.254")), HostKind::Metadata);
        assert_eq!(classify_ip(&ip("169.254.170.2")), HostKind::Metadata);
        assert_eq!(classify_ip(&ip("100.100.100.200")), HostKind::Metadata);
        assert_eq!(classify_ip(&ip("0.0.0.0")), HostKind::Unspecified);
        assert_eq!(classify_ip(&ip("0.1.2.3")), HostKind::Unspecified);
        assert_eq!(classify_ip(&ip("255.255.255.255")), HostKind::Broadcast);
        assert_eq!(classify_ip(&ip("8.8.8.8")), HostKind::Public);
        assert_eq!(classify_ip(&ip("1.1.1.1")), HostKind::Public);
    }

    #[test]
    fn classify_ipv6() {
        assert_eq!(classify_ip(&ip("::1")), HostKind::Loopback);
        assert_eq!(classify_ip(&ip("::")), HostKind::Unspecified);
        assert_eq!(classify_ip(&ip("fc00::1")), HostKind::Private);
        assert_eq!(classify_ip(&ip("fd12:3456::1")), HostKind::Private);
        assert_eq!(classify_ip(&ip("fe80::1")), HostKind::LinkLocal);
        assert_eq!(classify_ip(&ip("2001:db8::1")), HostKind::Public);
        // IPv4-mapped should fall through to v4 classification
        assert_eq!(
            classify_ip(&ip("::ffff:169.254.169.254")),
            HostKind::Metadata
        );
        assert_eq!(classify_ip(&ip("::ffff:127.0.0.1")), HostKind::Loopback);
        assert_eq!(classify_ip(&ip("::ffff:8.8.8.8")), HostKind::Public);
    }

    #[test]
    fn policy_decision_matrix() {
        // Strict: only Public passes
        assert!(policy_allows(SsrfPolicy::Strict, HostKind::Public));
        assert!(!policy_allows(SsrfPolicy::Strict, HostKind::Loopback));
        assert!(!policy_allows(SsrfPolicy::Strict, HostKind::Private));
        assert!(!policy_allows(SsrfPolicy::Strict, HostKind::Metadata));

        // Default: Public + Loopback pass
        assert!(policy_allows(SsrfPolicy::Default, HostKind::Public));
        assert!(policy_allows(SsrfPolicy::Default, HostKind::Loopback));
        assert!(!policy_allows(SsrfPolicy::Default, HostKind::Private));
        assert!(!policy_allows(SsrfPolicy::Default, HostKind::LinkLocal));
        assert!(!policy_allows(SsrfPolicy::Default, HostKind::Metadata));

        // AllowPrivate: everything except metadata / link-local / unspecified / broadcast
        assert!(policy_allows(SsrfPolicy::AllowPrivate, HostKind::Public));
        assert!(policy_allows(SsrfPolicy::AllowPrivate, HostKind::Loopback));
        assert!(policy_allows(SsrfPolicy::AllowPrivate, HostKind::Private));
        assert!(!policy_allows(
            SsrfPolicy::AllowPrivate,
            HostKind::LinkLocal
        ));
        assert!(!policy_allows(SsrfPolicy::AllowPrivate, HostKind::Metadata));
        assert!(!policy_allows(
            SsrfPolicy::AllowPrivate,
            HostKind::Unspecified
        ));
        assert!(!policy_allows(
            SsrfPolicy::AllowPrivate,
            HostKind::Broadcast
        ));
    }

    #[test]
    fn allowlist_exact_and_wildcard() {
        let list = vec![
            "127.0.0.1:11434".into(),
            "ollama.local".into(),
            "*.trusted.example".into(),
        ];
        assert!(is_in_allowlist("127.0.0.1:11434", &list));
        assert!(is_in_allowlist("ollama.local", &list));
        assert!(is_in_allowlist("OLLAMA.local", &list));
        assert!(is_in_allowlist("api.trusted.example", &list));
        assert!(is_in_allowlist("deep.nested.trusted.example", &list));
        assert!(is_in_allowlist("trusted.example", &list)); // apex allowed too
        assert!(!is_in_allowlist("127.0.0.1", &list)); // port mismatch
        assert!(!is_in_allowlist("notrustedexample", &list));
        assert!(!is_in_allowlist("other.example", &list));
    }

    #[tokio::test]
    async fn check_url_rejects_literal_metadata_ip() {
        let err = check_url(
            "http://169.254.169.254/latest/meta-data",
            SsrfPolicy::AllowPrivate,
            &[],
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Metadata"));
    }

    #[tokio::test]
    async fn check_url_rejects_private_in_default() {
        let err = check_url("http://10.0.0.1/", SsrfPolicy::Default, &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Private"));
    }

    #[tokio::test]
    async fn check_url_allows_loopback_in_default() {
        let ok = check_url("http://127.0.0.1:3000/", SsrfPolicy::Default, &[]).await;
        assert!(ok.is_ok(), "expected loopback allowed, got {:?}", ok);
    }

    #[tokio::test]
    async fn check_url_rejects_loopback_in_strict() {
        let err = check_url("http://127.0.0.1:3000/", SsrfPolicy::Strict, &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Loopback"));
    }

    #[tokio::test]
    async fn check_url_allows_private_with_policy() {
        let ok = check_url("http://192.168.1.100:11434/", SsrfPolicy::AllowPrivate, &[]).await;
        assert!(ok.is_ok(), "expected private allowed, got {:?}", ok);
    }

    #[tokio::test]
    async fn check_url_rejects_non_http_scheme() {
        let err = check_url("file:///etc/passwd", SsrfPolicy::Default, &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("scheme"));
    }

    #[tokio::test]
    async fn check_url_allowlist_bypasses_policy() {
        let list = vec!["127.0.0.1:22".into()];
        let ok = check_url("http://127.0.0.1:22/", SsrfPolicy::Strict, &list).await;
        assert!(
            ok.is_ok(),
            "expected allowlist to bypass Strict, got {:?}",
            ok
        );
    }

    #[test]
    fn allowlist_wildcard_matches_local_tld() {
        let list: Vec<String> = vec!["*.local".into()];
        assert!(is_in_allowlist("ollama.local", &list));
        assert!(is_in_allowlist("foo.bar.local", &list));
        assert!(!is_in_allowlist("ollama.notlocal", &list));
    }

    #[test]
    fn redirect_callback_behavior() {
        // Loopback host in Strict → block.
        assert!(check_host_blocking_sync(
            "127.0.0.1",
            SsrfPolicy::Strict,
            &[]
        ));
        // Loopback host in Default → allow.
        assert!(!check_host_blocking_sync(
            "127.0.0.1",
            SsrfPolicy::Default,
            &[]
        ));
        // Localhost literal.
        assert!(check_host_blocking_sync(
            "localhost",
            SsrfPolicy::Strict,
            &[]
        ));
        assert!(!check_host_blocking_sync(
            "localhost",
            SsrfPolicy::Default,
            &[]
        ));
        // Metadata always blocked.
        assert!(check_host_blocking_sync(
            "169.254.169.254",
            SsrfPolicy::AllowPrivate,
            &[]
        ));
        // Public passes in any policy.
        assert!(!check_host_blocking_sync(
            "8.8.8.8",
            SsrfPolicy::Strict,
            &[]
        ));
        // Allowlist bypasses.
        let list = vec!["127.0.0.1".into()];
        assert!(!check_host_blocking_sync(
            "127.0.0.1",
            SsrfPolicy::Strict,
            &list
        ));
    }
}
