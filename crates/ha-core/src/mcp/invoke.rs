//! Entry point for `tools::execution` dispatch of `mcp__*` tools.
//!
//! Every `mcp__<server>__<tool>` call flows through [`call_tool`]. We
//! intentionally mirror the return type of every other tool in the tree
//! (`anyhow::Result<String>`) so the dispatch match in
//! [`crate::tools::execution::execute_tool_with_context`] looks uniform.
//!
//! Result normalization (text / image / resource link) happens here too,
//! isolated from the rest of the subsystem so Phase 5 can extend it
//! without touching registry / client code.

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult, Content, RawContent};
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
/// [`crate::tools::execution`] *before* we get here. It's kept in the
/// signature to match the dispatch shape and to leave room for
/// future per-session overrides (per-server project_paths, etc.).
pub async fn call_tool(
    name: &str,
    args: &Value,
    _ctx: &crate::tools::ToolExecContext,
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

    let call_timeout_secs = handle.config.read().await.call_timeout_secs.max(1);

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

    let fut = dispatch_inner(handle.clone(), &entry.original_tool_name, args);
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
}

async fn dispatch_inner(
    handle: Arc<ServerHandle>,
    original_name: &str,
    args: &Value,
) -> McpResult<String> {
    let cfg_name = handle.config.read().await.name.clone();

    let peer_opt = {
        let guard = handle.client.lock().await;
        guard.as_ref().map(|s| s.peer().clone())
    };
    let peer = peer_opt.ok_or_else(|| McpError::NotReady {
        server: cfg_name.clone(),
        reason: "peer vanished during call".into(),
    })?;

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

    let body = normalize_content(&result.content);

    if result.is_error.unwrap_or(false) {
        return Err(McpError::ToolFailed {
            server: cfg_name,
            tool: original_name.to_string(),
            message: if body.is_empty() {
                "server reported isError=true with no body".into()
            } else {
                body
            },
        });
    }

    // Reset the per-server failure counter on a clean call so the
    // watchdog knows the connection is healthy.
    handle
        .consecutive_failures
        .store(0, std::sync::atomic::Ordering::Relaxed);

    Ok(body)
}

/// Collapse a heterogeneous `Vec<Content>` into a single string digestible
/// by the main conversation loop. The shape is intentionally close to
/// what the built-in text tools already return so the LLM sees a
/// familiar format.
///
/// * `Text` — appended verbatim.
/// * `Image` — placeholder `[image base64 …]` line (Phase 2 stops here;
///   Phase 5 will persist the image to disk + return a file reference).
/// * `EmbeddedResource` — `[resource mime=… uri=…]\n<body>`.
/// * `ResourceLink` — `[resource_link uri=…]`.
pub fn normalize_content(blocks: &[Content]) -> String {
    if blocks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        match &block.raw {
            RawContent::Text(t) => {
                out.push_str(&t.text);
            }
            RawContent::Image(img) => {
                out.push_str(&format!(
                    "[image mime={} size_b64={}]",
                    img.mime_type,
                    img.data.len()
                ));
            }
            RawContent::Resource(r) => {
                let uri = match &r.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, .. } => uri.as_str(),
                    rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => uri.as_str(),
                };
                let body = match &r.resource {
                    rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                        text.clone()
                    }
                    rmcp::model::ResourceContents::BlobResourceContents { blob, .. } => {
                        format!("[blob base64 size={}]", blob.len())
                    }
                };
                out.push_str(&format!("[resource uri={}]\n{}", uri, body));
            }
            RawContent::ResourceLink(link) => {
                out.push_str(&format!("[resource_link uri={}]", link.uri));
            }
            RawContent::Audio(a) => {
                out.push_str(&format!(
                    "[audio mime={} size_b64={}]",
                    a.mime_type,
                    a.data.len()
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{Content, RawContent, RawTextContent};

    fn text(t: &str) -> Content {
        Content {
            raw: RawContent::Text(RawTextContent {
                text: t.to_string(),
                meta: None,
            }),
            annotations: None,
        }
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
}
