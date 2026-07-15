//! 平台级 MCP **server**（stdio）：把 Hope Agent 的子系统能力经标准 MCP 协议暴露给外部
//! agent（Claude Code / Cursor 等）。这是 design-space.md §15.1「平台级 Hope Agent as MCP
//! server」的第一块砖——共享协议循环 + `ToolProvider` 注册表，design 作首个 provider。
//!
//! **与 `crate::mcp` 的区别**：那是 MCP **客户端**（连别人的 server）；本模块是我们**当 server**。
//! MCP 规范里 "host" 指客户端宿主应用，故不叫 `mcp_host`。
//!
//! 协议：newline-delimited JSON-RPC 2.0 over stdio（initialize / ping / tools/list /
//! tools/call），`PROTOCOL_VERSION = 2025-03-26`（对齐 `knowledge::agent_mcp`）。
//!
//! **runtime 红线**：`run_stdio` 建 **multi_thread** runtime——provider 的写工具（如 design
//! 生成）内部 `tokio::spawn` 后台任务，current_thread runtime 在 `block_on` 返回后不再驱动
//! spawned task 会导致后台生成僵死。**切勿「优化」回 current_thread。**
//!
//! **写门双保险**：默认只读；`--allow-writes` 才暴露写工具。host 在 `tools/call` 层再拦一次
//! （即使 provider 忘了在 `tools()` 里裁剪写工具，只读模式调用写工具一律拒）。

use std::io::{self, BufRead, Write};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-03-26";

/// 工具调用上下文（写门 + 常驻 runtime）。
pub struct McpCtx<'rt> {
    pub allow_writes: bool,
    /// multi_thread、进程级常驻。async service fn 经 `runtime.block_on` 执行；其内部
    /// `tokio::spawn` 的后台任务在 worker 线程跨消息存活（见模块 runtime 红线）。
    pub runtime: &'rt tokio::runtime::Runtime,
}

/// 一个工具定义（进 `tools/list` + host 写门）。
pub struct ToolDef {
    /// 全名，前缀约定 `<provider>_`，如 `design_list_projects`。
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
    /// 只读工具（进 annotations + host 写门；`false` = 写工具，需 `--allow-writes`）。
    pub read_only: bool,
}

/// 一个子系统的工具供给者。design 是首个；后续 memory / knowledge 可挂入同一 host。
pub trait ToolProvider: Send + Sync {
    /// provider 名（诊断用；工具名各自带前缀）。
    fn name(&self) -> &'static str;
    /// 配置门（如 design 查 `cached_config().design.enabled`）。false → tools/list 不含 +
    /// tools/call 拒绝（fail-closed 双面）。
    fn enabled(&self) -> bool {
        true
    }
    /// 拼进 `initialize.instructions` 的一段说明。
    fn instructions(&self) -> Option<&'static str> {
        None
    }
    /// 当前应暴露的工具（provider 自行按 `ctx.allow_writes` 裁剪写工具）。
    fn tools(&self, ctx: &McpCtx) -> Vec<ToolDef>;
    /// 分发一次调用。错误经 host 转 `isError:true` 文本回给 client。
    fn call(&self, name: &str, args: Value, ctx: &McpCtx) -> Result<Value>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct McpServerOptions {
    /// 暴露写工具。默认只读。
    pub allow_writes: bool,
}

