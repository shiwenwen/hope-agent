//! Phase 7 MCP stdio facade for Knowledge Space.
//!
//! This module intentionally implements only the small server surface external
//! agents need: initialize, tools/list, and tools/call over newline-delimited
//! JSON-RPC on stdio. The actual knowledge behavior stays in [`super::agent_api`].

use std::io::{self, BufRead, Write};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::agent_api;
use super::types::{
    KnowledgeAgentCompileProposeInput, KnowledgeAgentExpandInput, KnowledgeAgentReadInput,
    KnowledgeAgentSearchInput, KnowledgeAgentSourcesInput,
};

const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, Clone, Copy, Default)]
pub struct KnowledgeMcpOptions {
    /// Expose `knowledge_compile_propose`. The default MCP surface is read-only.
    pub allow_proposals: bool,
}

/// Run the Knowledge Space MCP server over process stdin/stdout.
pub fn run_stdio(options: KnowledgeMcpOptions) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow!("failed to create MCP runtime: {e}"))?;
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(message, options, Some(&runtime)),
            Err(e) => Some(jsonrpc_error(
                Value::Null,
                -32700,
                format!("parse error: {e}"),
            )),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

pub fn handle_message(
    message: Value,
    options: KnowledgeMcpOptions,
    runtime: Option<&tokio::runtime::Runtime>,
) -> Option<Value> {
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return id.map(|id| jsonrpc_error(id, -32600, "invalid request: missing method"));
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => id.map(|id| jsonrpc_result(id, initialize_result())),
        "ping" => id.map(|id| jsonrpc_result(id, json!({}))),
        "notifications/initialized" => None,
        "tools/list" => id.map(|id| jsonrpc_result(id, tools_list_result(options))),
        "tools/call" => id.map(|id| match call_tool(params, options, runtime) {
            Ok(result) => jsonrpc_result(id, result),
            Err(e) => jsonrpc_result(id, tool_text_result(e.to_string(), true)),
        }),
        "resources/list" => id.map(|id| jsonrpc_result(id, json!({ "resources": [] }))),
        "prompts/list" => id.map(|id| jsonrpc_result(id, json!({ "prompts": [] }))),
        _ => id.map(|id| jsonrpc_error(id, -32601, format!("method not found: {method}"))),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": {},
            "prompts": {}
        },
        "serverInfo": {
            "name": "hope-agent-knowledge",
            "version": crate::app_version()
        },
        "instructions": "Use the knowledge_* tools to read Hope Agent Knowledge Space notes. Raw sources are returned only when explicitly requested."
    })
}

fn tools_list_result(options: KnowledgeMcpOptions) -> Value {
    let mut tools = vec![
        tool_def(
            "knowledge_search",
            "Search Knowledge Space wiki notes. Raw source hits are included only when includeSources is true and kbId is provided.",
            search_schema(),
            true,
        ),
        tool_def(
            "knowledge_read",
            "Read one Knowledge Space note by kbId plus path or [[reference]].",
            read_schema(),
            true,
        ),
        tool_def(
            "knowledge_expand",
            "Read one note and return related notes from the same knowledge base.",
            expand_schema(),
            true,
        ),
        tool_def(
            "knowledge_sources",
            "List/search raw source metadata or read one explicit raw source when sourceId is provided.",
            sources_schema(),
            true,
        ),
    ];
    if options.allow_proposals {
        tools.push(tool_def(
            "knowledge_compile_propose",
            "Start a compile run that creates Review Diff proposals. It never applies .md writes by itself.",
            compile_propose_schema(),
            false,
        ));
    }
    json!({ "tools": tools })
}

fn tool_def(name: &str, description: &str, input_schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only
        }
    })
}

