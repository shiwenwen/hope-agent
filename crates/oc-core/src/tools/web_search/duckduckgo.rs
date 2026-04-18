use anyhow::Result;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

use super::helpers::{
    html_decode, read_text_capped, strip_html_tags, DEFAULT_WEB_FETCH_USER_AGENT,
    HTML_RESPONSE_BYTE_CAP, JSON_RESPONSE_BYTE_CAP,
};
use super::{SearchResult, DEFAULT_WEB_SEARCH_TIMEOUT_SECS};

/// Timestamp (epoch secs) until which DDG is rate-limited. Skip requests until then.
static DDG_RATE_LIMITED_UNTIL: AtomicU64 = AtomicU64::new(0);
/// Cooldown period after DDG rate-limits us (seconds).
const DDG_RATE_LIMIT_COOLDOWN_SECS: u64 = 30;

fn ddg_is_rate_limited() -> bool {
    let until = DDG_RATE_LIMITED_UNTIL.load(Ordering::Relaxed);
    if until == 0 {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now < until
}

fn ddg_mark_rate_limited() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    DDG_RATE_LIMITED_UNTIL.store(now + DDG_RATE_LIMIT_COOLDOWN_SECS, Ordering::Relaxed);
}

pub(super) async fn search_duckduckgo(
    query: &str,
    count: usize,
    _timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    // Skip if recently rate-limited — don't waste time on requests that will be 202'd
    if ddg_is_rate_limited() {
        app_warn!(
            "tool",
            "web_search",
            "DDG rate-limit cooldown active, skipping"
        );
        return Ok(Vec::new());
    }

    let client = build_ddg_client()?;

    // 1. Try Instant Answer API first (structured JSON, high quality for factual queries)
    let instant = ddg_instant_answer(&client, query).await;

    // 2. Scrape HTML search results, fallback to Lite endpoint
    let mut results = match ddg_html_search(&client, query, count).await {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => {
            app_warn!(
                "tool",
                "web_search",
                "DDG HTML search returned 0 results, falling back to Lite endpoint"
            );
            ddg_lite_search(&client, query, count).await?
        }
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("anti-bot") || err_msg.contains("rate-limited") {
                // DDG is blocking/throttling us — Lite will likely fail too, skip it
                app_warn!(
                    "tool",
                    "web_search",
                    "DDG blocked ({}), skipping Lite fallback",
                    err_msg
                );
                Vec::new()
            } else {
                app_warn!(
                    "tool",
                    "web_search",
                    "DDG HTML search failed: {}, falling back to Lite endpoint",
                    e
                );
                ddg_lite_search(&client, query, count).await?
            }
        }
    };

    if results.is_empty() && instant.is_none() {
        app_warn!(
            "tool",
            "web_search",
            "DDG all endpoints returned 0 results for query: {}",
            query
        );
    }

    // 3. Prepend instant answer if we got one and it's useful
    if let Some(ia) = instant {
        results.insert(0, ia);
    }

    // 4. Deduplicate by URL
    let mut seen = std::collections::HashSet::new();
    results.retain(|r| {
        if r.url.is_empty() {
            return true;
        }
        seen.insert(r.url.clone())
    });

    results.truncate(count);
    Ok(results)
}

/// Build a client with browser-like headers to avoid DDG bot detection.
fn build_ddg_client() -> Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, REFERER};
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    headers.insert(REFERER, HeaderValue::from_static("https://duckduckgo.com/"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));

    crate::provider::apply_proxy(
        reqwest::Client::builder()
            .user_agent(DEFAULT_WEB_FETCH_USER_AGENT)
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(
                DEFAULT_WEB_SEARCH_TIMEOUT_SECS,
            )),
    )
    .build()
    .map_err(|e| anyhow::anyhow!("Failed to create DDG HTTP client: {}", e))
}

