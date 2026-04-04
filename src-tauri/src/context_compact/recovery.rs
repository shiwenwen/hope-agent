// ── Post-Compaction File Recovery ──
//
// After Tier 3 LLM summarization, recently written/edited files' precise
// contents are lost from the conversation history. This module scans the
// summarized messages for file-modifying tool calls, reads the current
// disk content of the most recently modified files, and injects a
// synthetic recovery message so the model can continue editing without
// an extra read tool round.
//
// Reference: claude-code `createPostCompactFileAttachments()`.

use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

use super::config::CompactConfig;

/// Tool names that modify files on disk.
/// Primary names reference constants from `crate::tools`; aliases match the dispatcher.
const FILE_WRITE_TOOLS: &[&str] = &[
    crate::tools::TOOL_WRITE,       // "write"
    "write_file",                    // alias accepted by dispatcher
    crate::tools::TOOL_EDIT,         // "edit"
    "patch_file",                    // alias accepted by dispatcher
    crate::tools::TOOL_APPLY_PATCH,  // "apply_patch"
];

/// Max total bytes for all recovery content (~25K tokens).
const MAX_RECOVERY_TOTAL_BYTES: usize = 100_000;

/// Build a recovery message containing current disk contents of recently-edited files.
///
/// Returns `None` if no files need recovery (no writes, all in preserved, budget too small).
///
/// - `summarized_messages`: messages that were replaced by the summary
/// - `preserved_messages`: messages kept after the summary
/// - `tokens_freed`: approximate tokens freed by summarization
/// - `config`: compaction config for recovery settings
pub fn build_recovery_message(
    summarized_messages: &[Value],
    preserved_messages: &[Value],
    tokens_freed: u32,
    config: &CompactConfig,
) -> Option<Value> {
    if !config.recovery_enabled {
        return None;
    }

    let max_files = config.recovery_max_files.min(10).max(1);
    let max_file_bytes = config.recovery_max_file_bytes;
    let max_total_bytes = MAX_RECOVERY_TOTAL_BYTES;

    // Budget: 10% of freed tokens, converted to bytes (~4 bytes/token), capped
    let byte_budget = ((tokens_freed as usize).saturating_mul(4) / 10).min(max_total_bytes);

    if byte_budget < 500 {
        return None;
    }

    // Extract file paths from write/edit tool calls in summarized messages
    let written_paths = extract_written_file_paths(summarized_messages);
    if written_paths.is_empty() {
        return None;
    }

    // Dedup against files already referenced in preserved messages
    let preserved_paths = extract_written_file_paths(preserved_messages);
    let preserved_set: HashSet<&str> = preserved_paths.iter().map(|s| s.as_str()).collect();
    let candidates: Vec<&String> = written_paths
        .iter()
        .filter(|p| !preserved_set.contains(p.as_str()))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Take most recent files (last in list = most recently written)
    let recent: Vec<&String> = candidates.iter().rev().take(max_files).copied().collect();

    let mut recovery_parts: Vec<String> = Vec::new();
    let mut total_chars: usize = 0;

    for path in &recent {
        if total_chars >= byte_budget {
            break;
        }

        let abs_path = if Path::new(path.as_str()).is_absolute() {
            path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path.as_str()).to_string_lossy().to_string(),
                Err(_) => path.to_string(),
            }
        };

        match std::fs::read_to_string(&abs_path) {
            Ok(content) => {
                let truncated = if content.len() > max_file_bytes {
                    format!(
                        "{}...\n[truncated, {} total bytes]",
                        crate::truncate_utf8(&content, max_file_bytes),
                        content.len()
                    )
                } else {
                    content
                };

                let overhead = path.len() + 40; // <file path="..."> tags
                if total_chars + truncated.len() + overhead > byte_budget {
                    break;
                }

                recovery_parts.push(format!(
                    "<file path=\"{}\">\n{}\n</file>",
                    path, truncated
                ));
                total_chars += truncated.len() + overhead;
            }
            Err(_) => {
                // File may have been deleted, moved, or is binary — skip silently
                continue;
            }
        }
    }

    if recovery_parts.is_empty() {
        return None;
    }

    let content = format!(
        "[Post-compaction file recovery: current contents of recently-edited files]\n\n{}",
        recovery_parts.join("\n\n")
    );

    Some(serde_json::json!({
        "role": "user",
        "content": content
    }))
}

