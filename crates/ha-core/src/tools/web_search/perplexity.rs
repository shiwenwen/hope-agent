use anyhow::Result;

use super::helpers::{
    build_search_client, read_json_capped, read_text_capped, JSON_RESPONSE_BYTE_CAP,
};
use super::{record_llm_web_search_usage, SearchParams, SearchResult, WebSearchUsageContext};

const MODEL_ID: &str = "sonar";

pub(super) async fn search_perplexity(
    api_key: &str,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
    usage_ctx: &WebSearchUsageContext,
) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Perplexity API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut body = serde_json::json!({
        "model": MODEL_ID,
        "messages": [{"role": "user", "content": query}],
        "max_tokens": 1024,
        "return_citations": true
    });
    if let Some(ref freshness) = params.freshness {
        body["search_recency_filter"] = serde_json::Value::String(freshness.clone());
    }
    let started = std::time::Instant::now();
    let resp = match client
        .post("https://api.perplexity.ai/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            record_llm_web_search_usage(
                usage_ctx,
                "web_search.perplexity",
                "perplexity",
                "Perplexity",
                MODEL_ID,
                started.elapsed().as_millis() as u64,
                false,
                Some(format!("Perplexity request failed: {}", e)),
                None,
            );
            return Err(anyhow::anyhow!("Perplexity request failed: {}", e));
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = read_text_capped(resp, JSON_RESPONSE_BYTE_CAP)
            .await
            .unwrap_or_default();
        record_llm_web_search_usage(
            usage_ctx,
            "web_search.perplexity",
            "perplexity",
            "Perplexity",
            MODEL_ID,
            started.elapsed().as_millis() as u64,
            false,
            Some(format!("Perplexity failed ({}): {}", status, text)),
            None,
        );
        return Err(anyhow::anyhow!("Perplexity failed ({}): {}", status, text));
    }
    let data = read_json_capped(resp, JSON_RESPONSE_BYTE_CAP, "Perplexity").await?;
    record_llm_web_search_usage(
        usage_ctx,
        "web_search.perplexity",
        "perplexity",
        "Perplexity",
        MODEL_ID,
        started.elapsed().as_millis() as u64,
        true,
        None,
        Some(&data),
    );

    // Extract citations as search results
    let citations = data.get("citations").and_then(|v| v.as_array());
    let content = data
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut results: Vec<SearchResult> = citations.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|c| {
                let url = c.as_str()?.to_string();
                // Extract domain as title fallback
                let title = url.split('/').nth(2).unwrap_or(&url).to_string();
                Some(SearchResult {
                    title,
                    url,
                    snippet: String::new(),
                    source: "Perplexity".into(),
                })
            })
            .collect()
    });

    // If we got a summary but no citations, return the summary as a single result
    if results.is_empty() && !content.is_empty() {
        results.push(SearchResult {
            title: "Perplexity Summary".into(),
            url: String::new(),
            snippet: content.chars().take(500).collect(),
            source: "Perplexity".into(),
        });
    }

    Ok(results)
}
