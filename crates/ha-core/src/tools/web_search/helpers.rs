use anyhow::Result;

pub(super) use crate::security::http_stream::read_text_capped;

pub(super) const DEFAULT_WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// Cap for HTML scraping — larger because raw HTML carries boilerplate.
pub(super) const HTML_RESPONSE_BYTE_CAP: usize = 1_500_000;
/// Cap for JSON search-API responses — plenty for ≤10 results.
pub(super) const JSON_RESPONSE_BYTE_CAP: usize = 1_000_000;

/// SSRF-check a user-configurable search URL (only SearXNG uses this today;
/// vendor APIs are fixed public endpoints and don't need it).
pub(super) async fn check_search_url(url: &str) -> Result<()> {
    let cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(url, cfg.default_policy, &cfg.trusted_hosts)
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("SSRF check failed: {}", e))
}

/// Read the JSON body of a search API response with a byte cap, returning a
/// parsed `serde_json::Value`. `provider` is used to tag parse errors so the
/// caller doesn't need to construct an error context.
pub(super) async fn read_json_capped(
    resp: reqwest::Response,
    cap: usize,
    provider: &str,
) -> Result<serde_json::Value> {
    let text = read_text_capped(resp, cap).await?;
    serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("{} JSON parse failed: {}", provider, e))
}

pub(super) fn build_search_client(timeout_secs: u64) -> Result<reqwest::Client> {
    crate::provider::apply_proxy(
        reqwest::Client::builder()
            .user_agent(DEFAULT_WEB_FETCH_USER_AGENT)
            .timeout(std::time::Duration::from_secs(timeout_secs)),
    )
    .build()
    .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))
}

pub(super) fn build_search_client_for_url(
    target_url: &str,
    timeout_secs: u64,
) -> Result<reqwest::Client> {
    crate::provider::apply_proxy_for_url(
        reqwest::Client::builder()
            .user_agent(DEFAULT_WEB_FETCH_USER_AGENT)
            .timeout(std::time::Duration::from_secs(timeout_secs)),
        target_url,
    )
    .build()
    .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))
}

pub(super) fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result.trim().to_string()
}

pub(super) fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

pub(super) fn brave_freshness(f: &str) -> &str {
    match f {
        "day" => "pd",
        "week" => "pw",
        "month" => "pm",
        "year" => "py",
        _ => f,
    }
}

pub(super) fn google_date_restrict(f: &str) -> &str {
    match f {
        "day" => "d1",
        "week" => "w1",
        "month" => "m1",
        "year" => "y1",
        _ => f,
    }
}

pub(super) fn tavily_days(f: &str) -> u32 {
    match f {
        "day" => 1,
        "week" => 7,
        "month" => 30,
        "year" => 365,
        _ => 30,
    }
}
