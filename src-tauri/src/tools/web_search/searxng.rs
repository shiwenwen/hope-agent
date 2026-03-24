use anyhow::Result;
use serde_json::Value;

use super::helpers::build_search_client;
use super::{SearchResult, SearchParams};

pub(super) async fn search_searxng(instance_url: &str, query: &str, count: usize, params: &SearchParams, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    let client = build_search_client(timeout_secs)?;
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
    let resp = client.get(&url).send().await
        .map_err(|e| anyhow::anyhow!("SearXNG request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("SearXNG failed with status: {}", resp.status()));
    }
    let body: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("SearXNG JSON parse failed: {}", e))?;
    let results = body.get("results").and_then(|v| v.as_array());
    Ok(results.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect()
    }))
}
