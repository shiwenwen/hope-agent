//! ACP protocol type definitions (JSON-RPC 2.0 + Agent Client Protocol)

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC 2.0 base types ─────────────────────────────────────

/// Incoming JSON-RPC message (request or notification)
#[derive(Debug, Deserialize)]
pub struct JsonRpcMessage {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    // For responses (when acting as client)
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Outgoing JSON-RPC response
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Outgoing JSON-RPC notification (no id)
#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

// ── ACP-specific error codes ─────────────────────────────────────

pub const ERROR_PARSE: i64 = -32700;
pub const ERROR_INVALID_REQUEST: i64 = -32600;
pub const ERROR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERROR_INVALID_PARAMS: i64 = -32602;
pub const ERROR_INTERNAL: i64 = -32603;

// ── ACP Initialize ──────────────────────────────────────────────

pub const ACP_PROTOCOL_VERSION_V1: u32 = 1;

/// ACP v1 uses an integer major version. The legacy string variant exists only
/// for explicitly pinned 0.2 clients during the compatibility window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AcpProtocolVersion {
    V1(u32),
    Legacy(String),
}

impl AcpProtocolVersion {
    pub fn negotiated(requested: &Self) -> Self {
        match requested {
            Self::V1(ACP_PROTOCOL_VERSION_V1) => Self::V1(ACP_PROTOCOL_VERSION_V1),
            Self::Legacy(version) if version == "0.2" => Self::Legacy(version.clone()),
            _ => Self::V1(ACP_PROTOCOL_VERSION_V1),
        }
    }

    pub fn is_v1(&self) -> bool {
        matches!(self, Self::V1(ACP_PROTOCOL_VERSION_V1))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub protocol_version: AcpProtocolVersion,
    #[serde(default)]
    pub client_capabilities: Option<ClientCapabilities>,
    #[serde(default)]
    pub client_info: Option<ClientInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default)]
    pub fs: Option<FsCapabilities>,
    #[serde(default)]
    pub terminal: Option<bool>,
    #[serde(default)]
    pub auth: Option<Value>,
    #[serde(default)]
    pub elicitation: Option<Value>,
    #[serde(default)]
    pub session: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    #[serde(default)]
    pub read_text_file: bool,
    #[serde(default)]
    pub write_text_file: bool,
}

#[derive(Debug, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub protocol_version: AcpProtocolVersion,
    pub agent_capabilities: AgentCapabilities,
    pub agent_info: AgentInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_capabilities: Option<McpCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_capabilities: Option<SessionCapabilities>,
}

#[derive(Debug, Serialize)]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    #[serde(rename = "embeddedContext")]
    pub embedded_context: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct McpCapabilities {
    pub http: bool,
    pub sse: bool,
}

#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub name: String,
    pub title: String,
    pub version: String,
}

// ── ACP Session ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionRequest {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionRequest {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeSessionRequest {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

pub type ResumeSessionResponse = LoadSessionResponse;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsRequest {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct CloseSessionResponse {}

// ── ACP Session Modes & Config ──────────────────────────────────

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigOption {
    #[serde(rename = "type")]
    pub option_type: String,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub description: String,
    pub current_value: String,
    pub options: Vec<ConfigOptionValue>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ConfigOptionValue {
    pub value: String,
    pub name: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeState {
    pub current_mode_id: String,
    pub available_modes: Vec<SessionMode>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionModeRequest {
    pub session_id: String,
    #[serde(default)]
    pub mode_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetSessionModeResponse {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionConfigOptionRequest {
    pub session_id: String,
    pub config_id: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionConfigOptionResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

// ── ACP Prompt ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

/// ACP content block (text, image, resource, resource_link)
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        #[serde(default)]
        data: Option<String>,
        #[serde(default, rename = "mimeType")]
        mime_type: Option<String>,
    },
    #[serde(rename = "audio")]
    Audio {
        #[serde(default)]
        data: Option<String>,
        #[serde(default, rename = "mimeType")]
        mime_type: Option<String>,
    },
    #[serde(rename = "resource")]
    Resource {
        #[serde(default)]
        resource: Option<ResourceContent>,
    },
    #[serde(rename = "resource_link")]
    ResourceLink {
        #[serde(default)]
        uri: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
pub struct ResourceContent {
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: String,
}

// ── ACP Cancel ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelNotification {
    pub session_id: String,
}

// ── ACP Authenticate ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AuthenticateRequest {
    #[serde(default)]
    pub method: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthenticateResponse {}

// ── ACP Session Update notifications ────────────────────────────

/// Types of session updates sent as notifications to the client
#[derive(Debug, Serialize)]
#[serde(tag = "sessionUpdate")]
pub enum SessionUpdate {
    #[serde(rename = "agent_message_chunk")]
    AgentMessageChunk {
        #[serde(skip_serializing_if = "Option::is_none", rename = "messageId")]
        message_id: Option<String>,
        content: TextContent,
    },
    #[serde(rename = "agent_thought_chunk")]
    AgentThoughtChunk { content: TextContent },
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        title: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "rawInput")]
        raw_input: Option<Value>,
    },
    #[serde(rename = "tool_call_update")]
    ToolCallUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ToolCallContent>>,
    },
    #[serde(rename = "usage_update")]
    UsageUpdate {
        used: u64,
        size: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<SessionCost>,
    },
    #[serde(rename = "plan")]
    Plan { entries: Vec<PlanEntry> },
    #[serde(rename = "session_info_update")]
    SessionInfoUpdate {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "updatedAt")]
        updated_at: Option<String>,
    },
    #[serde(rename = "available_commands_update")]
    AvailableCommandsUpdate {
        #[serde(rename = "availableCommands")]
        available_commands: Vec<AvailableCommand>,
    },
    #[serde(rename = "current_mode_update")]
    CurrentModeUpdate {
        #[serde(rename = "currentModeId")]
        current_mode_id: String,
    },
    #[serde(rename = "config_option_update")]
    ConfigOptionUpdate {
        #[serde(rename = "configOptions")]
        config_options: Vec<SessionConfigOption>,
    },
    /// User message replay (for loadSession)
    #[serde(rename = "user_message_chunk")]
    UserMessageChunk {
        #[serde(skip_serializing_if = "Option::is_none", rename = "messageId")]
        message_id: Option<String>,
        content: TextContent,
    },
}

