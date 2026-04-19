use anyhow::Result;

use super::helpers::{
    build_search_client, google_date_restrict, read_json_capped, read_text_capped,
    JSON_RESPONSE_BYTE_CAP,
};
use super::{SearchParams, SearchResult};

pub(super) async fn search_google(
    api_key: &str,
    cx: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() || cx.is_empty() {
        return Err(anyhow::anyhow!(
            "Google Custom Search API key or CX not configured"
        ));
    }
    let client = build_search_client(timeout_secs)?;
    let mut url = format!(
        "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}&num={}",
        urlencoding::encode(api_key),
        urlencoding::encode(cx),
        urlencoding::encode(query),
        count.min(10)
    );
    if let Some(ref country) = params.country {
        url.push_str(&format!("&gl={}", urlencoding::encode(country)));
    }
    if let Some(ref lang) = params.language {
        url.push_str(&format!("&lr=lang_{}", urlencoding::encode(lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!(
            "&dateRestrict={}",
            google_date_restrict(freshness)
        ));
    }
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Google Custom Search request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP)
            .await
            .unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Google Custom Search failed ({}): {}",
            status,
            text
        ));
    }
    let data = read_json_capped(resp, JSON_RESPONSE_BYTE_CAP, "Google Custom Search").await?;
    let items = data.get("items").and_then(|v| v.as_array());
    Ok(items.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("link")?.as_str()?.to_string();
                let snippet = item
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SearchResult {
                    title,
                    url,
                    snippet,
                    source: "Google".into(),
                })
            })
            .collect()
    }))
}