/// DuckDuckGo Instant Answer API — returns structured data for factual queries.
async fn ddg_instant_answer(client: &reqwest::Client, query: &str) -> Option<SearchResult> {
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        urlencoding::encode(query)
    );
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP).await.ok()?;
    let data: Value = serde_json::from_str(&text).ok()?;

    // AbstractText + AbstractURL — encyclopedia-style answer
    let abstract_text = data
        .get("AbstractText")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let abstract_url = data
        .get("AbstractURL")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let abstract_source = data
        .get("AbstractSource")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !abstract_text.is_empty() && !abstract_url.is_empty() {
        return Some(SearchResult {
            title: format!("{} ({})", query, abstract_source),
            url: abstract_url.to_string(),
            snippet: abstract_text.chars().take(300).collect(),
            source: "DuckDuckGo".into(),
        });
    }

    // Answer field — direct factual answer
    let answer = data.get("Answer").and_then(|v| v.as_str()).unwrap_or("");
    if !answer.is_empty() {
        return Some(SearchResult {
            title: format!("{} — Instant Answer", query),
            url: String::new(),
            snippet: answer.to_string(),
            source: "DuckDuckGo".into(),
        });
    }

    None
}

/// Primary DDG search via the HTML endpoint.
async fn ddg_html_search(
    client: &reqwest::Client,
    query: &str,
    count: usize,
) -> Result<Vec<SearchResult>> {
    let search_url = "https://html.duckduckgo.com/html/";
    let resp = client
        .post(search_url)
        .form(&[("q", query), ("b", ""), ("kl", "")])
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("DuckDuckGo HTML request failed: {}", e))?;
    let status = resp.status();
    // DDG returns 202 when rate-limited
    if status == reqwest::StatusCode::ACCEPTED {
        app_warn!(
            "tool",
            "web_search",
            "DDG rate-limited (HTTP 202), cooldown {}s",
            DDG_RATE_LIMIT_COOLDOWN_SECS
        );
        ddg_mark_rate_limited();
        return Err(anyhow::anyhow!("DDG rate-limited (HTTP 202)"));
    }
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "DuckDuckGo HTML failed with status: {}",
            status
        ));
    }
    let html = read_text_capped(resp, HTML_RESPONSE_BYTE_CAP)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read DuckDuckGo response: {}", e))?;

    // Detect anti-bot redirect: DDG returns homepage instead of results
    if is_ddg_blocked(&html) {
        app_warn!(
            "tool",
            "web_search",
            "DDG HTML returned homepage (anti-bot/rate-limit), {}B response, cooldown {}s",
            html.len(),
            DDG_RATE_LIMIT_COOLDOWN_SECS
        );
        ddg_mark_rate_limited();
        return Err(anyhow::anyhow!("DDG blocked (anti-bot redirect)"));
    }

    let results = parse_ddg_results(&html, count);
    if results.is_empty() {
        let preview = crate::truncate_utf8(&html, 2048);
        app_warn!(
            "tool",
            "web_search",
            "DDG HTML parsed 0 results, raw response ({}B, preview {}B):\n{}",
            html.len(),
            preview.len(),
            preview
        );
    }
    Ok(results)
}

/// Fallback: DDG Lite endpoint (simpler HTML, more resilient).
async fn ddg_lite_search(
    client: &reqwest::Client,
    query: &str,
    count: usize,
) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        urlencoding::encode(query)
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("DuckDuckGo Lite request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "DuckDuckGo Lite failed with status: {}",
            resp.status()
        ));
    }
    let html = read_text_capped(resp, HTML_RESPONSE_BYTE_CAP)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read DDG Lite response: {}", e))?;
    let results = parse_ddg_lite_results(&html, count);
    if results.is_empty() {
        let preview = crate::truncate_utf8(&html, 2048);
        app_warn!(
            "tool",
            "web_search",
            "DDG Lite parsed 0 results, raw response ({}B, preview {}B):\n{}",
            html.len(),
            preview.len(),
            preview
        );
    }
    Ok(results)
}

