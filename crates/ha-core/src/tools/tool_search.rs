use anyhow::Result;
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::{
    definitions::ToolDefinition,
    dispatch::{all_dispatchable_tools, resolve_tool_fate, DispatchContext, ToolFate},
    ToolExecContext,
};

/// Handle the tool_search meta-tool: find tools and activate deferred matches.
///
/// Full parameter schemas are intentionally not copied into the tool result.
/// The orchestrator consumes the structured activation metadata and injects
/// those schemas into the next provider round, where they are actually
/// callable. This avoids paying for the same schema in both a tool result and
/// the provider `tools` array.
///
/// Supports two query forms:
/// - `"select:name1,name2"` — exact match by tool name or alias
/// - `"keyword1 keyword2"` — weighted search over name, aliases, hints,
///   description, parameter hints, effects, risk, and classifier tags.
///
/// Candidates pool: every tool whose dispatcher fate is `InjectEager` or
/// `InjectDeferred` for the current agent + global config. `Hidden` and
/// `HintOnly` tools are excluded — they're either disabled or
/// unprovisioned, so surfacing them via search would be misleading.
pub(crate) async fn tool_search(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(20) as usize;

    let app_config = crate::config::cached_config();
    // Load this session's AgentConfig to feed the dispatcher. tool_search is
    // a cold path (the model only calls it occasionally), so re-reading
    // agent.json is not a hot-loop concern.
    let agent_id = ctx
        .agent_id
        .as_deref()
        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);
    let agent_def = crate::agent_loader::load_agent(agent_id).ok();
    let default_cfg = crate::agent_config::AgentConfig::default();
    let agent_cfg = agent_def
        .as_ref()
        .map(|d| &d.config)
        .unwrap_or(&default_cfg);
    let session_access = crate::memory::effective_session_memory_access(
        ctx.session_id.as_deref(),
        ctx.session_db.as_ref().map(|handle| handle.0.as_ref()),
    );

    let dispatch_ctx = DispatchContext {
        agent_id,
        incognito: ctx.incognito,
        mcp_enabled: agent_cfg.capabilities.mcp_enabled,
        memory_enabled: agent_cfg.memory.enabled,
        use_memories: session_access.use_memories,
        contribute_to_memories: session_access.contribute_to_memories,
        tools_filter: &agent_cfg.capabilities.tools,
        app_config: &app_config,
    };

    // Single-pass over the static catalog: collect candidates and count
    // deferred ones in one walk so `resolve_tool_fate` runs once per tool.
    let mut candidates: Vec<ToolDefinition> = Vec::new();
    let mut deferred_names: HashSet<String> = HashSet::new();
    let mut total_deferred = 0usize;
    for t in all_dispatchable_tools() {
        match resolve_tool_fate(t, &dispatch_ctx) {
            ToolFate::InjectDeferred => {
                total_deferred += 1;
                deferred_names.insert(t.name.clone());
                candidates.push(t.clone());
            }
            ToolFate::InjectEager => candidates.push(t.clone()),
            _ => {}
        }
    }

    // Dynamic MCP tools (`mcp__<server>__<tool>`) — gated by agent.mcp_enabled
    // and the global MCP kill switch.
    if agent_cfg.capabilities.mcp_enabled && app_config.mcp_global.enabled {
        if let Some(mcp) = crate::mcp::McpManager::global() {
            for def in mcp.mcp_tool_definitions().iter() {
                if !candidates.iter().any(|c| c.name == def.name) {
                    if super::dispatch::should_defer_dynamic_mcp_tool(&def.name, &app_config) {
                        total_deferred += 1;
                        deferred_names.insert(def.name.clone());
                    }
                    candidates.push(def.clone());
                }
            }
        }
    }

    // Final exec-layer filter (skill / denied / plan-allowed) — defense in depth.
    candidates.retain(|t| ctx.is_tool_visible(&t.name));

    // KB-scoped tools (note_* / session_to_note) are useless without an attached
    // KB — hide them on a no-KB session, mirroring the eager-schema gate in
    // `Agent::build_tool_schemas` so they can't be resurrected here. The access
    // check is skipped unless such a tool actually survived the filters above.
    if candidates.iter().any(|t| super::is_kb_scoped_tool(&t.name))
        && !super::note::session_has_kb_access(ctx)
    {
        candidates.retain(|t| !super::is_kb_scoped_tool(&t.name));
    }

    // Project auto memory must follow the same live eligibility contract as
    // Agent::finalize_tool_schemas. Otherwise tool_search could claim it was
    // activated in a non-project session while the next-round inventory quite
    // correctly refuses to make it callable.
    let has_live_project = ctx.project_id.as_deref().is_some_and(|project_id| {
        if let Some(session_db) = ctx.session_db.as_ref() {
            crate::project::ProjectDB::new(session_db.0.clone())
                .get(project_id)
                .ok()
                .flatten()
                .is_some()
        } else {
            crate::get_project_db()
                .and_then(|db| db.get(project_id).ok().flatten())
                .is_some()
        }
    });
    if !has_live_project {
        candidates.retain(|tool| tool.name != super::TOOL_PROJECT_MEMORY);
        if deferred_names.remove(super::TOOL_PROJECT_MEMORY) {
            total_deferred = total_deferred.saturating_sub(1);
        }
    }

    // Select mode: "select:name1,name2" for exact matching by name or alias.
    // The prefix and selected names are case/space/hyphen insensitive.
    if let Some(names_str) = select_payload(query) {
        let raw_names: Vec<&str> = names_str.split(',').map(str::trim).collect();
        let names: Vec<String> = raw_names
            .iter()
            .copied()
            .map(normalize_selector)
            .filter(|s| !s.is_empty())
            .collect();
        let matched: Vec<&ToolDefinition> = candidates
            .iter()
            .filter(|t| {
                names.iter().any(|n| selector_matches(t, n))
                    || explicit_variant_selection(t, &raw_names).is_some()
            })
            .collect();

        let activated_tools: Vec<String> = matched
            .iter()
            .filter(|t| deferred_names.contains(&t.name))
            .map(|t| explicit_variant_selection(t, &raw_names).unwrap_or_else(|| t.name.clone()))
            .collect();
        ctx.emit_metadata(serde_json::json!({
            "kind": "tool_search_activation",
            "activatedToolNames": activated_tools,
        }))
        .await;

        let results: Vec<Value> = matched
            .iter()
            .map(|t| tool_to_summary(t, None, &app_config))
            .collect();

        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "mode": "select",
            "matched_tools": results.len(),
            "activated_tools": activated_tools,
            "total_deferred_tools": total_deferred,
            "total_candidates": candidates.len(),
            "truncated": false,
            "tools": results,
        }))?);
    }

    // Keyword search mode. Build a small weighted BM25 corpus each call so
    // tool_search stays data-driven: aliases/search_hints/effects added by
    // ToolDefinition v2 immediately participate in ranking.
    let query_terms = tokenize(query);
    let docs: Vec<SearchDoc<'_>> = candidates.iter().map(SearchDoc::new).collect();
    let idf = inverse_document_frequency(&docs);
    let avg_doc_len = average_doc_len(&docs);
    let normalized_query = normalize_text(query);

    let mut scored: Vec<(f64, &ToolDefinition)> = docs
        .iter()
        .map(|doc| {
            let mut score = 0.0;

            for term in &query_terms {
                if let Some(weight) = doc.term_weights.get(term) {
                    let idf = idf.get(term).copied().unwrap_or(1.0);
                    score += bm25_score(*weight, idf, doc.weighted_len, avg_doc_len);
                }
            }

            if !normalized_query.is_empty() {
                if normalize_selector(&doc.tool.name) == normalize_selector(query) {
                    score += 30.0;
                }
                if doc
                    .metadata
                    .aliases
                    .iter()
                    .any(|a| normalize_selector(a) == normalize_selector(query))
                {
                    score += 24.0;
                }
                if doc.phrase_text.contains(&normalized_query) {
                    score += 6.0;
                }
                if doc
                    .tool
                    .name
                    .to_ascii_lowercase()
                    .contains(&normalized_query)
                {
                    score += 10.0;
                }
            }

            (score, doc.tool)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let total_matches = scored.len();
    scored.truncate(max_results);

    // Fuzzy search auto-activates deferred matches, bounded by a schema budget
    // so one broad query cannot suddenly append several giant schemas. Exact
    // `select:` requests are user/model-explicit and are not budget-trimmed.
    const AUTO_ACTIVATION_SCHEMA_BUDGET: u32 = 6_000;
    let mut activated_tools = Vec::new();
    let mut activation_tokens = 0u32;
    for (_, tool) in &scored {
        if !deferred_names.contains(&tool.name) {
            continue;
        }
        let tool_tokens = crate::context_compact::estimate_tokens(&tool.parameters);
        let activation_name =
            best_call_variant_for_query(tool, query).unwrap_or_else(|| tool.name.clone());
        let tool_tokens = crate::tools::split_call_variant_name(&activation_name)
            .and_then(|(_, action)| {
                tool.to_compact_call_variant(action, super::ToolProvider::OpenAI)
            })
            .map(|schema| crate::context_compact::estimate_tokens(&schema))
            .unwrap_or(tool_tokens);
        if !activated_tools.is_empty()
            && activation_tokens.saturating_add(tool_tokens) > AUTO_ACTIVATION_SCHEMA_BUDGET
        {
            continue;
        }
        activation_tokens = activation_tokens.saturating_add(tool_tokens);
        activated_tools.push(activation_name);
    }
    ctx.emit_metadata(serde_json::json!({
        "kind": "tool_search_activation",
        "activatedToolNames": activated_tools,
    }))
    .await;

    let results: Vec<Value> = scored
        .iter()
        .map(|(score, t)| tool_to_summary(t, Some(*score), &app_config))
        .collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "query": query,
        "mode": "search",
        "matched_tools": results.len(),
        "activated_tools": activated_tools,
        "activation_schema_tokens_estimate": activation_tokens,
        "total_matches": total_matches,
        "total_deferred_tools": total_deferred,
        "total_candidates": candidates.len(),
        "truncated": total_matches > results.len(),
        "tools": results,
    }))?)
}

