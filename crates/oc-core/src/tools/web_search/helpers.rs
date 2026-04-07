use anyhow::Result;

pub(super) const DEFAULT_WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

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