fn parse_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < max_results {
        let link_marker = "class=\"result__a\"";
        let link_start = match html[pos..].find(link_marker) {
            Some(idx) => pos + idx,
            None => break,
        };

        let href_start = match html[..link_start].rfind("href=\"") {
            Some(idx) => idx + 6,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let href_end = match html[href_start..].find('"') {
            Some(idx) => href_start + idx,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let raw_url = &html[href_start..href_end];
        let url = extract_ddg_url(raw_url);

        let title_start = match html[link_start..].find('>') {
            Some(idx) => link_start + idx + 1,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title_end = match html[title_start..].find("</a>") {
            Some(idx) => title_start + idx,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title = strip_html_tags(&html[title_start..title_end]);

        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(snippet_start) = html[title_end..].find(snippet_marker) {
            let abs_snippet_start = title_end + snippet_start;
            if let Some(tag_end) = html[abs_snippet_start..].find('>') {
                let content_start = abs_snippet_start + tag_end + 1;
                // Try multiple end markers — DDG wraps snippets in <a> or <span>
                let end_pos = [
                    html[content_start..].find("</a>"),
                    html[content_start..].find("</span>"),
                    html[content_start..].find("</div>"),
                ]
                .iter()
                .filter_map(|x| *x)
                .min()
                .unwrap_or(0);
                if end_pos > 0 {
                    strip_html_tags(&html[content_start..content_start + end_pos])
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_decode(&title),
                url,
                snippet: html_decode(&snippet),
                source: "DuckDuckGo".into(),
            });
        }

        pos = title_end;
    }

    results
}

/// Detect if DDG returned its homepage instead of search results (anti-bot block).
fn is_ddg_blocked(html: &str) -> bool {
    // When blocked, DDG returns a page with canonical URL pointing to the homepage
    // and no search result markers at all
    let has_canonical_home = html.contains(r#"rel="canonical" href="https://duckduckgo.com/"#);
    let has_no_results = !html.contains("result__a")
        && !html.contains("result-link")
        && !html.contains("result__snippet");
    has_canonical_home && has_no_results
}

fn extract_ddg_url(raw: &str) -> String {
    if let Some(uddg_start) = raw.find("uddg=") {
        let url_start = uddg_start + 5;
        let url_end = raw[url_start..]
            .find('&')
            .map(|i| url_start + i)
            .unwrap_or(raw.len());
        let encoded = &raw[url_start..url_end];
        urlencoding::decode(encoded)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| encoded.to_string())
    } else if raw.starts_with("http") {
        raw.to_string()
    } else {
        raw.to_string()
    }
}

/// Parse DDG Lite HTML (table-based layout, simpler structure).
fn parse_ddg_lite_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;

    // DDG Lite uses <a rel="nofollow" ...> for result links inside <td> with class "result-link"
    while results.len() < max_results {
        // Find next result link
        let marker = "class=\"result-link\"";
        let block_start = match html[pos..].find(marker) {
            Some(idx) => pos + idx,
            None => break,
        };

        // Extract href
        let href = if let Some(a_start) = html[block_start..].find("href=\"") {
            let abs_start = block_start + a_start + 6;
            if let Some(end) = html[abs_start..].find('"') {
                html[abs_start..abs_start + end].to_string()
            } else {
                pos = block_start + marker.len();
                continue;
            }
        } else {
            pos = block_start + marker.len();
            continue;
        };

        // Extract title (text inside the <a> tag)
        let title = if let Some(tag_end) = html[block_start..].find('>') {
            let content_start = block_start + tag_end + 1;
            if let Some(a_end) = html[content_start..].find("</a>") {
                strip_html_tags(&html[content_start..content_start + a_end])
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Find snippet in the next <td class="result-snippet">
        let snippet_marker = "class=\"result-snippet\"";
        let snippet = if let Some(snip_start) = html[block_start..].find(snippet_marker) {
            let abs_snip = block_start + snip_start;
            if let Some(tag_end) = html[abs_snip..].find('>') {
                let content_start = abs_snip + tag_end + 1;
                if let Some(td_end) = html[content_start..].find("</td>") {
                    html_decode(&strip_html_tags(
                        &html[content_start..content_start + td_end],
                    ))
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let url = extract_ddg_url(&href);
        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_decode(&title),
                url,
                snippet,
                source: "DuckDuckGo".into(),
            });
        }

        pos = block_start + marker.len();
    }

    results
}
