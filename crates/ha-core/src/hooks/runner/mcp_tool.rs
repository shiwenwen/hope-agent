//! `mcp_tool` hook handler — invokes an MCP tool and returns its result as the
//! hook's stdout (design §7.4).
//!
//! The tool result rides the normal output parser: it becomes
//! `additionalContext` for the plaintext-accepting events (SessionStart /
//! UserPromptSubmit) or, for any event, when the tool emits the JSON protocol
//! envelope. A failed / unavailable tool is a non-blocking error (never blocks
//! the host path). Bounded by the handler deadline like every other handler.
//!
//! ## Input templating
//!
//! `config.input` supports `${dotted.path}` placeholders against the
//! [`HookInput`], expanded by [`expand_input_template`]. Without expansion a
//! `${tool_input.file_path}` value would land in the MCP tool as a literal
//! placeholder string — a silent failure for security-scanner / formatter
//! hooks that depend on routing the call's actual arguments. Unresolved
//! placeholders stay literal and surface as an audit warn.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::config::McpToolHookConfig;
use super::super::env::HookEnv;
use super::super::types::HookInput;
use super::{HookHandler, RawHookResult};

/// Default `mcp_tool` hook timeout.
const DEFAULT_MCP_TIMEOUT_SECS: u64 = 30;

pub struct McpToolHandler {
    config: McpToolHookConfig,
}

impl McpToolHandler {
    pub fn new(config: McpToolHookConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl HookHandler for McpToolHandler {
    fn identity(&self) -> String {
        // Include a hash of `config.input` so two `mcp_tool` hooks targeting the
        // same `(server, tool)` with DIFFERENT inputs (e.g. one passing
        // `${tool_input.file_path}` for write events, another passing
        // `${tool_input.command}` for exec events) are NOT collapsed by the
        // dispatch-time `(handler_type, identity)` dedup. Without this they'd
        // share one identity and only the first config would actually run.
        format!(
            "{}|{}|{}",
            self.config.server,
            self.config.tool,
            input_hash(self.config.input.as_ref()),
        )
    }

    fn handler_type(&self) -> &'static str {
        "mcp_tool"
    }

    fn default_timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout.unwrap_or(DEFAULT_MCP_TIMEOUT_SECS))
    }

    async fn run(&self, input: &HookInput, _env: &HookEnv, deadline: Instant) -> RawHookResult {
        let start = Instant::now();
        let name = format!("mcp__{}__{}", self.config.server, self.config.tool);
        let template = self.config.input.clone().unwrap_or_else(|| json!({}));
        let (args, unresolved) = expand_input_template(&template, input);
        if !unresolved.is_empty() {
            // Don't fail the call — a placeholder that can't resolve for the
            // current event (e.g. `${tool_response.*}` on a PreToolUse fire)
            // is legitimate. Audit so the hook author notices typos /
            // misrouted events rather than wondering why the MCP call got
            // garbage. The MCP server still sees the literal `${...}` and
            // can choose how to react.
            crate::app_warn!(
                "hooks",
                "mcp_tool",
                "mcp_tool hook '{}' has unresolved input placeholder(s) {:?}",
                name,
                unresolved
            );
        }

        // A minimal tool context — `call_tool` only reads session/agent ids for
        // logging; the MCP registry resolves the server + concurrency itself.
        let common = input.common();
        let ctx = crate::tools::ToolExecContext {
            session_id: (!common.session_id.is_empty()).then(|| common.session_id.clone()),
            agent_id: common.agent_id.clone(),
            ..Default::default()
        };

        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1));
        match tokio::time::timeout(remaining, crate::mcp::invoke::call_tool(&name, &args, &ctx))
            .await
        {
            Ok(Ok(body)) => RawHookResult {
                exit_code: Some(0),
                stdout: body,
                stderr: String::new(),
                duration: start.elapsed(),
                timed_out: false,
            },
            Ok(Err(e)) => {
                RawHookResult::non_blocking_error(format!("mcp_tool hook '{name}' failed: {e}"))
            }
            Err(_) => RawHookResult {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("mcp_tool hook '{name}' timed out"),
                duration: start.elapsed(),
                timed_out: true,
            },
        }
    }
}