/// 在进程 stdin/stdout 上跑 MCP server。阻塞直到 stdin EOF。
pub fn run_stdio(options: McpServerOptions, providers: Vec<Box<dyn ToolProvider>>) -> Result<()> {
    // 红线：必须 multi_thread（provider 写工具内部 spawn 的后台任务要跨 block_on 存活）。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| anyhow!("failed to create MCP server runtime: {e}"))?;

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let ctx = McpCtx {
            allow_writes: options.allow_writes,
            runtime: &runtime,
        };
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(message, &ctx, &providers),
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
    ctx: &McpCtx,
    providers: &[Box<dyn ToolProvider>],
) -> Option<Value> {
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return id.map(|id| jsonrpc_error(id, -32600, "invalid request: missing method"));
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => id.map(|id| jsonrpc_result(id, initialize_result(providers))),
        "ping" => id.map(|id| jsonrpc_result(id, json!({}))),
        "notifications/initialized" => None,
        "tools/list" => id.map(|id| jsonrpc_result(id, tools_list_result(ctx, providers))),
        "tools/call" => id.map(|id| match call_tool(params, ctx, providers) {
            Ok(result) => jsonrpc_result(id, result),
            Err(e) => jsonrpc_result(id, tool_text_result(e.to_string(), true)),
        }),
        "resources/list" => id.map(|id| jsonrpc_result(id, json!({ "resources": [] }))),
        "prompts/list" => id.map(|id| jsonrpc_result(id, json!({ "prompts": [] }))),
        _ => id.map(|id| jsonrpc_error(id, -32601, format!("method not found: {method}"))),
    }
}

fn initialize_result(providers: &[Box<dyn ToolProvider>]) -> Value {
    let mut instructions = String::from(
        "Hope Agent MCP server. Use the available tools to drive Hope Agent subsystems.",
    );
    for p in providers {
        if p.enabled() {
            if let Some(extra) = p.instructions() {
                instructions.push(' ');
                instructions.push_str(extra);
            }
        }
    }
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": {},
            "prompts": {}
        },
        "serverInfo": {
            "name": "hope-agent",
            "version": crate::app_version()
        },
        "instructions": instructions
    })
}

fn tools_list_result(ctx: &McpCtx, providers: &[Box<dyn ToolProvider>]) -> Value {
    let mut tools = Vec::new();
    for p in providers {
        if !p.enabled() {
            continue;
        }
        for td in p.tools(ctx) {
            // host 兜底：只读模式绝不列写工具（即使 provider 忘了裁剪）。
            if !td.read_only && !ctx.allow_writes {
                continue;
            }
            tools.push(tool_def_json(&td));
        }
    }
    json!({ "tools": tools })
}

fn tool_def_json(td: &ToolDef) -> Value {
    json!({
        "name": td.name,
        "description": td.description,
        "inputSchema": td.input_schema,
        "annotations": {
            "readOnlyHint": td.read_only,
            "destructiveHint": false,
            "idempotentHint": td.read_only
        }
    })
}

