//! Browser-style HTTP headers + Retry-After parsing shared by `web_fetch`.
//!
//! Cloudflare / Akamai's static-fingerprint tier rejects requests that
//! advertise a Chrome User-Agent but omit accompanying `Accept-*` and
//! `Sec-*` headers (the exact failure mode from session 770a1b6c, where
//! `web_fetch` returned bare `Fetch failed with status: 403` on a CF-
//! protected page). Installing the full header bundle defeats that tier.
//!
//! Dynamic JS challenges still need a real browser — the `browser` tool's
//! 8-action surface is the documented escape hatch.

pub const DEFAULT_BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// Headers a browser-like client sends on a top-level navigation. Compression
/// is intentionally omitted: reqwest installs the encodings that the compiled
/// decoder feature-set can actually decode.
pub const FETCH_BROWSER_HEADERS: &[(&str, &str)] = &[
    (
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
    ),
    ("Accept-Language", "en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7"),
    ("Cache-Control", "no-cache"),
    ("Pragma", "no-cache"),
    (
        "sec-ch-ua",
        "\"Chromium\";v=\"122\", \"Not(A:Brand\";v=\"24\", \"Google Chrome\";v=\"122\"",
    ),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", "\"macOS\""),
    ("Sec-Fetch-Dest", "document"),
    ("Sec-Fetch-Mode", "navigate"),
    ("Sec-Fetch-Site", "none"),
    ("Sec-Fetch-User", "?1"),
    ("Upgrade-Insecure-Requests", "1"),
];

/// Build the same header set as a reusable map for redirect-safe requests.
pub fn browser_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in FETCH_BROWSER_HEADERS {
        let Ok(name) = reqwest::header::HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = reqwest::header::HeaderValue::from_str(value) else {
            continue;
        };
        headers.insert(name, value);
    }
    headers
}

/// Browser-like headers whose client hints remain consistent with the chosen
/// User-Agent. A custom UA gets the generic navigation headers only; sending
/// fixed Chrome/macOS hints beside a Firefox, bot, or newer Chrome UA creates a
/// contradictory fingerprint and can reduce compatibility.
pub fn browser_headers_for_user_agent(user_agent: &str) -> reqwest::header::HeaderMap {
    let mut headers = browser_headers();
    if user_agent.trim() != DEFAULT_BROWSER_USER_AGENT {
        for name in ["sec-ch-ua", "sec-ch-ua-mobile", "sec-ch-ua-platform"] {
            headers.remove(name);
        }
    }
    headers
}

/// Install [`FETCH_BROWSER_HEADERS`] on a `reqwest::RequestBuilder`.
pub fn apply_browser_headers(mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    for (k, v) in FETCH_BROWSER_HEADERS {
        rb = rb.header(*k, *v);
    }
    rb
}

/// Parse a `Retry-After` header as integer seconds or an HTTP-date, capped at
/// `cap`. The cap prevents an origin from parking a tool worker for hours.
pub fn retry_after_seconds(h: Option<&reqwest::header::HeaderValue>, cap: u64) -> Option<u64> {
    let raw = h?.to_str().ok()?;
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(seconds.min(cap));
    }
    let when = httpdate::parse_http_date(raw).ok()?;
    let seconds = when
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default()
        .as_secs();
    Some(seconds.min(cap))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn retry_after_seconds_parses_integer_and_caps() {
        assert_eq!(
            retry_after_seconds(Some(&HeaderValue::from_static("3")), 5),
            Some(3)
        );
        assert_eq!(
            retry_after_seconds(Some(&HeaderValue::from_static("99")), 5),
            Some(5)
        );
    }

    #[test]
    fn retry_after_seconds_rejects_garbage_and_accepts_dates() {
        assert_eq!(retry_after_seconds(None, 5), None);
        assert_eq!(
            retry_after_seconds(Some(&HeaderValue::from_static("not-a-number")), 5),
            None
        );
        assert_eq!(
            retry_after_seconds(
                Some(&HeaderValue::from_static("Wed, 21 Oct 2099 07:28:00 GMT")),
                5
            ),
            Some(5)
        );
    }

    #[test]
    fn apply_browser_headers_installs_sec_ch_ua_and_friends() {
        let client = reqwest::Client::new();
        let rb = client.get("http://example.com");
        let rb = apply_browser_headers(rb);
        let req = rb.build().expect("request build");
        let headers = req.headers();
        // Spot-check the markers CF specifically looks for.
        assert!(headers.get("sec-ch-ua").is_some());
        assert!(headers.get("sec-ch-ua-platform").is_some());
        assert!(headers.get("Sec-Fetch-Mode").is_some());
        assert_eq!(
            headers.get("Accept-Language").unwrap().to_str().unwrap(),
            "en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7"
        );
    }
}
