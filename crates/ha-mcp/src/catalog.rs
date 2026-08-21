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

use std::collections::{HashMap, HashSet};

use rmcp::model;
use serde_json::{json, Value};

use ha_core::tools::{ToolDefinition, ToolTier};

use super::config::McpServerConfig;

/// Provider ceiling for the complete `mcp__<server>__<tool>` identifier.
const NAMESPACED_TOOL_NAME_CAP: usize = 64;
/// Compatibility-safe component cap when no server name is available. A
/// validated MCP server name is at most 32 bytes, leaving 25 bytes after the
/// namespace separators.
const MIN_TOOL_NAME_CAP: usize = 25;

fn tool_name_cap(server_name: &str) -> usize {
    let server_component = ha_core::mcp::catalog::server_namespace_component(server_name);
    NAMESPACED_TOOL_NAME_CAP
        .saturating_sub("mcp__".len() + server_component.len() + "__".len())
        .max(1)
}

fn sanitize_tool_name_with_cap(raw: &str, cap: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(cap));
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
        if out.len() >= cap {
            break;
        }
    }
    if out.is_empty() {
        "tool".to_string()
    } else {
        out
    }
}

/// Sanitize an MCP tool name for use in the namespaced identifier:
/// * replace every non `[A-Za-z0-9_]` with `_`
/// * clamp to the worst-case component budget for a 32-byte server name
/// * guarantee at least one character (empty input falls back to `tool`)
///
/// Namespace construction uses the actual server name to reclaim otherwise
/// unused bytes. This standalone helper retains its historical worst-case cap.
pub fn sanitize_tool_name(raw: &str) -> String {
    sanitize_tool_name_with_cap(raw, MIN_TOOL_NAME_CAP)
}

/// Join the namespaced tool identifier the LLM sees.
pub fn namespaced_tool_name(server_name: &str, original_tool_name: &str) -> String {
    let tool_name = sanitize_tool_name_with_cap(original_tool_name, tool_name_cap(server_name));
    namespaced_tool_name_from_sanitized(server_name, &tool_name)
}

fn namespaced_tool_name_from_sanitized(server_name: &str, sanitized_tool_name: &str) -> String {
    let server_component = ha_core::mcp::catalog::server_namespace_component(server_name);
    format!("mcp__{server_component}__{sanitized_tool_name}")
}

fn legacy_namespaced_tool_name_from_sanitized(
    server_name: &str,
    sanitized_tool_name: &str,
) -> String {
    format!("mcp__{server_name}__{sanitized_tool_name}")
}

/// Assign collision-safe namespaced tool identifiers for a server catalog.
///
/// Sanitization alone can collapse distinct MCP tool names (`foo-bar` and
/// `foo.bar` both become `foo_bar`). The LLM-visible names must remain unique,
/// so later collisions get `_2`, `_3`, ... suffixes while staying inside the
/// provider 64-character tool-name ceiling.
pub fn assign_namespaced_tool_names<'a, I>(server_name: &str, originals: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let originals: Vec<&str> = originals.into_iter().collect();
    let legacy_names = assign_namespaced_tool_names_with_cap(
        server_name,
        originals.iter().copied(),
        MIN_TOOL_NAME_CAP,
        None,
        true,
    );
    let legacy_owner_by_name: HashMap<String, usize> = legacy_names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, index))
        .collect();
    assign_namespaced_tool_names_with_cap(
        server_name,
        originals,
        tool_name_cap(server_name),
        Some(&legacy_owner_by_name),
        false,
    )
}

/// Rebuild the names emitted before Hope reclaimed unused namespace bytes or
/// escaped ambiguous server delimiters. These identifiers are compatibility
/// aliases only and must not be exposed as duplicate provider schemas.
pub(crate) fn assign_legacy_namespaced_tool_names<'a, I>(
    server_name: &str,
    originals: I,
) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    assign_namespaced_tool_names_with_cap(server_name, originals, MIN_TOOL_NAME_CAP, None, true)
}

