use anyhow::Result;
use serde_json::Value;

use super::helpers::build_search_client;
use super::SearchResult;

pub(super) async fn search_grok(api_key: &str, query: &str, count: usize, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Grok (X.AI) API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let body = serde_json::json!({
        "model": "grok-3-mini-fast",
        "messages": [{"role": "user", "content": format!(
            "Search the web for: {}. Return exactly {} results as JSON array with fields: title, url, snippet. Only return the JSON array, no other text.",
            query, count
        )}],
        "search_parameters": {"mode": "auto"}
    });
    let resp = client
        .post("https://api.x.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Grok request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Grok failed ({}): {}", status, text));
    }
    let data: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Grok JSON parse failed: {}", e))?;

    // Extract search results from response
    let mut results = Vec::new();

    // Try to parse citations/search_results from the response
    if let Some(search_results) = data.get("search_results").and_then(|v| v.as_array()) {
        for item in search_results.iter().take(count) {
            if let (Some(title), Some(url)) = (
                item.get("title").and_then(|v| v.as_str()),
                item.get("url").and_then(|v| v.as_str()),
            ) {
                results.push(SearchResult {
                    title: title.to_string(),
                    url: url.to_string(),
                    snippet: item.get("snippet").or(item.get("description"))
                        .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                });
            }
        }
    }

    // Fallback: parse model content as JSON array
    if results.is_empty() {
        if let Some(content) = data.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
        {
            // Try to find JSON array in the content
            if let Some(start) = content.find('[') {
                if let Some(end) = content.rfind(']') {
                    if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&content[start..=end]) {
                        for item in arr.iter().take(count) {
                            if let (Some(title), Some(url)) = (
                                item.get("title").and_then(|v| v.as_str()),
                                item.get("url").and_then(|v| v.as_str()),
                            ) {
                                results.push(SearchResult {
                                    title: title.to_string(),
                                    url: url.to_string(),
                                    snippet: item.get("snippet")
                                        .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                });
                            }
                        }
                    }
                }
            }
            // If still empty, return content as a single result
            if results.is_empty() && !content.is_empty() {
                results.push(SearchResult {
                    title: "Grok Summary".into(),
                    url: String::new(),
                    snippet: content.chars().take(500).collect(),
                });
            }
        }
    }

    Ok(results)
}
