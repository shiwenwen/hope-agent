use anyhow::Result;
use serde_json::Value;

use super::helpers::{build_search_client_for_url, read_text_capped, JSON_RESPONSE_BYTE_CAP};
use super::{SearchParams, SearchResult};

pub(super) async fn search_searxng(
    instance_url: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    let client = build_search_client_for_url(instance_url, timeout_secs)?;
    let mut url = format!(
        "{}/search?q={}&format=json&categories=general&pageno=1",
        instance_url.trim_end_matches('/'),
        urlencoding::encode(query)
    );
    if let Some(ref lang) = params.language {
        url.push_str(&format!("&language={}", urlencoding::encode(lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!("&time_range={}", urlencoding::encode(freshness)));
    }
    app_info!("tool", "web_search", "SearXNG request URL: {}", url);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("SearXNG request failed (url={}): {}", url, e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP)
            .await
            .unwrap_or_default();
        let preview = crate::truncate_utf8(&body, 1024);
        app_warn!(
            "tool",
            "web_search",
            "SearXNG failed with status {}: {}",
            status,
            preview
        );
        return Err(anyhow::anyhow!("SearXNG failed with status: {}", status));
    }
    let body_text = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP)
        .await
        .map_err(|e| anyhow::anyhow!("SearXNG response read failed: {}", e))?;
    let body: Value = serde_json::from_str(&body_text).map_err(|e| {
        let preview = crate::truncate_utf8(&body_text, 2048);
        app_warn!(
            "tool",
            "web_search",
            "SearXNG JSON parse failed: {}. Raw response ({}B):\n{}",
            e,
            body_text.len(),
            preview
        );
        anyhow::anyhow!("SearXNG JSON parse failed: {}", e)
    })?;
    let results = body.get("results").and_then(|v| v.as_array());
    let parsed: Vec<SearchResult> = results.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let source = item
                    .get("engines")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|e| e.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "SearXNG".into());
                Some(SearchResult {
                    title,
                    url,
                    snippet,
                    source,
                })
            })
            .collect()
    });
    if parsed.is_empty() {
        let preview = crate::truncate_utf8(&body_text, 2048);
        app_warn!(
            "tool",
            "web_search",
            "SearXNG returned 0 results. Raw JSON ({}B):\n{}",
            body_text.len(),
            preview
        );
    }
    Ok(parsed)
}
