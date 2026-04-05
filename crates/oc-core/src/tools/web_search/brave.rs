use anyhow::Result;

use super::helpers::{brave_freshness, build_search_client};
use super::{SearchParams, SearchResult};

pub(super) async fn search_brave(
    api_key: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Brave Search API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count
    );
    if let Some(ref country) = params.country {
        url.push_str(&format!("&country={}", urlencoding::encode(country)));
    }
    if let Some(ref lang) = params.language {
        url.push_str(&format!("&search_lang={}", urlencoding::encode(lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!("&freshness={}", brave_freshness(freshness)));
    }
    let resp = client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Brave Search request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Brave Search failed ({}): {}",
            status,
            body
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Brave Search JSON parse failed: {}", e))?;
    let web = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|v| v.as_array());
    Ok(web.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SearchResult {
                    title,
                    url,
                    snippet,
                    source: "Brave".into(),
                })
            })
            .collect()
    }))
}