/// Convert a definition to compact discovery metadata. The full parameters
/// arrive through the next round's actual tool schema, not through text.
fn tool_to_summary(
    t: &ToolDefinition,
    score: Option<f64>,
    app_config: &crate::config::AppConfig,
) -> Value {
    let mut obj = t.to_api_metadata(app_config);
    if let Some(map) = obj.as_object_mut() {
        let variants = t
            .call_variant_actions()
            .iter()
            .map(|action| format!("{}__{}", t.name, action))
            .collect::<Vec<_>>();
        if !variants.is_empty() {
            map.insert("callVariants".to_string(), serde_json::json!(variants));
        }
        if let Some(score) = score {
            map.insert(
                "score".to_string(),
                serde_json::json!((score * 1000.0).round() / 1000.0),
            );
        }
    }
    obj
}

fn explicit_variant_selection(t: &ToolDefinition, raw_names: &[&str]) -> Option<String> {
    for action in t.call_variant_actions() {
        let candidate = format!("{}__{}", t.name, action);
        if raw_names
            .iter()
            .any(|raw| normalize_selector(raw) == normalize_selector(&candidate))
        {
            return Some(candidate);
        }
    }
    None
}

fn best_call_variant_for_query(t: &ToolDefinition, query: &str) -> Option<String> {
    let normalized = normalize_text(query);
    let words: HashSet<&str> = normalized.split_whitespace().collect();
    for action in t.call_variant_actions() {
        let normalized_action = normalize_text(action);
        if !normalized_action.is_empty() && normalized.contains(&normalized_action) {
            return Some(format!("{}__{}", t.name, action));
        }
    }
    let aliases: &[(&str, &[&str])] = match t.name.as_str() {
        crate::tools::TOOL_BROWSER => &[
            ("snapshot", &["snapshot", "screenshot", "pdf", "role tree"]),
            (
                "act",
                &[
                    "click", "fill", "hover", "drag", "select", "press", "upload",
                ],
            ),
            (
                "observe",
                &["console", "network", "page errors", "downloads"],
            ),
            ("navigate", &["navigate", "reload", "forward", "back"]),
            ("tabs", &["tab", "tabs"]),
            ("profile", &["profile", "chromium"]),
            (
                "control",
                &["evaluate", "cdp", "dialog", "resize", "scroll"],
            ),
            ("status", &["status"]),
        ],
        crate::tools::TOOL_MAC_CONTROL => &[
            ("visual", &["visual", "ocr", "find text", "screen point"]),
            ("elements", &["element", "elements", "accessibility"]),
            ("windows", &["window", "windows"]),
            ("apps", &["app", "apps", "launch app", "quit app"]),
            ("clipboard", &["clipboard", "pasteboard"]),
            ("dialog", &["dialog", "file picker"]),
            ("menu", &["menu", "menu bar"]),
            ("dock", &["dock"]),
            ("spaces", &["space", "spaces", "desktop"]),
            ("snapshot", &["snapshot", "screenshot"]),
            ("wait", &["wait"]),
            ("diagnostics", &["diagnostics", "diagnose"]),
            ("permissions", &["permissions", "permission"]),
            (
                "act",
                &["click", "type", "hotkey", "scroll", "drag", "swipe"],
            ),
            ("status", &["status"]),
        ],
        _ => return None,
    };
    for (action, hints) in aliases {
        if hints.iter().any(|hint| {
            let normalized_hint = normalize_text(hint);
            normalized.contains(&normalized_hint)
                || normalized_hint
                    .split_whitespace()
                    .all(|word| words.contains(word))
        }) {
            return Some(format!("{}__{}", t.name, action));
        }
    }
    None
}

