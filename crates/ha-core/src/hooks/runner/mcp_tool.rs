//! `mcp_tool` hook handler — invokes an MCP tool and returns its result as the
//! hook's stdout (design §7.4).
//!
//! The tool result rides the normal output parser: it becomes
//! `additionalContext` for the plaintext-accepting events (SessionStart /
//! UserPromptSubmit) or, for any event, when the tool emits the JSON protocol
//! envelope. A failed / unavailable tool is a non-blocking error (never blocks
//! the host path). Bounded by the handler deadline like every other handler.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;

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
        format!("{}|{}", self.config.server, self.config.tool)
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
        let args = self.config.input.clone().unwrap_or_else(|| json!({}));

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
        match tokio::time::timeout(remaining, crate::mcp::invoke::call_tool(&name, &args, &ctx)).await
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
