//! MCP 配置 wire 类型再导出（定义在 ha-config-schema，`crate::mcp::
//! config::*` 既有路径不变）。保存期校验 `validate_server_config`、名称
//! 规则与占位符展开等子系统逻辑在 ha-mcp 的 `config` 模块。

pub use ha_config_schema::mcp::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};
