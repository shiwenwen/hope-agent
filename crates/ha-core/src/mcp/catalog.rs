//! MCP ↔ hope-agent tool catalog bridge.
//!
//! Responsibilities:
//! * Translate `rmcp::model::Tool` → the in-tree `ToolDefinition` shape.
//! * Apply the namespace scheme `mcp__<server_name>__<tool_name>` with
//!   sanitization + collision-safe truncation so the resulting identifier
//!   fits the 64-char limits imposed by Anthropic / OpenAI tool schemas.
//! * Flatten union `anyOf` / `oneOf` at the top level of `inputSchema`
//!   because some providers reject those at the root (we preserve them
//!   in nested positions).

use rmcp::model;
use serde_json::{json, Value};

use crate::tools::{ToolDefinition, ToolTier};

use super::config::McpServerConfig;

/// Max length for a *tool* name after the `mcp__<server>__` prefix.
/// The overall namespace fits: `"mcp__" + <=32 server + "__" + this` =
/// 5 + 32 + 2 + 25 = 64 chars, at the Anthropic / OpenAI ceiling.
const TOOL_NAME_CAP: usize = 25;

/// Sanitize an MCP tool name for use in the namespaced identifier:
/// * replace every non `[A-Za-z0-9_]` with `_`
/// * clamp to [`TOOL_NAME_CAP`] bytes
/// * guarantee at least one character (empty input falls back to `tool`)
pub fn sanitize_tool_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(TOOL_NAME_CAP));
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
        if out.len() >= TOOL_NAME_CAP {
            break;
        }
    }
    if out.is_empty() {
        "tool".to_string()
    } else {
        out
    }
}

/// Join the namespaced tool identifier the LLM sees.
pub fn namespaced_tool_name(server_name: &str, original_tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        server_name,
        sanitize_tool_name(original_tool_name)
    )
}

/// The `prefix_bytes` / `suffix_bytes` constants let callers decide
/// whether a name has our MCP prefix without ad-hoc string matching.
pub const MCP_TOOL_PREFIX: &str = "mcp__";

/// True iff the name is owned by the MCP subsystem. Exported for the
/// dispatch fallback branch in `tools::execution`.
pub fn is_mcp_tool_name(name: &str) -> bool {
    name.starts_with(MCP_TOOL_PREFIX)
}

/// Parse a namespaced MCP tool name of the form
/// `mcp__<server>__<tool>` into its two halves. Returns `None` when
/// the name isn't MCP-shaped or lacks the double-underscore separator
/// between the server and tool parts.
pub fn split_mcp_tool_name(name: &str) -> Option<(&str, &str)> {
    name.strip_prefix(MCP_TOOL_PREFIX)?.split_once("__")
}

/// True when the namespaced MCP tool belongs to a server whose tools should
/// be discoverable via `tool_search` instead of eagerly injected.
pub fn tool_belongs_to_deferred_server(name: &str, servers: &[McpServerConfig]) -> bool {
    let Some((server, _tool)) = split_mcp_tool_name(name) else {
        return false;
    };
    servers
        .iter()
        .any(|cfg| cfg.enabled && cfg.name == server && cfg.deferred_tools)
}

/// Whether any configured MCP server has opted its tools into deferred loading.
pub fn has_deferred_tool_server(servers: &[McpServerConfig]) -> bool {
    servers
        .iter()
        .any(|cfg| cfg.enabled && cfg.deferred_tools)
}

/// Build a short "MCP Capabilities" system-prompt section when any
/// MCP server has landed a populated tool catalog (i.e. a server that
/// completed at least one `tools/list` round is connected). Reads
/// purely from [`crate::mcp::McpManager::mcp_tool_definitions`] —
/// sync, `ArcSwap`-backed — so it can be called from the sync
/// `build_full_system_prompt` path without awaiting any lock.
///
/// The snippet intentionally does not enumerate every resource / prompt
/// — that list can be large and requires an async read of the per-
/// server state. The agent discovers those via the `mcp_resource`
/// and `mcp_prompt` tools we point at here.
pub fn system_prompt_snippet() -> Option<String> {
    let mgr = crate::mcp::McpManager::global()?;
    let defs = mgr.mcp_tool_definitions();
    if defs.is_empty() {
        return None;
    }
    let mut servers: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for d in defs.iter() {
        if let Some((server, _tool)) = split_mcp_tool_name(&d.name) {
            servers.insert(server.to_string());
        }
    }
    if servers.is_empty() {
        return None;
    }
    let list = servers.into_iter().collect::<Vec<_>>().join(", ");
    Some(format!(
        "# MCP Capabilities\n\n\
         Connected MCP servers: {list}\n\
         - Tools exposed by each server appear in the tool catalog with the `mcp__<server>__<tool>` naming.\n\
         - `mcp_resource(server=..., action=\"list\"|\"read\")` — inspect files / records / documents the server hosts.\n\
         - `mcp_prompt(server=..., action=\"list\"|\"get\")` — use prompt templates the server publishes."
    ))
}