struct SearchDoc<'a> {
    tool: &'a ToolDefinition,
    metadata: super::definitions::ToolMetadata,
    term_weights: HashMap<String, f64>,
    weighted_len: f64,
    phrase_text: String,
}

impl<'a> SearchDoc<'a> {
    fn new(tool: &'a ToolDefinition) -> Self {
        let metadata = tool.v2_metadata();
        let mut fields: Vec<(String, f64)> = vec![
            (tool.name.clone(), 9.0),
            (tool.name.replace('_', " "), 8.0),
            (tool.description.clone(), 1.2),
            (metadata.searchable_text(), 2.2),
        ];
        fields.extend(metadata.aliases.iter().cloned().map(|s| (s, 7.0)));
        fields.extend(metadata.search_hints.iter().cloned().map(|s| (s, 4.0)));
        fields.extend(metadata.classifier_tags.iter().cloned().map(|s| (s, 3.0)));

        if let Some(props) = tool
            .parameters
            .get("properties")
            .and_then(|v| v.as_object())
        {
            for (name, schema) in props {
                fields.push((name.clone(), 2.5));
                if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
                    fields.push((desc.to_string(), 0.8));
                }
            }
        }

        let mut term_weights = HashMap::new();
        let mut weighted_len = 0.0;
        let mut phrase_parts = Vec::new();
        for (field, weight) in fields {
            phrase_parts.push(normalize_text(&field));
            for token in tokenize(&field) {
                *term_weights.entry(token).or_insert(0.0) += weight;
                weighted_len += weight;
            }
        }

