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

use crate::tools::ToolDefinition;

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
/// for server `cfg`. `always_load` is hoisted to the caller so the
/// "always_load_servers" global whitelist can be honored without
/// plumbing `McpGlobalSettings` through here.
pub fn rmcp_tool_to_definition(
    cfg: &McpServerConfig,
    tool: &model::Tool,
    always_load: bool,
) -> ToolDefinition {
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

    ToolDefinition {
        name,
        description: desc,
        parameters,
        internal: false,
        deferred: !always_load,
        always_load,
        async_capable: false,
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
        let def = rmcp_tool_to_definition(&cfg, &tool, false);
        assert_eq!(def.name, "mcp__example__my_tool");
        assert!(def.description.starts_with("[example] "));
        assert_eq!(def.parameters["type"], "object");
        assert!(def.deferred);
        assert!(!def.always_load);
    }

    #[test]
    fn tool_to_definition_always_load_flag_flips_deferred() {
        let tool = model::Tool::new("my_tool", "x", std::sync::Arc::new(serde_json::Map::new()));
        let cfg = min_cfg("example");
        let def = rmcp_tool_to_definition(&cfg, &tool, true);
        assert!(!def.deferred);
        assert!(def.always_load);
    }
}