// ── Schema conversion ────────────────────────────────────────────

/// Best-effort sanitation of the inputSchema the server advertises.
///
/// MCP tools are supposed to publish a JSON Schema object at
/// `inputSchema`, but the wild population has enough shapes that we
/// need to be defensive:
/// * `null` / empty → synthesize `{ "type":"object", "properties":{} }`
/// * already an object without `type` → inject `type:"object"`
/// * top-level `anyOf` / `oneOf` of object variants → merge their
///   `properties` (intersection of `required`) — lets Claude /
///   OpenAI accept the schema without a root-level union.
///
/// Nested unions are preserved as-is.
pub fn normalize_input_schema(raw: Value) -> Value {
    let mut obj = match raw {
        Value::Object(m) => m,
        _ => {
            return json!({ "type": "object", "properties": {} });
        }
    };

    // Top-level union → flatten.
    if obj.get("type").is_none() {
        if let Some(union) = obj
            .remove("anyOf")
            .or_else(|| obj.remove("oneOf"))
            .and_then(|v| match v {
                Value::Array(a) => Some(a),
                _ => None,
            })
        {
            let (props, required) = merge_object_union(&union);
            obj.insert("type".into(), json!("object"));
            obj.insert("properties".into(), Value::Object(props));
            if !required.is_empty() {
                obj.insert(
                    "required".into(),
                    Value::Array(required.into_iter().map(Value::String).collect()),
                );
            }
        } else {
            obj.insert("type".into(), json!("object"));
        }
    }

    // Ensure properties exists — some servers return `{"type":"object"}`
    // alone and Anthropic rejects missing `properties` on a root object.
    obj.entry("properties".to_string())
        .or_insert_with(|| json!({}));

    Value::Object(obj)
}