/// Stable hex hash of a JSON template, used to disambiguate two `mcp_tool`
/// hooks targeting the same `(server, tool)` with different inputs. SipHash
/// (`std::collections::hash_map::DefaultHasher`) is fine here — collisions are
/// implausible for short JSON shapes and a collision is at worst a wrongful
/// dedup of two identical-by-luck templates (which would behave the same).
fn input_hash(input: Option<&Value>) -> String {
    let mut h = DefaultHasher::new();
    match input {
        Some(v) => serde_json::to_string(v).unwrap_or_default().hash(&mut h),
        None => "".hash(&mut h),
    }
    format!("{:016x}", h.finish())
}

/// Recursively expand `${dotted.path}` placeholders in `template` against
/// `hook_input`. Returns the expanded value and the list of placeholder paths
/// that didn't resolve for this event (unresolved placeholders stay literal,
/// so a configured `${tool_response.foo}` on a PreToolUse fire leaves the
/// MCP call seeing `"${tool_response.foo}"` and the caller logs an audit).
///
/// Path roots recognized:
///   - `tool_input.*` — only on `PreToolUse` / `PostToolUse` / `PostToolUseFailure`
///   - `tool_response.*` — only on `PostToolUse` (PostToolUseFailure carries
///     `error`, not `tool_response`, by design)
///   - `tool_name` — same events as `tool_input.*`
///   - `session_id`, `cwd`, `agent_id` — common fields, always available
///     (`agent_id` resolves to empty when not set)
///   - `prompt` — only on `UserPromptSubmit`
fn expand_input_template(template: &Value, hook_input: &HookInput) -> (Value, Vec<String>) {
    let mut unresolved = Vec::new();
    let expanded = expand_value(template, hook_input, &mut unresolved);
    (expanded, unresolved)
}

fn expand_value(v: &Value, input: &HookInput, unresolved: &mut Vec<String>) -> Value {
    match v {
        Value::String(s) => Value::String(expand_string(s, input, unresolved)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|x| expand_value(x, input, unresolved))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), expand_value(v, input, unresolved)))
                .collect(),
        ),
        // Numbers / bool / null pass through unchanged — placeholders only
        // appear inside string leaves of the template.
        _ => v.clone(),
    }
}

