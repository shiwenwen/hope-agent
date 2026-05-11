use anyhow::Result;
use serde_json::Value;

use super::helpers::{
    build_search_client, check_search_url, read_json_capped, read_text_capped,
    JSON_RESPONSE_BYTE_CAP,
};
use super::{SearchParams, SearchResult};

const BOCHA_WEB_SEARCH_ENDPOINT: &str = "https://api.bochaai.com/v1/web-search";

pub(super) async fn search_bocha(
    api_key: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Bocha AI Search API key not configured"));
    }

    check_search_url(BOCHA_WEB_SEARCH_ENDPOINT).await?;
    let client = build_search_client(timeout_secs)?;
    let body = serde_json::json!({
        "query": query,
        "freshness": params
            .freshness
            .as_deref()
            .map(bocha_freshness)
            .unwrap_or("noLimit"),
        "summary": true,
        "count": count,
    });

    let resp = client
        .post(BOCHA_WEB_SEARCH_ENDPOINT)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Bocha AI Search request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP)
            .await
            .unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Bocha AI Search failed ({}): {}",
            status,
            text
        ));
    }

    let data = read_json_capped(resp, JSON_RESPONSE_BYTE_CAP, "Bocha AI Search").await?;
    let results = data
        .get("webPages")
        .or_else(|| data.get("data").and_then(|d| d.get("webPages")))
        .and_then(|w| w.get("value"))
        .and_then(|v| v.as_array());

    Ok(results.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(parse_bocha_result)
            .collect()
    }))
}

fn parse_bocha_result(item: &Value) -> Option<SearchResult> {
    let title = item.get("name")?.as_str()?.to_string();
    let url = item.get("url")?.as_str()?.to_string();
    let snippet = item
        .get("summary")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| item.get("snippet").and_then(|v| v.as_str()))
        .unwrap_or("")
        .trim();
    let source = item
        .get("siteName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|site| format!("Bocha: {}", site.trim()))
        .unwrap_or_else(|| "Bocha".into());

    Some(SearchResult {
        title,
        url,
        snippet: crate::truncate_utf8(snippet, 900).to_string(),
        source,
    })
}

fn bocha_freshness(f: &str) -> &str {
    match f {
        "day" => "oneDay",
        "week" => "oneWeek",
        "month" => "oneMonth",
        "year" => "oneYear",
        _ => "noLimit",
    }
}
