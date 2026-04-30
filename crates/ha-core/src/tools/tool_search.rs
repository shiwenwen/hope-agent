use anyhow::Result;
use serde_json::Value;

use super::{
    definitions::ToolDefinition,
    dispatch::{all_dispatchable_tools, resolve_tool_fate, DispatchContext, ToolFate},
    ToolExecContext,
};

/// Handle the tool_search meta-tool: find tools by query and return their full schemas.
///
/// Supports two query forms:
/// - `"select:name1,name2"` — exact match by tool name
/// - `"keyword1 keyword2"` — fuzzy search by name/description relevance
///
/// Candidates pool: every tool whose dispatcher fate is `InjectEager` or
/// `InjectDeferred` for the current agent + global config. `Hidden` and
/// `HintOnly` tools are excluded — they're either disabled or
/// unprovisioned, so surfacing them via search would be misleading.
pub(crate) async fn tool_search(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(20) as usize;

    let app_config = crate::config::cached_config();
    // Load this session's AgentConfig to feed the dispatcher. tool_search is
    // a cold path (the model only calls it occasionally), so re-reading
    // agent.json is not a hot-loop concern.
    let agent_id = ctx.agent_id.as_deref().unwrap_or("default");
    let agent_def = crate::agent_loader::load_agent(agent_id).ok();
    let default_cfg = crate::agent_config::AgentConfig::default();
    let agent_cfg = agent_def
        .as_ref()
        .map(|d| &d.config)
        .unwrap_or(&default_cfg);

    let dispatch_ctx = DispatchContext {
        agent_id,
        mcp_enabled: agent_cfg.capabilities.mcp_enabled,
        memory_enabled: agent_cfg.memory.enabled,
        tools_filter: &agent_cfg.capabilities.tools,
        capability_toggles: &agent_cfg.capabilities.capability_toggles,
        app_config: &app_config,
    };

    // Single-pass over the static catalog: collect candidates and count
    // deferred ones in one walk so `resolve_tool_fate` runs once per tool.
    let mut candidates: Vec<ToolDefinition> = Vec::new();
    let mut total_deferred = 0usize;
    for t in all_dispatchable_tools() {
        match resolve_tool_fate(t, &dispatch_ctx) {
            ToolFate::InjectDeferred => {
                total_deferred += 1;
                candidates.push(t.clone());
            }
            ToolFate::InjectEager => candidates.push(t.clone()),
            _ => {}
        }
    }

    // Dynamic MCP tools (`mcp__<server>__<tool>`) — gated by agent.mcp_enabled.
    if agent_cfg.capabilities.mcp_enabled {
        if let Some(mcp) = crate::mcp::McpManager::global() {
            for def in mcp.mcp_tool_definitions().iter() {
                if !candidates.iter().any(|c| c.name == def.name) {
                    if crate::mcp::catalog::tool_belongs_to_deferred_server(
                        &def.name,
                        &app_config.mcp_servers,
                    ) {
                        total_deferred += 1;
                    }
                    candidates.push(def.clone());
                }
            }
        }
    }

    // Final exec-layer filter (skill / denied / plan-allowed) — defense in depth.
    candidates.retain(|t| ctx.is_tool_visible(&t.name));

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
