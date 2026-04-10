use anyhow::Result;
use serde_json::Value;

use super::{
    definitions::{get_available_tools, ToolDefinition},
    ToolExecContext,
};

/// Handle the tool_search meta-tool: find tools by query and return their full schemas.
///
/// Supports two query forms:
/// - `"select:name1,name2"` — exact match by tool name
/// - `"keyword1 keyword2"` — fuzzy search by name/description relevance
pub(crate) async fn tool_search(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(20) as usize;

    // Collect all tools (including conditionally-injected ones)
    let mut candidates = get_available_tools();
    // Also include conditionally-injected tools that may not be in get_available_tools
    let extra_tools = collect_extra_tools();
    for t in extra_tools {
        if !candidates.iter().any(|c| c.name == t.name) {
            candidates.push(t);
        }
    }

    candidates.retain(|t| ctx.is_tool_visible(&t.name));

    let total_deferred = candidates.iter().filter(|t| t.deferred).count();

    // Select mode: "select:name1,name2" for exact matching
    if let Some(names_str) = query.strip_prefix("select:") {
        let names: Vec<&str> = names_str.split(',').map(|s| s.trim()).collect();
        let matched: Vec<&ToolDefinition> = candidates
            .iter()
            .filter(|t| names.iter().any(|n| n.eq_ignore_ascii_case(&t.name)))
            .collect();

        let results: Vec<Value> = matched.iter().map(|t| tool_to_schema(t)).collect();

        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "matched_tools": results.len(),
            "total_deferred_tools": total_deferred,
            "tools": results,
        }))?);
    }

    // Keyword search mode
    let query_lower = query.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(f64, &ToolDefinition)> = candidates
        .iter()
        .map(|t| {
            let name_lower = t.name.to_lowercase();
            let desc_lower = t.description.to_lowercase();
            let mut score = 0.0;

            // Exact name match
            if name_lower == query_lower {
                score += 10.0;
            }
            // Name contains full query
            if name_lower.contains(&query_lower) {
                score += 5.0;
            }
            // Per-keyword scoring
            for kw in &keywords {
                if name_lower.contains(kw) {
                    score += 2.0;
                }
                if desc_lower.contains(kw) {
                    score += 1.0;
                }
            }
            (score, t)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_results);

    let results: Vec<Value> = scored.iter().map(|(_, t)| tool_to_schema(t)).collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "matched_tools": results.len(),
        "total_deferred_tools": total_deferred,
        "tools": results,
    }))?)
}

/// Convert a ToolDefinition to its full schema for the response.
fn tool_to_schema(t: &ToolDefinition) -> Value {
    serde_json::json!({
        "name": t.name,
        "description": t.description,
        "parameters": t.parameters,
    })
}

/// Collect conditionally-injected tools that may not appear in get_available_tools().
fn collect_extra_tools() -> Vec<ToolDefinition> {
    let mut extras = Vec::new();
    extras.push(super::definitions::get_subagent_tool());
    extras.push(super::definitions::get_tool_search_tool());
    if let Some(config) = load_image_gen_config() {
        extras.push(super::definitions::get_image_generate_tool_dynamic(&config));
    }
    extras.push(super::definitions::get_web_search_tool());
    extras.push(super::definitions::get_notification_tool());
    extras.push(super::definitions::get_canvas_tool());
    extras.push(super::definitions::get_acp_spawn_tool());
    extras
}

/// Try to load image generation config from app config.
fn load_image_gen_config() -> Option<crate::tools::image_generate::ImageGenConfig> {
    Some(crate::config::cached_config().image_generate.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_select_query() {
        let args = json!({ "query": "select:read,write" });
        let result = tool_search(&args, &ToolExecContext::default())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["matched_tools"].as_u64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn test_keyword_query() {
        let args = json!({ "query": "memory", "max_results": 3 });
        let result = tool_search(&args, &ToolExecContext::default())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["matched_tools"].as_u64().unwrap() > 0);
        assert!(parsed["matched_tools"].as_u64().unwrap() <= 3);
    }

    #[tokio::test]
    async fn test_empty_query() {
        let args = json!({ "query": "xyznonexistent" });
        let result = tool_search(&args, &ToolExecContext::default())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["matched_tools"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_agent_filter_hides_denied_tools() {
        let args = json!({ "query": "select:read,write" });
        let mut ctx = ToolExecContext::default();
        ctx.agent_tool_filter.deny = vec!["write".to_string()];

        let result = tool_search(&args, &ctx).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let tools = parsed["tools"].as_array().unwrap();

        assert_eq!(parsed["matched_tools"].as_u64().unwrap(), 1);
        assert_eq!(tools[0]["name"].as_str().unwrap(), "read");
    }
}
