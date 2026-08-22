//! Hope Agent 自带 MCP 服务端共用的协议协商与 2026 结果封装。
//!
//! 两个 stdio 服务端都必须走这里，避免协议版本、发现结果和
//! `resultType` 兼容规则再次漂移。

use serde_json::{json, Value};

pub const MCP_PROTOCOL_2026_07_28: &str = "2026-07-28";
pub const MCP_PROTOCOL_2025_11_25: &str = "2025-11-25";
pub const MCP_PROTOCOL_2025_06_18: &str = "2025-06-18";
pub const MCP_PROTOCOL_2025_03_26: &str = "2025-03-26";

pub const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    MCP_PROTOCOL_2026_07_28,
    MCP_PROTOCOL_2025_11_25,
    MCP_PROTOCOL_2025_06_18,
    MCP_PROTOCOL_2025_03_26,
];

/// 单条 stdio 连接的协商状态。未初始化时按最新协议生成结果；收到
/// `initialize` 后，精确接受受支持版本，未知版本回落到最新版本。
#[derive(Debug, Clone, Copy)]
pub struct McpProtocolSession {
    negotiated_version: &'static str,
}

impl Default for McpProtocolSession {
    fn default() -> Self {
        Self {
            negotiated_version: MCP_PROTOCOL_2026_07_28,
        }
    }
}

impl McpProtocolSession {
    pub fn negotiate_initialize(&mut self, params: &Value) -> &'static str {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        self.negotiated_version = requested
            .and_then(|requested| {
                MCP_SUPPORTED_PROTOCOL_VERSIONS
                    .iter()
                    .copied()
                    .find(|supported| *supported == requested)
            })
            .unwrap_or(MCP_PROTOCOL_2026_07_28);
        self.negotiated_version
    }

    pub fn negotiated_version(&self) -> &'static str {
        self.negotiated_version
    }

    pub fn complete_result(&self, mut result: Value) -> Value {
        let Some(object) = result.as_object_mut() else {
            return result;
        };
        if self.negotiated_version == MCP_PROTOCOL_2026_07_28 {
            object.insert("resultType".into(), Value::String("complete".into()));
        } else {
            object.remove("resultType");
        }
        result
    }
}

pub fn discover_result(
    capabilities: Value,
    server_name: &str,
    server_version: &str,
    instructions: &str,
) -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": MCP_SUPPORTED_PROTOCOL_VERSIONS,
        "capabilities": capabilities,
        "instructions": instructions,
        "ttlMs": 0,
        "cacheScope": "private",
        "_meta": {
            "io.modelcontextprotocol/serverInfo": {
                "name": server_name,
                "version": server_version
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_legacy_version_is_preserved_and_omits_result_type() {
        let mut session = McpProtocolSession::default();
        assert_eq!(
            session.negotiate_initialize(&json!({
                "protocolVersion": MCP_PROTOCOL_2025_03_26
            })),
            MCP_PROTOCOL_2025_03_26
        );
        assert!(session
            .complete_result(json!({}))
            .get("resultType")
            .is_none());
    }

    #[test]
    fn latest_and_unknown_versions_use_2026_result_discriminator() {
        let mut session = McpProtocolSession::default();
        assert_eq!(
            session.negotiate_initialize(&json!({ "protocolVersion": "2099-01-01" })),
            MCP_PROTOCOL_2026_07_28
        );
        assert_eq!(session.complete_result(json!({}))["resultType"], "complete");
    }
}