fn assign_namespaced_tool_names_with_cap<'a, I>(
    server_name: &str,
    originals: I,
    component_cap: usize,
    reserved_owner_by_name: Option<&HashMap<String, usize>>,
    legacy_raw_namespace: bool,
) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut next_ordinal_by_base: HashMap<String, usize> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    for (index, original) in originals.into_iter().enumerate() {
        let base = sanitize_tool_name_with_cap(original, component_cap);
        let mut ordinal = next_ordinal_by_base
            .get(&base)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        loop {
            let tool_part = suffixed_tool_name(&base, ordinal, component_cap);
            let namespaced = if legacy_raw_namespace {
                legacy_namespaced_tool_name_from_sanitized(server_name, &tool_part)
            } else {
                namespaced_tool_name_from_sanitized(server_name, &tool_part)
            };
            let reserved_by_other = reserved_owner_by_name.is_some_and(|owners| {
                owners
                    .get(&namespaced)
                    .is_some_and(|owner_index| *owner_index != index)
            });
            if !reserved_by_other && used.insert(namespaced.clone()) {
                next_ordinal_by_base.insert(base.clone(), ordinal);
                out.push(namespaced);
                break;
            }
            ordinal = ordinal.saturating_add(1);
        }
    }

    out
}

fn suffixed_tool_name(base: &str, ordinal: usize, cap: usize) -> String {
    if ordinal <= 1 {
        return base.to_string();
    }
    let suffix = format!("_{ordinal}");
    let keep = cap.saturating_sub(suffix.len());
    let mut out = String::with_capacity(cap);
    out.push_str(&base[..base.len().min(keep)]);
    out.push_str(&suffix);
    out
}

// 命名约定（`mcp__` 前缀判定/拆分/deferred 判定）单一来源在 kernel：
// `ha_core::mcp::catalog`。这里再导出供本 crate 内部与既有调用路径使用，
// **不得在特征侧另写前缀判定**。
pub use ha_core::mcp::catalog::{
    has_deferred_tool_server, is_mcp_tool_name, server_namespace_component, split_mcp_tool_name,
    tool_belongs_to_deferred_server, MCP_TOOL_PREFIX,
};

/// True iff a server config exposes the original MCP tool name. The filters
/// are stored pre-namespace: `allowed_tools=["search"]`, not
/// `mcp__server__search`.
pub(crate) fn tool_allowed_by_server_config(
    cfg: &McpServerConfig,
    original_tool_name: &str,
) -> bool {
    if cfg.denied_tools.iter().any(|d| d == original_tool_name) {
        return false;
    }
    cfg.allowed_tools.is_empty() || cfg.allowed_tools.iter().any(|a| a == original_tool_name)
}

/// True while at least one effective server has never completed a catalog
/// round. Provider request assembly uses this synchronous signal to keep
/// Hope's local `tool_search` callable until lazy MCP schemas exist.
pub fn has_pending_catalogs() -> bool {
    let Some(mgr) = crate::McpManager::global() else {
        return false;
    };
    let app_config = ha_core::config::cached_config();
    if !app_config.mcp_global.enabled {
        return false;
    }
    let cataloged = mgr.cataloged_server_ids();
    app_config.mcp_servers.iter().any(|server| {
        server.enabled
            && crate::config::validate_server_config(server).is_ok()
            && !app_config
                .mcp_global
                .denied_servers
                .iter()
                .any(|denied| denied == &server.name)
            && !cataloged.contains(&server.id)
    })
}