fn expand_string(s: &str, input: &HookInput, unresolved: &mut Vec<String>) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            if let Some(close_rel) = bytes[i + 2..].iter().position(|b| *b == b'}') {
                let path_start = i + 2;
                let path_end = path_start + close_rel;
                let path = &s[path_start..path_end];
                if !path.is_empty() {
                    match resolve_path(path, input) {
                        Some(val) => out.push_str(&value_to_string(&val)),
                        None => {
                            out.push_str(&s[i..=path_end]); // keep literal
                            unresolved.push(path.to_string());
                        }
                    }
                } else {
                    // `${}` collapses to literal — no useful expansion.
                    out.push_str("${}");
                }
                i = path_end + 1;
                continue;
            }
            // Unterminated `${` → treat the rest as literal.
            out.push_str(&s[i..]);
            break;
        }
        // Copy the WHOLE UTF-8 char at `i`, not a single byte. `bytes[i] as
        // char` Latin-1-expands a multi-byte sequence — a CJK / accented
        // literal in a template (e.g. `{"path":"项目/笔记.md ${tool_input.x}"}`)
        // would be mangled into mojibake before reaching the MCP tool. `i`
        // always lands on a char boundary (every other branch advances past
        // an ASCII `$`/`{`/`}` delimiter), so the slice + `chars().next()`
        // never panics.
        let ch = s[i..]
            .chars()
            .next()
            .expect("loop index stays on a UTF-8 char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Format a `Value` for substitution into a string template. Scalars become
/// their natural textual form (no surrounding quotes); composite values
/// serialize back to JSON so a template like `"args": "${tool_input}"`
/// produces a valid JSON string carrying the nested shape.
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn resolve_path(path: &str, input: &HookInput) -> Option<Value> {
    let mut parts = path.split('.');
    let root = parts.next()?;
    let mut cur = root_value(root, input)?;
    for part in parts {
        cur = match cur {
            Value::Object(ref map) => map.get(part)?.clone(),
            Value::Array(ref arr) => {
                let idx: usize = part.parse().ok()?;
                arr.get(idx)?.clone()
            }
            _ => return None,
        };
    }
    Some(cur)
}

fn root_value(name: &str, input: &HookInput) -> Option<Value> {
    let common = input.common();
    match name {
        "session_id" => Some(Value::String(common.session_id.clone())),
        "cwd" => Some(Value::String(common.cwd.to_string_lossy().into_owned())),
        "agent_id" => common.agent_id.clone().map(Value::String),
        "tool_name" => input.tool_name().map(|s| Value::String(s.to_string())),
        "tool_input" => input.tool_input().cloned(),
        "tool_response" => match input {
            HookInput::PostToolUse { tool_response, .. } => Some(tool_response.clone()),
            _ => None,
        },
        "prompt" => match input {
            HookInput::UserPromptSubmit { prompt, .. } => Some(Value::String(prompt.clone())),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{CommonHookInput, PermissionMode};
    use std::path::PathBuf;

    fn common(event: &str) -> CommonHookInput {
        CommonHookInput {
            session_id: "sess-abc".into(),
            transcript_path: PathBuf::from("/tmp/t.jsonl"),
            cwd: PathBuf::from("/tmp/proj"),
            permission_mode: PermissionMode::Default,
            hook_event_name: event.into(),
            agent_id: Some("ha-main".into()),
            agent_type: None,
        }
    }

    fn pre_tool(tool: &str, tool_input: Value) -> HookInput {
        HookInput::PreToolUse {
            common: common("PreToolUse"),
            tool_name: tool.into(),
            tool_input,
            tool_use_id: "c1".into(),
        }
    }

    #[test]
    fn expand_tool_input_dotted_path() {
        let input = pre_tool("exec", json!({ "command": "rm -rf /tmp/x" }));
        let template = json!({ "cmd": "${tool_input.command}" });
        let (out, unresolved) = expand_input_template(&template, &input);
        assert_eq!(out, json!({ "cmd": "rm -rf /tmp/x" }));
        assert!(unresolved.is_empty());
    }

    #[test]
    fn multibyte_utf8_literal_and_value_preserved() {
        // Regression: `bytes[i] as char` Latin-1-expanded multi-byte UTF-8 in
        // the literal runs of a template, mangling CJK / accented text before
        // it reached the MCP tool. Both the literal around the placeholder AND
        // the substituted value must round-trip verbatim.
        let input = pre_tool("write", json!({ "path": "项目/笔记.md" }));
        let template = json!({ "msg": "保存 café → ${tool_input.path} ✓" });
        let (out, unresolved) = expand_input_template(&template, &input);
        assert_eq!(out, json!({ "msg": "保存 café → 项目/笔记.md ✓" }));
        assert!(unresolved.is_empty());
    }

    #[test]
    fn expand_common_fields() {
        let input = pre_tool("write", json!({ "path": "src/a.rs" }));
        let template = json!({
            "session": "${session_id}",
            "cwd": "${cwd}",
            "agent": "${agent_id}",
            "tool": "${tool_name}",
        });
        let (out, unresolved) = expand_input_template(&template, &input);
        assert_eq!(out["session"], json!("sess-abc"));
        assert_eq!(out["cwd"], json!("/tmp/proj"));
        assert_eq!(out["agent"], json!("ha-main"));
        assert_eq!(out["tool"], json!("write"));
        assert!(unresolved.is_empty());
    }

    #[test]
    fn unresolved_placeholder_stays_literal_and_is_reported() {
        // `tool_response` is only set on PostToolUse — referencing it from a
        // PreToolUse fire must leave the placeholder literal so the MCP
        // server doesn't get a silently-empty value (security hooks would
        // then mistake "no error" for "no response", a fail-open).
        let input = pre_tool("read", json!({ "path": "src/a.rs" }));
        let template = json!({ "out": "${tool_response.status}" });
        let (out, unresolved) = expand_input_template(&template, &input);
        assert_eq!(out, json!({ "out": "${tool_response.status}" }));
        assert_eq!(unresolved, vec!["tool_response.status"]);
    }

    #[test]
    fn expand_nested_object_and_array() {
        let input = pre_tool("exec", json!({ "command": "ls", "options": ["-la", "-h"] }));
        let template = json!({
            "outer": {
                "cmd": "${tool_input.command}",
                "first_opt": "${tool_input.options.0}",
            },
            "list": ["${tool_name}", "${session_id}"],
        });
        let (out, _u) = expand_input_template(&template, &input);
        assert_eq!(out["outer"]["cmd"], json!("ls"));
        assert_eq!(out["outer"]["first_opt"], json!("-la"));
        assert_eq!(out["list"][0], json!("exec"));
        assert_eq!(out["list"][1], json!("sess-abc"));
    }

    #[test]
    fn whole_tool_input_object_serializes_as_json_string() {
        let input = pre_tool("exec", json!({ "command": "ls", "cwd": "/x" }));
        let template = json!({ "args": "${tool_input}" });
        let (out, _u) = expand_input_template(&template, &input);
        // The whole input object lands as a JSON string — composite values
        // can't drop into a string template any other way.
        let s = out["args"].as_str().expect("args resolves to a string");
        let reparsed: Value = serde_json::from_str(s).expect("reparseable");
        assert_eq!(reparsed["command"], json!("ls"));
        assert_eq!(reparsed["cwd"], json!("/x"));
    }

    #[test]
    fn identity_disambiguates_different_inputs() {
        // Two hook configs targeting the same (server, tool) but with
        // different `input` templates must NOT dedup to one identity, or the
        // dispatch dedup would silently drop one of them.
        let a = McpToolHandler::new(McpToolHookConfig {
            server: "scanner".into(),
            tool: "scan".into(),
            input: Some(json!({ "path": "${tool_input.file_path}" })),
            timeout: None,
            if_rule: None,
            status_message: None,
            once: None,
        });
        let b = McpToolHandler::new(McpToolHookConfig {
            server: "scanner".into(),
            tool: "scan".into(),
            input: Some(json!({ "cmd": "${tool_input.command}" })),
            timeout: None,
            if_rule: None,
            status_message: None,
            once: None,
        });
        assert_ne!(a.identity(), b.identity());
        // But two configs with identical inputs (including None) still dedup.
        let c = McpToolHandler::new(McpToolHookConfig {
            server: "scanner".into(),
            tool: "scan".into(),
            input: Some(json!({ "path": "${tool_input.file_path}" })),
            timeout: None,
            if_rule: None,
            status_message: None,
            once: None,
        });
        assert_eq!(a.identity(), c.identity());
    }

    #[test]
    fn missing_optional_root_is_unresolved_not_empty() {
        // `agent_id` is `Option<String>`; when absent the placeholder must
        // stay literal so the MCP server sees something visibly wrong rather
        // than an empty string (a silent fail-open for hooks that gate on
        // agent identity).
        let mut c = common("PreToolUse");
        c.agent_id = None;
        let input = HookInput::PreToolUse {
            common: c,
            tool_name: "exec".into(),
            tool_input: json!({}),
            tool_use_id: "c1".into(),
        };
        let template = json!({ "agent": "${agent_id}" });
        let (out, unresolved) = expand_input_template(&template, &input);
        assert_eq!(out, json!({ "agent": "${agent_id}" }));
        assert_eq!(unresolved, vec!["agent_id"]);
    }
}