        Self {
            tool,
            metadata,
            term_weights,
            weighted_len,
            phrase_text: phrase_parts.join(" "),
        }
    }
}

fn average_doc_len(docs: &[SearchDoc<'_>]) -> f64 {
    if docs.is_empty() {
        return 1.0;
    }
    let total: f64 = docs.iter().map(|doc| doc.weighted_len).sum();
    (total / docs.len() as f64).max(1.0)
}

fn bm25_score(weighted_tf: f64, idf: f64, doc_len: f64, avg_doc_len: f64) -> f64 {
    let k1 = 1.2;
    let b = 0.75;
    let norm = k1 * (1.0 - b + b * (doc_len.max(1.0) / avg_doc_len.max(1.0)));
    idf * ((weighted_tf * (k1 + 1.0)) / (weighted_tf + norm))
}

fn inverse_document_frequency(docs: &[SearchDoc<'_>]) -> HashMap<String, f64> {
    let mut doc_freq: HashMap<String, usize> = HashMap::new();
    for doc in docs {
        let unique: HashSet<&String> = doc.term_weights.keys().collect();
        for term in unique {
            *doc_freq.entry(term.clone()).or_insert(0) += 1;
        }
    }
    let n = docs.len() as f64;
    doc_freq
        .into_iter()
        .map(|(term, df)| {
            let idf = ((n + 1.0) / (df as f64 + 1.0)).ln() + 1.0;
            (term, idf)
        })
        .collect()
}

fn select_payload(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    if trimmed.len() < "select:".len() {
        return None;
    }
    let (prefix, rest) = trimmed.split_at("select:".len());
    if prefix.eq_ignore_ascii_case("select:") {
        Some(rest)
    } else {
        None
    }
}

fn selector_matches(tool: &ToolDefinition, normalized: &str) -> bool {
    normalize_selector(&tool.name) == normalized
        || tool
            .v2_metadata()
            .aliases
            .iter()
            .any(|alias| normalize_selector(alias) == normalized)
}

fn normalize_selector(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn normalize_text(value: &str) -> String {
    tokenize(value).join(" ")
}

fn tokenize(value: &str) -> Vec<String> {
    value
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .flat_map(|part| part.split('_'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
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
        assert_eq!(parsed["mode"].as_str().unwrap(), "select");
    }

    #[tokio::test]
    async fn select_emits_activation_metadata_without_duplicating_schema() {
        let sink = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let ctx = ToolExecContext {
            metadata_sink: Some(sink.clone()),
            ..ToolExecContext::default()
        };
        let result = tool_search(&json!({ "query": "select:browser" }), &ctx)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let first = &parsed["tools"].as_array().unwrap()[0];
        assert!(first.get("parameters").is_none());

        let metadata = sink.lock().await.clone().unwrap();
        assert_eq!(metadata["kind"], "tool_search_activation");
        assert!(metadata["activatedToolNames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "browser"));
    }

    #[tokio::test]
    async fn workflow_is_not_discoverable_via_tool_search() {
        let args = json!({ "query": "select:workflow" });
        let result = tool_search(&args, &ToolExecContext::default())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["matched_tools"].as_u64().unwrap(), 0);
        assert!(parsed["tools"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn project_memory_is_not_discoverable_without_a_live_project() {
        let result = tool_search(
            &json!({ "query": "select:project_memory" }),
            &ToolExecContext::default(),
        )
        .await
        .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["matched_tools"].as_u64().unwrap(), 0);
        assert!(parsed["tools"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn project_memory_is_discoverable_with_a_bound_live_project() {
        let dir = tempfile::tempdir().unwrap();
        let session_db = std::sync::Arc::new(
            crate::session::SessionDB::open(&dir.path().join("sessions.db")).unwrap(),
        );
        let project_db = crate::project::ProjectDB::new(session_db.clone());
        project_db.migrate().unwrap();
        let project = project_db
            .create(crate::project::CreateProjectInput {
                name: "Memory test".into(),
                description: None,
                logo: None,
                color: None,
                default_agent_id: None,
                default_model_id: None,
                working_dir: None,
            })
            .unwrap();
        let ctx = ToolExecContext {
            project_id: Some(project.id),
            session_db: Some(crate::tools::execution::SessionDbHandle(session_db)),
            ..ToolExecContext::default()
        };

        let result = tool_search(&json!({ "query": "select:project_memory" }), &ctx)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["matched_tools"].as_u64().unwrap(), 1);
        assert_eq!(parsed["tools"][0]["name"], "project_memory");
    }

    #[tokio::test]
    async fn exact_call_variant_activation_stays_compact_and_structured() {
        let sink = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let ctx = ToolExecContext {
            metadata_sink: Some(sink.clone()),
            ..ToolExecContext::default()
        };
        let result = tool_search(&json!({ "query": "select:browser__snapshot" }), &ctx)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let tool = &parsed["tools"][0];
        assert_eq!(tool["name"], "browser");
        assert!(tool["callVariants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "browser__snapshot"));
        let metadata = sink.lock().await.clone().unwrap();
        assert!(metadata["activatedToolNames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "browser__snapshot"));
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
        assert_eq!(parsed["mode"].as_str().unwrap(), "search");
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
    async fn test_context_denied_tools_are_hidden() {
        let args = json!({ "query": "select:read,write" });
        let ctx = ToolExecContext {
            denied_tools: vec!["write".to_string()],
            ..ToolExecContext::default()
        };

        let result = tool_search(&args, &ctx).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let tools = parsed["tools"].as_array().unwrap();

        assert_eq!(parsed["matched_tools"].as_u64().unwrap(), 1);
        assert_eq!(tools[0]["name"].as_str().unwrap(), "read");
    }

    #[tokio::test]
    async fn test_select_query_is_case_space_and_alias_tolerant() {
        let args = json!({ "query": "SELECT: Read, modify file" });
        let result = tool_search(&args, &ToolExecContext::default())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let names: Vec<&str> = parsed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"edit"));
    }

    #[tokio::test]
    async fn test_keyword_query_uses_v2_metadata() {
        let args = json!({ "query": "shell terminal command", "max_results": 1 });
        let result = tool_search(&args, &ToolExecContext::default())
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let first = &parsed["tools"].as_array().unwrap()[0];
        assert_eq!(first["name"].as_str().unwrap(), "exec");
        assert_eq!(first["metadata"]["risk"].as_str().unwrap(), "strict");
        assert!(first["metadata"]["effects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("execute_process")));
        assert!(first["score"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn incognito_hides_memory_tools_from_deferred_discovery() {
        let args = json!({
            "query": "select:save_memory,recall_memory,memory_get,update_memory,delete_memory,update_core_memory",
            "max_results": 20
        });
        let ctx = ToolExecContext {
            incognito: true,
            ..ToolExecContext::default()
        };

        let result = tool_search(&args, &ctx).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert_eq!(
            parsed["matched_tools"].as_u64().unwrap(),
            0,
            "incognito tool_search must not resurrect Memory tier tools"
        );
        assert!(parsed["tools"].as_array().unwrap().is_empty());
    }
}