/// Build a short MCP capability-data block for configured, effective servers.
/// The provider renderer places this in the dynamic user-data lane: configured
/// server names are user-owned data, while the fixed discovery contract is
/// already declared by the stable platform prompt. Reads only sync config and
/// never awaits a runtime lock.
///
/// The snippet intentionally does not enumerate every resource / prompt
/// — that list can be large and requires an async read of the per-
/// server state. The agent discovers those via the `mcp_resource`
/// and `mcp_prompt` tools we point at here.
pub fn system_prompt_snippet() -> Option<String> {
    let _manager = crate::McpManager::global()?;
    let app_config = ha_core::config::cached_config();
    if !app_config.mcp_global.enabled {
        return None;
    }
    let configured: std::collections::BTreeSet<String> = app_config
        .mcp_servers
        .iter()
        .filter(|server| {
            server.enabled
                && crate::config::validate_server_config(server).is_ok()
                && !app_config
                    .mcp_global
                    .denied_servers
                    .iter()
                    .any(|denied| denied == &server.name)
        })
        .map(|server| server.name.clone())
        .collect();
    if configured.is_empty() {
        return None;
    }
    let list = configured.into_iter().collect::<Vec<_>>().join(", ");
    Some(format!(
        "MCP Capabilities\n\n\
         Configured MCP servers: {list}\n\
         - Tools exposed by each server appear in the tool catalog with the `mcp__<server>__<tool>` naming.\n\
         - If a configured server's tool is absent, call `tool_search` once to attempt lazy discovery; follow its result instead of retrying automatically.\n\
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
    rmcp_tool_to_definition_with_name(cfg, tool, namespaced_tool_name(&cfg.name, &orig))
}

/// Build a [`ToolDefinition`] using a pre-assigned namespaced name.
///
/// Catalog refresh paths use this after running collision resolution across the
/// whole server catalog; the single-tool helper above remains for tests and
/// call sites that do not need cross-tool uniqueness.
pub fn rmcp_tool_to_definition_with_name(
    cfg: &McpServerConfig,
    tool: &model::Tool,
    name: String,
) -> ToolDefinition {
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
        tier: ToolTier::Mcp,
        internal: false,
        concurrent_safe: false,
        // MCP 2026-07-28 task mode is selected by the call response, not by
        // an untrusted per-tool declaration. Keep catalog entries foreground
        // until the invocation path receives and validates a task handle.
        background_policy: ha_core::tools::BackgroundPolicy::ForegroundOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServerConfig, McpTransportSpec, McpTrustLevel};

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
        assert_eq!(sanitize_tool_name(&long).len(), MIN_TOOL_NAME_CAP);
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
    fn short_server_reclaims_namespace_budget_for_readable_tool_names() {
        assert_eq!(
            namespaced_tool_name("azure-mcp-large", "storage_blob_container_get"),
            "mcp__azure-mcp-large__storage_blob_container_get"
        );
    }

    #[test]
    fn delimiter_in_server_name_gets_an_unambiguous_namespace() {
        assert_eq!(
            namespaced_tool_name("foo__bar", "read"),
            "mcp__fooUUbar__read"
        );
        assert_eq!(
            assign_legacy_namespaced_tool_names("foo__bar", ["read"]),
            vec!["mcp__foo__bar__read"]
        );
    }

    #[test]
    fn legacy_assignment_preserves_pre_budget_reclamation_name() {
        assert_eq!(
            assign_legacy_namespaced_tool_names("azure-mcp-large", ["storage_blob_container_get"],),
            vec!["mcp__azure-mcp-large__storage_blob_container_ge"]
        );
    }

    #[test]
    fn expanded_names_never_steal_another_tools_legacy_identifier() {
        let long = "abcdefghijklmnopqrstuvwxyz_long";
        let old_prefix = "abcdefghijklmnopqrstuvwxy";

        assert_eq!(
            assign_namespaced_tool_names("srv", [long, old_prefix]),
            vec![
                "mcp__srv__abcdefghijklmnopqrstuvwxyz_long",
                "mcp__srv__abcdefghijklmnopqrstuvwxy_2",
            ]
        );
        assert_eq!(
            assign_legacy_namespaced_tool_names("srv", [long, old_prefix]),
            vec![
                "mcp__srv__abcdefghijklmnopqrstuvwxy",
                "mcp__srv__abcdefghijklmnopqrstuvw_2",
            ]
        );
    }

    #[test]
    fn assigned_names_are_collision_safe_and_bounded() {
        let long = "a".repeat(100);
        let names =
            assign_namespaced_tool_names("srv", ["foo-bar", "foo.bar", "foo_bar", long.as_str()]);
        assert_eq!(names[0], "mcp__srv__foo_bar");
        assert_eq!(names[1], "mcp__srv__foo_bar_2");
        assert_eq!(names[2], "mcp__srv__foo_bar_3");
        assert!(names[3].len() <= NAMESPACED_TOOL_NAME_CAP);
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
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
    fn mcp_catalog_does_not_preapprove_background_execution() {
        let cfg = min_cfg("srv");
        let tool = model::Tool::new("remote", "x", std::sync::Arc::new(serde_json::Map::new()));
        assert!(!rmcp_tool_to_definition(&cfg, &tool).supports_generic_job());
    }
}