/// Extract file paths from write/edit/apply_patch tool calls in messages.
/// Returns paths in order of first appearance (deduped). The caller uses
/// `.rev().take(N)` to select the most recently appearing paths.
fn extract_written_file_paths(messages: &[Value]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for msg in messages {
        let tool_calls = extract_tool_calls_from_message(msg);
        for (name, args) in tool_calls {
            if !FILE_WRITE_TOOLS.contains(&name.as_str()) {
                continue;
            }

            let extracted = if name == crate::tools::TOOL_APPLY_PATCH {
                extract_paths_from_patch_args(&args)
            } else {
                extract_path_from_write_edit_args(&args)
            };

            for path in extracted {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
    }

    paths
}

/// Extract (tool_name, arguments) pairs from a message, format-agnostic.
fn extract_tool_calls_from_message(msg: &Value) -> Vec<(String, Value)> {
    let mut calls = Vec::new();

    // Anthropic: assistant message with content array containing tool_use blocks
    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let (Some(name), Some(input)) = (
                    block.get("name").and_then(|n| n.as_str()),
                    block.get("input"),
                ) {
                    calls.push((name.to_string(), input.clone()));
                }
            }
        }
    }

    // OpenAI Chat: assistant message with tool_calls array
    if let Some(tool_calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        for tc in tool_calls {
            if let (Some(name), Some(args_str)) = (
                tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str()),
                tc.get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str()),
            ) {
                if let Ok(args) = serde_json::from_str::<Value>(args_str) {
                    calls.push((name.to_string(), args));
                }
            }
        }
    }

    // OpenAI Responses: type=function_call
    if msg.get("type").and_then(|t| t.as_str()) == Some("function_call") {
        if let (Some(name), Some(args_str)) = (
            msg.get("name").and_then(|n| n.as_str()),
            msg.get("arguments").and_then(|a| a.as_str()),
        ) {
            if let Ok(args) = serde_json::from_str::<Value>(args_str) {
                calls.push((name.to_string(), args));
            }
        }
    }

    calls
}

/// Extract path from write/edit tool arguments (tries "path" then "file_path").
fn extract_path_from_write_edit_args(args: &Value) -> Vec<String> {
    args.get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|p| p.as_str())
        .map(|s| vec![s.to_string()])
        .unwrap_or_default()
}

/// Extract paths from apply_patch arguments by parsing patch header lines.
fn extract_paths_from_patch_args(args: &Value) -> Vec<String> {
    let input = match args.get("input").and_then(|i| i.as_str()) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut paths = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        // Match patch header formats:
        // "*** Add File: <path>"
        // "*** Update File: <path>"
        // "*** Move to: <path>"
        if let Some(rest) = trimmed.strip_prefix("*** Add File:") {
            paths.push(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("*** Update File:") {
            paths.push(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("*** Move to:") {
            paths.push(rest.trim().to_string());
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_anthropic_write() {
        let msg = json!({
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "tc1",
                    "name": "write",
                    "input": { "path": "/tmp/test.rs", "content": "fn main() {}" }
                }
            ]
        });
        let paths = extract_written_file_paths(&[msg]);
        assert_eq!(paths, vec!["/tmp/test.rs"]);
    }

    #[test]
    fn test_extract_openai_chat_edit() {
        let msg = json!({
            "role": "assistant",
            "tool_calls": [{
                "id": "tc1",
                "type": "function",
                "function": {
                    "name": "edit",
                    "arguments": "{\"file_path\": \"/tmp/test.rs\", \"old_string\": \"a\", \"new_string\": \"b\"}"
                }
            }]
        });
        let paths = extract_written_file_paths(&[msg]);
        assert_eq!(paths, vec!["/tmp/test.rs"]);
    }

    #[test]
    fn test_extract_responses_function_call() {
        let msg = json!({
            "type": "function_call",
            "call_id": "fc1",
            "name": "write_file",
            "arguments": "{\"path\": \"/tmp/new.ts\"}"
        });
        let paths = extract_written_file_paths(&[msg]);
        assert_eq!(paths, vec!["/tmp/new.ts"]);
    }

    #[test]
    fn test_extract_apply_patch() {
        let msg = json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "tc1",
                "name": "apply_patch",
                "input": {
                    "input": "*** Add File: /tmp/a.rs\n+line1\n*** Update File: /tmp/b.rs\n@@ -1,1 +1,1 @@\n-old\n+new"
                }
            }]
        });
        let paths = extract_written_file_paths(&[msg]);
        assert_eq!(paths, vec!["/tmp/a.rs", "/tmp/b.rs"]);
    }

    #[test]
    fn test_dedup_paths() {
        let msgs = vec![
            json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use", "id": "tc1", "name": "write",
                    "input": { "path": "/tmp/a.rs" }
                }]
            }),
            json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use", "id": "tc2", "name": "edit",
                    "input": { "path": "/tmp/a.rs", "old_string": "x", "new_string": "y" }
                }]
            }),
        ];
        let paths = extract_written_file_paths(&msgs);
        // Deduplicated: only one entry
        assert_eq!(paths, vec!["/tmp/a.rs"]);
    }

    #[test]
    fn test_non_write_tools_ignored() {
        let msg = json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use", "id": "tc1", "name": "read_file",
                "input": { "path": "/tmp/test.rs" }
            }]
        });
        let paths = extract_written_file_paths(&[msg]);
        assert!(paths.is_empty());
    }
}
