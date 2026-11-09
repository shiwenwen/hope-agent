//! Entry point for `tools::execution` dispatch of `mcp__*` tools.
//!
//! Every `mcp__<server>__<tool>` call flows through [`call_tool`]. We
//! intentionally mirror the return type of every other tool in the tree
//! (`anyhow::Result<String>`) so the dispatch match in
//! [`ha_core::tools::execution::execute_tool_with_context`] looks uniform.
//!
//! Result normalization (text / image / resource link) happens here too,
//! isolated from the rest of the subsystem so Phase 5 can extend it
//! without touching registry / client code.

use std::sync::Arc;
use std::time::Duration;

use ha_core::mcp_hooks::McpToolResultFormat;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use serde_json::Value;
use tokio::time::timeout;

use super::client::ensure_connected;
use super::errors::{McpError, McpResult};
use super::registry::{McpManager, ServerHandle};

/// Dispatch a namespaced `mcp__<server>__<tool>` call to the owning
/// server. Returns a string shaped the same way every other built-in
/// tool does so the caller can inject it directly into the next
/// tool_result message.
///
/// `ctx` is currently unused at this layer — tool-level approval and
/// sandboxing are enforced by the generic wrapper in
/// [`ha_core::tools::execution`] *before* we get here. It's kept in the
/// signature to match the dispatch shape and to leave room for
/// future per-session overrides (per-server project_paths, etc.).
pub async fn call_tool(
    name: &str,
    args: &Value,
    ctx: &ha_core::tools::ToolExecContext,
) -> anyhow::Result<String> {
    call_tool_with_format(name, args, ctx, McpToolResultFormat::Conversation).await
}

pub(crate) async fn call_tool_with_format(
    name: &str,
    args: &Value,
    ctx: &ha_core::tools::ToolExecContext,
    format: McpToolResultFormat,
) -> anyhow::Result<String> {
    let manager = McpManager::global().ok_or_else(|| {
        anyhow::anyhow!(
            "MCP subsystem is not initialized; tool '{}' cannot be dispatched",
            name
        )
    })?;

    if !manager.is_enabled().await {
        anyhow::bail!(
            "MCP subsystem is disabled in config (mcpGlobal.enabled=false); \
             tool '{}' is unavailable",
            name
        );
    }

    let entry = manager.lookup_tool(name).await.ok_or_else(|| {
        anyhow::anyhow!(
            "MCP tool '{}' is not registered — the owning server may be \
             offline or the tool was removed by a server-side catalog \
             refresh. Try reconnecting the server in Settings → MCP Servers.",
            name
        )
    })?;

    let handle = manager.get_by_id(&entry.server_id).await.ok_or_else(|| {
        anyhow::anyhow!(
            "MCP server '{}' (id={}) is not in the registry",
            entry.server_name,
            entry.server_id
        )
    })?;

    // Lazy connect on first use; if we're in Failed/NeedsAuth the error
    // bubbles up as an anyhow which the tool loop surfaces to the model
    // as an actionable message.
    ensure_connected(manager, handle.clone())
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let call_timeout_secs = handle.config.read().await.call_timeout_secs;

    // Global + per-server concurrency gating. The `Owned` permits are
    // dropped when the call completes or errors out.
    let _global = manager
        .global_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| anyhow::anyhow!("MCP global semaphore closed: {e}"))?;
    let _local = handle
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| anyhow::anyhow!("MCP per-server semaphore closed: {e}"))?;

    let start = std::time::Instant::now();
    let fut = dispatch_inner(handle.clone(), &entry.original_tool_name, args, format);
    let result: anyhow::Result<String> = if call_timeout_secs == 0 {
        match fut.await {
            Ok(body) => Ok(body),
            Err(e) => Err(anyhow::anyhow!("{}", e)),
        }
    } else {
        match timeout(Duration::from_secs(call_timeout_secs), fut).await {
            Ok(Ok(body)) => Ok(body),
            Ok(Err(e)) => Err(anyhow::anyhow!("{}", e)),
            Err(_elapsed) => {
                // Treat as a per-server failure so the health counter grows
                // and the watchdog can escalate.
                handle
                    .consecutive_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Err(anyhow::anyhow!(
                    "{}",
                    McpError::Timeout {
                        server: entry.server_name.clone(),
                        tool: entry.original_tool_name.clone(),
                        secs: call_timeout_secs,
                    }
                ))
            }
        }
    };
    emit_learning(
        &entry,
        name,
        &result,
        start.elapsed().as_millis() as u64,
        ctx,
    );
    result
}

