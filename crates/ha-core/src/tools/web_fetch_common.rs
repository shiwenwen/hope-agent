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

/// Headers a real Chrome on macOS sends on a top-level navigation. The
/// `Accept-Encoding` deliberately omits `br` — `reqwest` only decodes
/// brotli with the `brotli` feature enabled, which is not on our default
/// build. Falsely advertising `br` would cause raw brotli bytes to land in
/// the response body.
pub const FETCH_BROWSER_HEADERS: &[(&str, &str)] = &[
    (
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
    ),
    ("Accept-Language", "en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7"),
    ("Accept-Encoding", "gzip, deflate"),
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

/// Install [`FETCH_BROWSER_HEADERS`] on a `reqwest::RequestBuilder`.
pub fn apply_browser_headers(mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    for (k, v) in FETCH_BROWSER_HEADERS {
        rb = rb.header(*k, *v);
    }
    rb
}

/// Parse a `Retry-After` header as integer seconds, capped at `cap`.
///
/// HTTP-date form (`Retry-After: Wed, 21 Oct 2015 07:28:00 GMT`) is rare
/// from CF / CDN edge — returning `None` for it keeps us free of a date-
/// parsing dependency. Cap defends against malicious servers asking us to
/// sleep for hours.
pub fn retry_after_seconds(h: Option<&reqwest::header::HeaderValue>, cap: u64) -> Option<u64> {
    h?.to_str().ok()?.parse::<u64>().ok().map(|n| n.min(cap))
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
    fn retry_after_seconds_rejects_garbage_and_dates() {
        assert_eq!(retry_after_seconds(None, 5), None);
        assert_eq!(
            retry_after_seconds(Some(&HeaderValue::from_static("not-a-number")), 5),
            None
        );
        // HTTP-date form intentionally unsupported.
        assert_eq!(
            retry_after_seconds(
                Some(&HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT")),
                5
            ),
            None
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