fn call_tool(
    params: Value,
    options: KnowledgeMcpOptions,
    runtime: Option<&tokio::runtime::Runtime>,
) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tools/call requires params.name"))?;
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    let value = match name {
        "knowledge_search" => {
            let input: KnowledgeAgentSearchInput = serde_json::from_value(arguments)?;
            serde_json::to_value(agent_api::search(input)?)?
        }
        "knowledge_read" => {
            let input: KnowledgeAgentReadInput = serde_json::from_value(arguments)?;
            serde_json::to_value(agent_api::read(input)?)?
        }
        "knowledge_expand" => {
            let input: KnowledgeAgentExpandInput = serde_json::from_value(arguments)?;
            serde_json::to_value(agent_api::expand(input)?)?
        }
        "knowledge_sources" => {
            let input: KnowledgeAgentSourcesInput = serde_json::from_value(arguments)?;
            serde_json::to_value(agent_api::sources(input)?)?
        }
        "knowledge_compile_propose" if options.allow_proposals => {
            let runtime =
                runtime.ok_or_else(|| anyhow!("knowledge_compile_propose requires runtime"))?;
            let input: KnowledgeAgentCompileProposeInput = serde_json::from_value(arguments)?;
            serde_json::to_value(runtime.block_on(agent_api::compile_propose(input))?)?
        }
        "knowledge_compile_propose" => {
            return Err(anyhow!(
                "knowledge_compile_propose is disabled; start with --allow-proposals to expose it"
            ));
        }
        _ => return Err(anyhow!("unknown knowledge MCP tool: {name}")),
    };
    Ok(tool_text_result(
        serde_json::to_string_pretty(&value)?,
        false,
    ))
}

fn tool_text_result(text: String, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn jsonrpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

fn search_schema() -> Value {
    json!({
        "type": "object",
        "required": ["query"],
        "properties": {
            "query": { "type": "string" },
            "kbId": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
            "includeSources": { "type": "boolean", "default": false }
        }
    })
}

fn read_schema() -> Value {
    json!({
        "type": "object",
        "required": ["kbId"],
        "properties": {
            "kbId": { "type": "string" },
            "path": { "type": "string" },
            "reference": { "type": "string" },
            "includeSourceRefs": { "type": "boolean", "default": true }
        }
    })
}

fn expand_schema() -> Value {
    json!({
        "type": "object",
        "required": ["kbId", "path"],
        "properties": {
            "kbId": { "type": "string" },
            "path": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 25 }
        }
    })
}

fn sources_schema() -> Value {
    json!({
        "type": "object",
        "required": ["kbId"],
        "properties": {
            "kbId": { "type": "string" },
            "sourceId": { "type": "string" },
            "query": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
            "includeContent": { "type": "boolean", "default": false }
        }
    })
}

fn compile_propose_schema() -> Value {
    json!({
        "type": "object",
        "required": ["kbId", "sourceIds"],
        "properties": {
            "kbId": { "type": "string" },
            "sourceIds": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1
            },
            "strategy": { "type": "string" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_returns_server_info() {
        let response = handle_message(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
            KnowledgeMcpOptions::default(),
            None,
        )
        .expect("response");
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "hope-agent-knowledge"
        );
        assert_eq!(
            response["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
    }

    #[test]
    fn tools_list_is_read_only_by_default() {
        let response = handle_message(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            KnowledgeMcpOptions::default(),
            None,
        )
        .expect("response");
        let tools = response["result"]["tools"].as_array().expect("tools");
        let names = tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"knowledge_search"));
        assert!(names.contains(&"knowledge_sources"));
        assert!(!names.contains(&"knowledge_compile_propose"));
    }

    #[test]
    fn tools_list_can_expose_compile_propose() {
        let response = handle_message(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            KnowledgeMcpOptions {
                allow_proposals: true,
            },
            None,
        )
        .expect("response");
        let tools = response["result"]["tools"].as_array().expect("tools");
        assert!(tools
            .iter()
            .any(|t| t["name"].as_str() == Some("knowledge_compile_propose")));
    }

    #[test]
    fn notification_initialized_has_no_response() {
        let response = handle_message(
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            KnowledgeMcpOptions::default(),
            None,
        );
        assert!(response.is_none());
    }

    #[test]
    fn disabled_compile_tool_returns_tool_error() {
        let response = handle_message(
            json!({
                "jsonrpc": "2.0",
                "id": "call-1",
                "method": "tools/call",
                "params": {
                    "name": "knowledge_compile_propose",
                    "arguments": { "kbId": "kb", "sourceIds": ["src"] }
                }
            }),
            KnowledgeMcpOptions::default(),
            None,
        )
        .expect("response");
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("disabled"));
    }
}