fn emit_learning(
    entry: &crate::registry::ToolIndexEntry,
    namespaced_name: &str,
    result: &anyhow::Result<String>,
    duration_ms: u64,
    ctx: &ha_core::tools::ToolExecContext,
) {
    // Branch the meta object so the success event omits `error`
    // entirely rather than surfacing a literal `"error": null` —
    // keeps Dashboard Learning consumers from having to special-case
    // the null sentinel.
    let (kind, meta) = match result {
        Ok(_) => (
            ha_core::learning_events::EVT_MCP_TOOL_CALLED,
            serde_json::json!({
                "server": entry.server_name,
                "tool": entry.original_tool_name,
                "durationMs": duration_ms,
            }),
        ),
        Err(e) => (
            ha_core::learning_events::EVT_MCP_TOOL_FAILED,
            serde_json::json!({
                "server": entry.server_name,
                "tool": entry.original_tool_name,
                "durationMs": duration_ms,
                "error": e.to_string(),
            }),
        ),
    };
    ha_core::learning_events::emit(
        kind,
        ctx.session_id.as_deref(),
        Some(namespaced_name),
        Some(&meta),
    );
}

async fn dispatch_inner(
    handle: Arc<ServerHandle>,
    original_name: &str,
    args: &Value,
    format: McpToolResultFormat,
) -> McpResult<String> {
    let cfg_name = handle.config.read().await.name.clone();
    let peer = handle.peer().await?;

    let arguments = match args {
        Value::Object(m) => Some(m.clone()),
        Value::Null => None,
        _ => {
            // MCP spec expects a JSON object for `arguments`; wrap
            // primitives into a single-key object so servers that were
            // written for Claude Desktop-style input don't reject us.
            let mut m = serde_json::Map::new();
            m.insert("value".into(), args.clone());
            Some(m)
        }
    };

    let mut params = CallToolRequestParams::new(original_name.to_string());
    params.arguments = arguments;

    let result: CallToolResult = peer
        .call_tool(params)
        .await
        .map_err(|e| McpError::Protocol {
            server: cfg_name.clone(),
            code: None,
            message: format!("{e}"),
        })?;

    let body = normalize_result_for(&result, &cfg_name, original_name, format)?;

    // Reset the per-server failure counter on a clean call so the
    // watchdog knows the connection is healthy.
    handle
        .consecutive_failures
        .store(0, std::sync::atomic::Ordering::Relaxed);

    Ok(body)
}

/// Keep successful structured data alongside the human-readable content. A
/// present JSON null is meaningful; rmcp preserves it as `Some(Value::Null)`.
/// Error diagnostics keep their existing text-only contract: copying structured
/// payloads or protocol metadata there would also copy them into Learning events.
fn normalize_result_for(
    result: &CallToolResult,
    server: &str,
    tool: &str,
    format: McpToolResultFormat,
) -> McpResult<String> {
    let mut body = normalize_content(&result.content);
    if result.is_error.unwrap_or(false) {
        return Err(McpError::ToolFailed {
            server: server.to_string(),
            tool: tool.to_string(),
            message: if body.is_empty() {
                "server reported isError=true with no body".into()
            } else {
                body
            },
        });
    }
    if let Some(structured) = &result.structured_content {
        if let McpToolResultFormat::Hook(event) = format {
            return Ok(ha_core::hooks::parse::mcp_hook_stdout(
                structured, &body, event,
            ));
        }
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("[structuredContent]\n");
        body.push_str(&structured.to_string());
    }
    Ok(body)
}

