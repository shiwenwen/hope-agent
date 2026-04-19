use anyhow::Result;

use super::helpers::{
    build_search_client, read_json_capped, read_text_capped, tavily_days, JSON_RESPONSE_BYTE_CAP,
};
use super::{SearchParams, SearchResult};

pub(super) async fn search_tavily(
    api_key: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Tavily API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut body = serde_json::json!({
        "api_key": api_key,
        "query": query,
        "max_results": count,
        "include_answer": false,
    });
    if let Some(ref country) = params.country {
        body["country"] = serde_json::Value::String(country.clone());
    }
    if let Some(ref freshness) = params.freshness {
        body["days"] = serde_json::Value::Number(serde_json::Number::from(tavily_days(freshness)));
    }
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Tavily request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP)
            .await
            .unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Tavily search failed ({}): {}",
            status,
            body
        ));
    }
    let data = read_json_capped(resp, JSON_RESPONSE_BYTE_CAP, "Tavily").await?;
    let results = data.get("results").and_then(|v| v.as_array());
    Ok(results.map_or_else(Vec::new, |arr| {
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
                Some(SearchResult {
                    title,
                    url,
                    snippet,
                    source: "Tavily".into(),
                })
            })
            .collect()
    }))
}
