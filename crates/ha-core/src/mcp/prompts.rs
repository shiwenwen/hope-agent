//! MCP prompt primitives — `prompts/list` snapshot + `prompts/get` RPC
//! wrapper. Symmetric to [`super::resources`].
//!
//! Prompts are server-hosted templates a user or agent invokes by name
//! to produce a seed conversation (one or more `{role, content}`
//! messages). Common uses: "analyze-commit", "review-pr",
//! "explain-error". We project the rmcp types down to plain-string
//! shapes so frontends + tool handlers don't drag rmcp into their
//! dependency tree.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use rmcp::model;
use serde::Serialize;

use super::registry::ServerState;

/// Compact shape of one entry in `prompts/list`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSummary {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<PromptArgument>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub required: bool,
}

/// One message in a `prompts/get` result. Each message carries a role
/// (`user` / `assistant`) and a text body. Image / resource message
/// parts flatten to a short placeholder so the caller gets a single
/// printable string — full multimedia handling lives in a later phase.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMessage {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPromptResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

pub async fn list_prompts(server_name_or_id: &str) -> Result<Vec<PromptSummary>> {
    let handle = super::locate_server(server_name_or_id).await?;
    let state = handle.state.lock().await;
    let prompts = match &*state {
        ServerState::Ready { prompts, .. } => prompts.clone(),
        other => {
            return Err(anyhow!(
                "MCP server '{}' is not in a Ready state (currently {}); \
                 connect it first before listing prompts",
                server_name_or_id,
                other.label()
            ));
        }
    };
    Ok(prompts.into_iter().map(project_prompt).collect())
}

pub async fn get_prompt(
    server_name_or_id: &str,
    name: &str,
    arguments: Option<BTreeMap<String, String>>,
) -> Result<GetPromptResponse> {
    let handle = super::locate_server(server_name_or_id).await?;
    let peer = handle.peer().await.map_err(|e| anyhow!("{e}"))?;
    // rmcp accepts `arguments: Map<String, Value>`; we convert the
    // simpler string-to-string shape callers pass in.
    let mut req = model::GetPromptRequestParams::new(name);
    if let Some(m) = arguments {
        req = req.with_arguments(
            m.into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect::<serde_json::Map<_, _>>(),
        );
    }
    let result = peer
        .get_prompt(req)
        .await
        .map_err(|e| anyhow!("prompts/get failed: {e}"))?;
    Ok(GetPromptResponse {
        description: result.description,
        messages: result.messages.into_iter().map(project_message).collect(),
    })
}

fn project_prompt(p: model::Prompt) -> PromptSummary {
    PromptSummary {
        name: p.name,
        description: p.description,
        arguments: p
            .arguments
            .unwrap_or_default()
            .into_iter()
            .map(|a| PromptArgument {
                name: a.name,
                description: a.description,
                required: a.required.unwrap_or(false),
            })
            .collect(),
    }
}

/// Flatten an rmcp prompt message to `{role, text}`. Non-text content
/// (images, embedded resources) reduces to a short bracketed placeholder
/// — same convention as [`crate::mcp::invoke`]'s tool-result
/// normalization so the two paths behave identically for the LLM.
fn project_message(m: model::PromptMessage) -> PromptMessage {
    let role = match m.role {
        model::PromptMessageRole::User => "user",
        model::PromptMessageRole::Assistant => "assistant",
    };
    let text = match m.content {
        model::PromptMessageContent::Text { text } => text,
        model::PromptMessageContent::Image { image } => {
            format!(
                "[image mime={} size_b64={}]",
                image.mime_type,
                image.data.len()
            )
        }
        model::PromptMessageContent::Resource { resource } => match resource.raw.resource {
            model::ResourceContents::TextResourceContents { text, uri, .. } => {
                format!("[resource uri={uri}]\n{text}")
            }
            model::ResourceContents::BlobResourceContents { uri, blob, .. } => {
                format!("[resource uri={uri} blob size={}]", blob.len())
            }
        },
        model::PromptMessageContent::ResourceLink { link } => {
            format!("[resource_link uri={}]", link.raw.uri)
        }
    };
    PromptMessage {
        role: role.to_string(),
        text,
    }
}

/// `mcp_prompt` tool: exposes `list` / `get` through a single action
/// dispatcher. `get` accepts an optional `arguments` object mapping
/// prompt argument names to string values.
pub(crate) async fn tool_mcp_prompt(args: &serde_json::Value) -> Result<String> {
    let server = args
        .get("server")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing required field: server"))?;
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");
    match action {
        "list" => {
            let prompts = list_prompts(server).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "server": server,
                "prompts": prompts,
            }))?)
        }
        "get" => {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing required field: name (for action=get)"))?;
            // Reject non-string argument values loudly — silently
            // dropping them would let the model believe the prompt was
            // called with its intended args when in fact the server
            // saw an empty object, producing confusing downstream
            // results.
            let arguments = args
                .get("arguments")
                .and_then(|v| v.as_object())
                .map(|obj| -> Result<BTreeMap<String, String>> {
                    obj.iter()
                        .map(|(k, v)| {
                            v.as_str()
                                .map(|s| (k.clone(), s.to_string()))
                                .ok_or_else(|| {
                                    anyhow!("prompt argument '{k}' must be a string (got {})", v)
                                })
                        })
                        .collect()
                })
                .transpose()?;
            let resp = get_prompt(server, name, arguments).await?;
            Ok(serde_json::to_string_pretty(&resp)?)
        }
        other => Err(anyhow!("unknown action '{other}'; supported: list, get")),
    }
}