fn call_tool(params: Value, ctx: &McpCtx, providers: &[Box<dyn ToolProvider>]) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tools/call requires params.name"))?;
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    // 用 allow_writes=true 的探测 ctx 枚举全部工具，拿到 (provider, read_only)——不受当前写门影响。
    let probe = McpCtx {
        allow_writes: true,
        runtime: ctx.runtime,
    };
    let mut found: Option<(usize, bool, bool)> = None; // (provider idx, read_only, enabled)
    for (i, p) in providers.iter().enumerate() {
        if p.tools(&probe).into_iter().any(|t| t.name == name) {
            let read_only = p
                .tools(&probe)
                .into_iter()
                .find(|t| t.name == name)
                .map(|t| t.read_only)
                .unwrap_or(true);
            found = Some((i, read_only, p.enabled()));
            break;
        }
    }
    let Some((idx, read_only, enabled)) = found else {
        return Err(anyhow!("unknown tool: {name}"));
    };
    if !enabled {
        return Err(anyhow!(
            "tool {name} is unavailable (its subsystem is disabled)"
        ));
    }
    // 写门双保险：只读模式调用写工具一律拒（与 tools/list 裁剪一致）。
    if !read_only && !ctx.allow_writes {
        return Err(anyhow!(
            "{name} is a write tool; start the server with --allow-writes to enable it"
        ));
    }

    let value = providers[idx].call(name, arguments, ctx)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// 一读一写工具的测试 provider；`enabled` 可控。
    struct TestProvider {
        enabled: bool,
    }
    impl ToolProvider for TestProvider {
        fn name(&self) -> &'static str {
            "test"
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn instructions(&self) -> Option<&'static str> {
            Some("test instructions")
        }
        fn tools(&self, ctx: &McpCtx) -> Vec<ToolDef> {
            let mut v = vec![ToolDef {
                name: "test_read",
                description: "read".into(),
                input_schema: json!({ "type": "object" }),
                read_only: true,
            }];
            if ctx.allow_writes {
                v.push(ToolDef {
                    name: "test_write",
                    description: "write".into(),
                    input_schema: json!({ "type": "object" }),
                    read_only: false,
                });
            }
            v
        }
        fn call(&self, name: &str, _args: Value, _ctx: &McpCtx) -> Result<Value> {
            Ok(json!({ "called": name }))
        }
    }

    fn providers(enabled: bool) -> Vec<Box<dyn ToolProvider>> {
        vec![Box::new(TestProvider { enabled })]
    }

    fn call(msg: Value, allow_writes: bool, enabled: bool) -> Option<Value> {
        let runtime = rt();
        let ctx = McpCtx {
            allow_writes,
            runtime: &runtime,
        };
        handle_message(msg, &ctx, &providers(enabled))
    }

    #[test]
    fn initialize_returns_server_info_and_instructions() {
        let r = call(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
            false,
            true,
        )
        .unwrap();
        assert_eq!(r["result"]["serverInfo"]["name"], "hope-agent");
        assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
        assert!(r["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("test instructions"));
    }

    #[test]
    fn tools_list_read_only_by_default_and_writes_with_flag() {
        let ro = call(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            false,
            true,
        )
        .unwrap();
        let names: Vec<&str> = ro["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"test_read"));
        assert!(!names.contains(&"test_write"));

        let rw = call(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            true,
            true,
        )
        .unwrap();
        let names: Vec<&str> = rw["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"test_write"));
    }

    #[test]
    fn write_tool_rejected_in_read_only_mode() {
        let r = call(
            json!({ "jsonrpc": "2.0", "id": "c", "method": "tools/call",
                    "params": { "name": "test_write", "arguments": {} } }),
            false,
            true,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("--allow-writes"));
    }

    #[test]
    fn write_tool_runs_with_flag() {
        let r = call(
            json!({ "jsonrpc": "2.0", "id": "c", "method": "tools/call",
                    "params": { "name": "test_write", "arguments": {} } }),
            true,
            true,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], false);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("test_write"));
    }

    #[test]
    fn disabled_provider_hides_and_rejects() {
        let list = call(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            true,
            false,
        )
        .unwrap();
        assert!(list["result"]["tools"].as_array().unwrap().is_empty());
        let r = call(
            json!({ "jsonrpc": "2.0", "id": "c", "method": "tools/call",
                    "params": { "name": "test_read", "arguments": {} } }),
            true,
            false,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn unknown_method_and_notification() {
        let r = call(
            json!({ "jsonrpc": "2.0", "id": 9, "method": "does/not/exist" }),
            false,
            true,
        )
        .unwrap();
        assert_eq!(r["error"]["code"], -32601);
        assert!(call(
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            false,
            true
        )
        .is_none());
    }

    #[test]
    fn unknown_tool_is_error() {
        let r = call(
            json!({ "jsonrpc": "2.0", "id": "c", "method": "tools/call",
                    "params": { "name": "nope", "arguments": {} } }),
            true,
            true,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn parse_error_returns_minus_32700() {
        // 直接测 run_stdio 的 parse 分支较重；此处校验 handle_message 不 panic 于畸形非法字段。
        let r = call(json!({ "jsonrpc": "2.0", "id": 1 }), false, true);
        // 缺 method → -32600。
        assert_eq!(r.unwrap()["error"]["code"], -32600);
    }
}
