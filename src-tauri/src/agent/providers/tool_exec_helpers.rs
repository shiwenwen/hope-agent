//! Shared helpers for tool execution across all providers.
//! Extracted to eliminate duplication of logging and cancel-watcher patterns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::super::api_types::FunctionCallItem;
use crate::tools::{self, ToolExecContext};

/// Log tool execution input.
pub(super) fn log_tool_input(tc: &FunctionCallItem, round: u32) {
    if let Some(logger) = crate::get_logger() {
        let args_str = tc.arguments.as_str();
        let args_preview = if args_str.len() > 2048 {
            format!(
                "{}...(truncated, total {}B)",
                crate::truncate_utf8(args_str, 2048),
                args_str.len()
            )
        } else {
            args_str.to_string()
        };
        logger.log(
            "debug",
            "agent",
            "agent::tool_exec::input",
            &format!("Tool exec [{}] id={}", tc.name, tc.call_id),
            Some(
                json!({
                    "tool_name": tc.name,
                    "call_id": tc.call_id,
                    "arguments": args_preview,
                    "round": round,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

/// Log tool execution output.
pub(super) fn log_tool_output(
    call_id: &str,
    name: &str,
    result: &str,
    elapsed_ms: u64,
    round: u32,
) {
    if let Some(logger) = crate::get_logger() {
        let result_preview = if result.len() > 2048 {
            format!(
                "{}...(truncated, total {}B)",
                crate::truncate_utf8(result, 2048),
                result.len()
            )
        } else {
            result.to_string()
        };
        let is_error = result.starts_with("Tool error:");
        logger.log(
            if is_error { "warn" } else { "debug" },
            "agent",
            "agent::tool_exec::output",
            &format!(
                "Tool result [{}] {}B, {}ms{}",
                name,
                result.len(),
                elapsed_ms,
                if is_error { " (ERROR)" } else { "" }
            ),
            Some(
                json!({
                    "tool_name": name,
                    "call_id": call_id,
                    "result_size_bytes": result.len(),
                    "elapsed_ms": elapsed_ms,
                    "is_error": is_error,
                    "result_preview": result_preview,
                    "round": round,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }
}

/// Execute a tool with cancel-flag racing.
/// Returns (result_string, elapsed_ms).
pub(super) async fn execute_tool_with_cancel(
    name: &str,
    args: &serde_json::Value,
    ctx: &ToolExecContext,
    cancel: &Arc<AtomicBool>,
) -> (String, u64) {
    let tool_start = std::time::Instant::now();
    let cancel_clone = cancel.clone();
    let result = tokio::select! {
        res = tools::execute_tool_with_context(name, args, ctx) => {
            match res {
                Ok(r) => r,
                Err(e) => format!("Tool error: {}", e),
            }
        }
        _ = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if cancel_clone.load(Ordering::SeqCst) { break; }
            }
        } => {
            String::from("Tool execution cancelled by user")
        }
    };
    let elapsed_ms = tool_start.elapsed().as_millis() as u64;
    (result, elapsed_ms)
}