fn merge_object_union(variants: &[Value]) -> (serde_json::Map<String, Value>, Vec<String>) {
    use std::collections::BTreeSet;
    let mut merged_props = serde_json::Map::<String, Value>::new();
    let mut intersection: Option<BTreeSet<String>> = None;
    for v in variants {
        let Some(obj) = v.as_object() else { continue };
        if let Some(p) = obj.get("properties").and_then(|x| x.as_object()) {
            for (k, v) in p {
                merged_props.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        let req: BTreeSet<String> = obj
            .get("required")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(|x| x.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        intersection = Some(match intersection {
            None => req,
            Some(cur) => cur.intersection(&req).cloned().collect(),
        });
    }
    (
        merged_props,
        intersection.unwrap_or_default().into_iter().collect(),
    )
}

// ── ToolDefinition conversion ────────────────────────────────────

/// Build a [`ToolDefinition`] from an rmcp `Tool` under the naming rules
/// for server `cfg`. All MCP-derived tools are uniformly `Tier::Mcp`; the
/// per-agent `capabilities.mcp_enabled` flag gates injection.
pub fn rmcp_tool_to_definition(cfg: &McpServerConfig, tool: &model::Tool) -> ToolDefinition {
    let orig = tool.name.to_string();
    let name = namespaced_tool_name(&cfg.name, &orig);
    let description_owned: String = tool
        .description
        .as_ref()
        .map(|d| d.to_string())
        .unwrap_or_default();
    let desc = if description_owned.trim().is_empty() {
        format!("MCP tool from server '{}'", cfg.name)
    } else {
        format!("[{}] {}", cfg.name, description_owned)
    };

    // rmcp serializes `input_schema` as an `Arc<serde_json::Map>` —
    // convert to a plain Value so we can normalize in place.
    let raw_schema = Value::Object((*tool.input_schema).clone());
    let parameters = normalize_input_schema(raw_schema);

    // MCP spec 2025-11-25 introduced per-tool `execution.taskSupport`:
    // `required` → server mandates task-mode invocation (long-running),
    // `optional` → client chooses, `forbidden` (default) → sync only.
    // We map both `required` and `optional` onto ha-agent's
    // `async_capable=true`, which lets the existing "sync budget
    // timeout → auto-background" logic in the tool loop kick in when
    // the call takes too long.
    let async_capable = matches!(
        tool.execution
            .as_ref()
            .and_then(|e| e.task_support.as_ref()),
        Some(rmcp::model::TaskSupport::Required | rmcp::model::TaskSupport::Optional)
    );

    ToolDefinition {
        name,
        description: desc,
        parameters,
        tier: ToolTier::Mcp,
        internal: false,
        concurrent_safe: false,
        async_capable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::config::{McpServerConfig, McpTransportSpec, McpTrustLevel};

    fn min_cfg(name: &str) -> McpServerConfig {
        McpServerConfig {
            id: "id-1".into(),
            name: name.into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        }
    }

    #[test]
    fn sanitize_strips_and_truncates() {
        assert_eq!(sanitize_tool_name("foo-bar.baz"), "foo_bar_baz");
        assert_eq!(sanitize_tool_name(""), "tool");
        let long = "a".repeat(100);
        assert_eq!(sanitize_tool_name(&long).len(), TOOL_NAME_CAP);
    }

    #[test]
    fn namespace_fits_in_anthropic_openai_limit() {
        let max_server = "s".repeat(32);
        let max_tool = "x".repeat(100);
        let n = namespaced_tool_name(&max_server, &max_tool);
        assert!(
            n.len() <= 64,
            "namespaced name too long: {} ({} chars)",
            n,
            n.len()
        );
    }

    #[test]
    fn is_mcp_tool_name_matches_prefix() {
        assert!(is_mcp_tool_name("mcp__srv__foo"));
        assert!(!is_mcp_tool_name("read"));
        assert!(!is_mcp_tool_name("mcpsomething"));
    }

    #[test]
    fn normalize_missing_type_defaults_object() {
        let raw = json!({"properties": { "x": {"type": "string"} }});
        let norm = normalize_input_schema(raw);
        assert_eq!(norm["type"], "object");
    }

    #[test]
    fn normalize_empty_schema_synthesizes_object() {
        let n = normalize_input_schema(Value::Null);
        assert_eq!(n["type"], "object");
        assert!(n["properties"].is_object());
    }

    #[test]
    fn normalize_flattens_top_level_any_of() {
        // Two object variants; `a` is required in both → should land in
        // the merged `required`. `b` only in the first → dropped.
        let raw = json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": { "a": {"type": "string"}, "b": {"type": "string"} },
                    "required": ["a", "b"],
                },
                {
                    "type": "object",
                    "properties": { "a": {"type": "string"} },
                    "required": ["a"],
                },
            ]
        });
        let n = normalize_input_schema(raw);
        assert_eq!(n["type"], "object");
        assert!(n["properties"]["a"].is_object());
        assert!(n["properties"]["b"].is_object());
        let required: Vec<&str> = n["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(required, vec!["a"]);
    }

    #[test]
    fn tool_to_definition_embeds_server_in_description() {
        let mut tool = model::Tool::new(
            "my_tool",
            "original description",
            std::sync::Arc::new(serde_json::Map::new()),
        );
        tool.title = None;
        let cfg = min_cfg("example");
        let def = rmcp_tool_to_definition(&cfg, &tool);
        assert_eq!(def.name, "mcp__example__my_tool");
        assert!(def.description.starts_with("[example] "));
        assert_eq!(def.parameters["type"], "object");
        // All MCP-derived tools are uniformly Tier::Mcp; gating happens at
        // agent.capabilities.mcp_enabled time.
        assert!(matches!(def.tier, ToolTier::Mcp));
        assert!(def.is_always_load());
    }

    #[test]
    fn async_capable_tracks_task_support() {
        let cfg = min_cfg("srv");
        let schema = std::sync::Arc::new(serde_json::Map::new());

        // Default (no execution block) → sync-only.
        let default_tool = model::Tool::new("fast", "x", schema.clone());
        assert!(!rmcp_tool_to_definition(&cfg, &default_tool).async_capable);

        // `required` or `optional` → async_capable=true so the tool
        // loop's "sync budget → auto-background" branch can engage.
        let mut required_tool = model::Tool::new("long_required", "x", schema.clone());
        required_tool.execution = Some(model::ToolExecution::from_raw(Some(
            model::TaskSupport::Required,
        )));
        assert!(rmcp_tool_to_definition(&cfg, &required_tool).async_capable);

        let mut optional_tool = model::Tool::new("long_optional", "x", schema.clone());
        optional_tool.execution = Some(model::ToolExecution::from_raw(Some(
            model::TaskSupport::Optional,
        )));
        assert!(rmcp_tool_to_definition(&cfg, &optional_tool).async_capable);

        // Explicit `forbidden` → sync-only (same as default).
        let mut forbidden_tool = model::Tool::new("short", "x", schema);
        forbidden_tool.execution = Some(model::ToolExecution::from_raw(Some(
            model::TaskSupport::Forbidden,
        )));
        assert!(!rmcp_tool_to_definition(&cfg, &forbidden_tool).async_capable);
    }
}
