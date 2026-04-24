//! MCP resource primitives — `resources/list` snapshot + `resources/read`
//! RPC wrapper.
//!
//! Split from [`invoke`] because resources aren't tool calls: they're a
//! passive catalog the host fetches on demand. The sync path (the
//! `Ready` state already caches the most recent `list`) and the RPC
//! path (`read`) both live here so tool handlers stay thin.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rmcp::model;
use serde::Serialize;

use super::registry::ServerState;

/// Compact shape of one entry in `resources/list`. Mirrors the subset
/// of `rmcp::model::Resource` that frontends and tool handlers actually
/// read — raw `rmcp` types aren't serde-friendly in every field, so we
/// project to plain strings here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSummary {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// One content block returned by `resources/read`. Spec allows a single
/// `read` call to return multiple blocks (e.g. a PDF resource split
/// into text + embedded image).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResourcePart {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// UTF-8 text body when the content is a text resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Base64-encoded payload when the content is a binary resource.
    /// We re-encode from the server's own base64 string so callers can
    /// tell "it's really binary" from "the server happened to use b64
    /// for text" — structurally identical but semantically different.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_base64: Option<String>,
}

/// Return the cached resource catalog for a server.
///
/// Fast path — reads the last `resources/list` snapshot embedded in
/// `ServerState::Ready`. Does **not** trigger a connect or a fresh
/// `list` call; callers wanting live data should invoke
/// `test_connection` / `reconnect_server` first.
pub async fn list_resources(server_name_or_id: &str) -> Result<Vec<ResourceSummary>> {
    let handle = super::locate_server(server_name_or_id).await?;
    let state = handle.state.lock().await;
    let resources = match &*state {
        ServerState::Ready { resources, .. } => resources.clone(),
        other => {
            return Err(anyhow!(
                "MCP server '{}' is not in a Ready state (currently {}); \
                 connect it first before listing resources",
                server_name_or_id,
                other.label()
            ));
        }
    };
    Ok(resources
        .into_iter()
        .map(|r| ResourceSummary {
            uri: r.raw.uri,
            name: r.raw.name,
            description: r.raw.description,
            mime_type: r.raw.mime_type,
        })
        .collect())
}

/// Call `resources/read` on the live connection and normalize the
/// response. Mirrors the content-block handling in
/// [`crate::mcp::invoke`] so a resource read looks identical to a
/// `tool_call` that happened to return a `RawContent::Resource`.
pub async fn read_resource(server_name_or_id: &str, uri: &str) -> Result<Vec<ReadResourcePart>> {
    let handle = super::locate_server(server_name_or_id).await?;
    let peer = handle.peer().await.map_err(|e| anyhow!("{e}"))?;
    let result = peer
        .read_resource(model::ReadResourceRequestParams::new(uri))
        .await
        .map_err(|e| anyhow!("resources/read failed: {e}"))?;
    Ok(result
        .contents
        .into_iter()
        .map(normalize_resource_content)
        .collect())
}

fn normalize_resource_content(c: model::ResourceContents) -> ReadResourcePart {
    match c {
        model::ResourceContents::TextResourceContents {
            uri,
            text,
            mime_type,
            meta: _,
        } => ReadResourcePart {
            uri,
            mime_type,
            text: Some(text),
            blob_base64: None,
        },
        model::ResourceContents::BlobResourceContents {
            uri,
            blob,
            mime_type,
            meta: _,
        } => ReadResourcePart {
            uri,
            mime_type,
            text: None,
            // `blob` is already base64 per spec; we re-encode defensively
            // in case a non-conforming server hands back raw bytes in
            // the field (rare but observed in the wild).
            blob_base64: Some(maybe_reencode(&blob)),
        },
    }
}

/// Most servers already send a valid base64 string for `blob`. For the
/// minority that don't (observed in the wild: a handful of first-party
/// servers hand raw bytes through the field), encode on their behalf.
///
/// Uses a zero-allocation **charset validation** instead of a full
/// `BASE64.decode` — a compliant blob can be megabytes, and allocating
/// a decoded scratch buffer just to discard it would double the peak
/// memory of every `resources/read`.
fn maybe_reencode(blob: &str) -> String {
    if is_valid_base64(blob) {
        blob.to_string()
    } else {
        BASE64.encode(blob.as_bytes())
    }
}

/// True iff `s` could plausibly be URL-safe **or** standard base64
/// (spec allows either on the wire). Accepts trailing `=` padding; rejects
/// anything outside the base64 alphabet. Does not validate that the
/// bytes actually decode to legitimate content — that's the consumer's
/// concern.
fn is_valid_base64(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    // Standard base64 is padded to multiples of 4. Unpadded URL-safe
    // form is also accepted — we allow either length mod 4 == 0 (with
    // `=` padding) or the unpadded alternative.
    let stripped = s.trim_end_matches('=');
    s.bytes().all(|b| {
        b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'-' || b == b'_' || b == b'='
    }) && (s.len() - stripped.len()) <= 2
}

/// `mcp_resource` tool: exposes `list` / `read` through a single action
/// dispatcher. Model picks the action, we route to the sync snapshot
/// (list) or the RPC (read).
pub(crate) async fn tool_mcp_resource(args: &serde_json::Value) -> Result<String> {
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
            let resources = list_resources(server).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "server": server,
                "resources": resources,
            }))?)
        }
        "read" => {
            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing required field: uri (for action=read)"))?;
            let parts = read_resource(server, uri).await?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "server": server,
                "uri": uri,
                "contents": parts,
            }))?)
        }
        other => Err(anyhow!("unknown action '{other}'; supported: list, read")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_validator_accepts_compliant_input() {
        assert!(is_valid_base64(""));
        assert!(is_valid_base64("SGVsbG8="));
        assert!(is_valid_base64("SGVsbG8gd29ybGQ="));
        assert!(is_valid_base64("aGVsbG8"));
        // URL-safe alphabet.
        assert!(is_valid_base64("aG_-bA"));
    }

    #[test]
    fn base64_validator_rejects_non_charset() {
        assert!(!is_valid_base64("hello world"));
        assert!(!is_valid_base64("contains*illegal"));
        // Too much padding → reject.
        assert!(!is_valid_base64("SGVsbG8==="));
    }

    #[test]
    fn maybe_reencode_passes_through_valid_and_encodes_raw() {
        let compliant = "SGVsbG8="; // "Hello" in base64
        assert_eq!(maybe_reencode(compliant), compliant);

        let raw = "not-base64 with spaces";
        let encoded = maybe_reencode(raw);
        assert_ne!(encoded, raw);
        assert!(is_valid_base64(&encoded));
    }
}
