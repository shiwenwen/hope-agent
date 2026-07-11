use anyhow::Result;
use serde_json::Value;

use super::ToolExecContext;

pub(crate) async fn tool_lsp(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    crate::lsp::tool_lsp(args, ctx).await
}