/// Collapse a heterogeneous `Vec<ContentBlock>` into a single string digestible
/// by the main conversation loop. The shape is intentionally close to
/// what the built-in text tools already return so the LLM sees a
/// familiar format.
///
/// * `Text` — appended verbatim.
/// * `Image` — placeholder `[image base64 …]` line (Phase 2 stops here;
///   Phase 5 will persist the image to disk + return a file reference).
/// * `EmbeddedResource` — `[resource mime=… uri=…]\n<body>`.
/// * `ResourceLink` — `[resource_link uri=…]`.
pub fn normalize_content(blocks: &[ContentBlock]) -> String {
    if blocks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        match block {
            ContentBlock::Text(t) => {
                out.push_str(&t.text);
            }
            ContentBlock::Image(img) => {
                out.push_str(&format!(
                    "[image mime={} size_b64={}]",
                    img.mime_type,
                    img.data.len()
                ));
            }
            ContentBlock::Resource(r) => {
                let uri = match &r.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, .. } => uri.as_str(),
                    rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => uri.as_str(),
                    _ => "unsupported:",
                };
                let body = match &r.resource {
                    rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                        text.clone()
                    }
                    rmcp::model::ResourceContents::BlobResourceContents { blob, .. } => {
                        format!("[blob base64 size={}]", blob.len())
                    }
                    _ => "[unsupported resource content]".to_string(),
                };
                out.push_str(&format!("[resource uri={}]\n{}", uri, body));
            }
            ContentBlock::ResourceLink(link) => {
                out.push_str(&format!("[resource_link uri={}]", link.uri));
            }
            ContentBlock::Audio(a) => {
                out.push_str(&format!(
                    "[audio mime={} size_b64={}]",
                    a.mime_type,
                    a.data.len()
                ));
            }
            _ => out.push_str("[unsupported MCP content block]"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn normalize_result(result: &CallToolResult, server: &str, tool: &str) -> McpResult<String> {
        normalize_result_for(result, server, tool, McpToolResultFormat::Conversation)
    }
    fn text(t: &str) -> ContentBlock {
        ContentBlock::text(t)
    }

    #[test]
    fn normalize_single_text() {
        assert_eq!(normalize_content(&[text("hello")]), "hello");
    }

    #[test]
    fn normalize_multi_text_separator() {
        let blocks = [text("a"), text("b")];
        assert_eq!(normalize_content(&blocks), "a\n\nb");
    }

    #[test]
    fn normalize_empty_is_empty() {
        assert_eq!(normalize_content(&[]), "");
    }

    fn from_wire(value: Value) -> CallToolResult {
        serde_json::from_value(value).expect("synthetic MCP wire result")
    }

    #[test]
    fn result_without_structure_preserves_legacy_content() {
        let result = from_wire(serde_json::json!({
            "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}],
            "_meta": {"private": "not model content"}
        }));
        assert_eq!(
            normalize_result(&result, "server", "tool").unwrap(),
            "a\n\nb"
        );
        assert_eq!(
            normalize_result(
                &from_wire(serde_json::json!({"content": []})),
                "server",
                "tool"
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn result_preserves_every_structured_json_shape_with_and_without_text() {
        for value in [
            Value::Null,
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!(12.5),
            serde_json::json!("text\n[structuredContent]\n雪"),
            serde_json::json!([]),
            serde_json::json!([1, null, {"name": "雪"}]),
            serde_json::json!({}),
            serde_json::json!({"items": [], "total_count": 0}),
        ] {
            for text in [None, Some("Human-readable fallback")] {
                // Both legacy and 2026 complete responses use this SDK path.
                for complete in [false, true] {
                    let mut wire = serde_json::json!({
                        "content": text.map(|t| vec![serde_json::json!({"type": "text", "text": t})]).unwrap_or_default(),
                        "structuredContent": value,
                        "_meta": {"private": "not model content"}
                    });
                    if text.is_none() {
                        wire.as_object_mut().unwrap().remove("content");
                    }
                    if complete {
                        wire["resultType"] = serde_json::json!("complete");
                    }
                    let result = from_wire(wire);
                    assert_eq!(result.structured_content.as_ref(), Some(&value));
                    let normalized = normalize_result(&result, "server", "tool").unwrap();
                    let prefix = text.map(|t| format!("{t}\n\n")).unwrap_or_default();
                    let json = normalized
                        .strip_prefix(&format!("{prefix}[structuredContent]\n"))
                        .unwrap();
                    assert_eq!(serde_json::from_str::<Value>(json).unwrap(), value);
                    assert!(!normalized.contains("not model content"));
                }
            }
        }
    }

    #[test]
    fn structured_content_does_not_replace_media_fallbacks() {
        let result = from_wire(serde_json::json!({
            "content": [{"type": "image", "mimeType": "image/png", "data": "AA=="}],
            "structuredContent": {"width": 1}
        }));
        assert_eq!(
            normalize_result(&result, "server", "tool").unwrap(),
            "[image mime=image/png size_b64=4]\n\n[structuredContent]\n{\"width\":1}"
        );
    }

    #[test]
    fn structured_errors_remain_failures_without_new_payload_diagnostics() {
        for text in [None, Some("try a different argument")] {
            let result = from_wire(serde_json::json!({
                "content": text.map(|t| vec![serde_json::json!({"type": "text", "text": t})]).unwrap_or_default(),
                "structuredContent": {"private": "not diagnostic content"},
                "isError": true
            }));
            for format in [
                McpToolResultFormat::Conversation,
                McpToolResultFormat::Hook(ha_core::hooks::HookEvent::PreToolUse),
            ] {
                match normalize_result_for(&result, "server", "tool", format).unwrap_err() {
                    McpError::ToolFailed {
                        server,
                        tool,
                        message,
                    } => {
                        assert_eq!(server, "server");
                        assert_eq!(tool, "tool");
                        assert_eq!(
                            message,
                            text.unwrap_or("server reported isError=true with no body")
                        );
                    }
                    other => panic!("unexpected error: {other}"),
                }
            }
        }
    }
}