/// Build the version-specific `session/update` notification envelope.
/// ACP v1 renamed the nested payload from `sessionUpdate` to `update`.
pub fn session_update_params(
    protocol_version: &AcpProtocolVersion,
    session_id: &str,
    update: Value,
) -> Value {
    let mut params = serde_json::Map::new();
    params.insert(
        "sessionId".to_string(),
        Value::String(session_id.to_string()),
    );
    params.insert(
        if protocol_version.is_v1() {
            "update"
        } else {
            "sessionUpdate"
        }
        .to_string(),
        update,
    );
    Value::Object(params)
}

#[derive(Debug, Serialize)]
pub struct SessionCost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Serialize)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl TextContent {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ToolCallContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub content: TextContent,
}

#[derive(Debug, Serialize)]
pub struct AvailableCommand {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Session metadata parsing helpers ────────────────────────────

/// Parse _meta object to extract session routing hints
pub fn parse_session_meta(meta: &Option<Value>) -> SessionMeta {
    let meta = match meta.as_ref().and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return SessionMeta::default(),
    };
    SessionMeta {
        agent_id: meta
            .get("agentId")
            .and_then(|v| v.as_str())
            .map(String::from),
        session_key: meta
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .map(String::from),
        reset_session: meta
            .get("resetSession")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

#[derive(Debug, Default)]
pub struct SessionMeta {
    pub agent_id: Option<String>,
    pub session_key: Option<String>,
    pub reset_session: bool,
}

// ── Prompt extraction helpers ───────────────────────────────────

/// Maximum prompt size (2MB) to prevent DoS
pub const MAX_PROMPT_BYTES: usize = 2 * 1024 * 1024;

/// Extract text content from ACP prompt content blocks
pub fn extract_text_from_prompt(prompt: &[ContentBlock]) -> anyhow::Result<String> {
    let mut parts = Vec::new();
    let mut total_bytes: usize = 0;

    for block in prompt {
        let text = match block {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::Resource { resource } => resource.as_ref().and_then(|r| r.text.clone()),
            ContentBlock::ResourceLink { uri, title } => {
                let title_part = title
                    .as_deref()
                    .map(|t| format!(" ({t})"))
                    .unwrap_or_default();
                let uri_part = uri.as_deref().unwrap_or("");
                Some(format!("[Resource link{title_part}] {uri_part}"))
            }
            ContentBlock::Image { .. } | ContentBlock::Audio { .. } => None,
        };
        if let Some(t) = text {
            total_bytes += t.len() + if parts.is_empty() { 0 } else { 1 };
            if total_bytes > MAX_PROMPT_BYTES {
                anyhow::bail!(
                    "Prompt exceeds maximum allowed size of {} bytes",
                    MAX_PROMPT_BYTES
                );
            }
            parts.push(t);
        }
    }
    Ok(parts.join("\n"))
}

/// Extract image attachments from ACP prompt content blocks
pub fn extract_images_from_prompt(prompt: &[ContentBlock]) -> Vec<ha_core::agent::Attachment> {
    let mut attachments = Vec::new();
    for block in prompt {
        if let ContentBlock::Image {
            data: Some(data),
            mime_type,
        } = block
        {
            attachments.push(ha_core::agent::Attachment {
                name: "image".to_string(),
                mime_type: mime_type.clone().unwrap_or_else(|| "image/png".to_string()),
                source: None,
                data: Some(data.clone()),
                file_path: None,
                upload_id: None,
                quote_lines: None,
                quote_revealable: None,
                quote_role: None,
                quote_project_root: None,
                quote_worktree_root: None,
            });
        }
    }
    attachments
}

/// Infer tool kind from tool name
pub fn infer_tool_kind(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("read") {
        return "read";
    }
    if n.contains("write") || n.contains("edit") {
        return "edit";
    }
    if n.contains("delete") || n.contains("remove") {
        return "delete";
    }
    if n.contains("search") || n.contains("find") {
        return "search";
    }
    if n.contains("exec") || n.contains("run") || n.contains("bash") {
        return "execute";
    }
    if n.contains("fetch") || n.contains("http") {
        return "fetch";
    }
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_update_envelope_follows_negotiated_protocol() {
        let update = serde_json::json!({"sessionUpdate": "usage_update", "used": 7, "size": 10});
        assert_eq!(
            session_update_params(
                &AcpProtocolVersion::V1(ACP_PROTOCOL_VERSION_V1),
                "session-1",
                update.clone(),
            ),
            serde_json::json!({"sessionId": "session-1", "update": update})
        );
        assert_eq!(
            session_update_params(
                &AcpProtocolVersion::Legacy("0.2".into()),
                "session-1",
                serde_json::json!({"sessionUpdate": "usage_update", "used": 7, "size": 10}),
            ),
            serde_json::json!({
                "sessionId": "session-1",
                "sessionUpdate": {"sessionUpdate": "usage_update", "used": 7, "size": 10}
            })
        );
    }
}
