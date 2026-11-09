//! In-memory JSON-RPC -> rmcp -> wired MCP hook runner -> HookOutput parser.
//! No subprocess, network, runtime/database initialization or user data.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::hooks::runner::mcp_tool::McpToolHandler;
use ha_core::hooks::runner::HookHandler;
use ha_core::hooks::{CommonHookInput, HookDecision, HookEvent, HookInput, PermissionMode};
use ha_mcp::config::{McpGlobalSettings, McpServerConfig};
use ha_mcp::registry::{McpManager, ServerState};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn structured_hook_decisions_survive_wire_dispatch_and_parsing() {
    tokio::time::timeout(Duration::from_secs(10), run_fixture())
        .await
        .expect("bounded in-memory fixture");
}

async fn run_fixture() {
    let (client, server) = tokio::io::duplex(65_536);
    let server_task = tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let Some(id) = request.get("id") else {
                continue;
            };
            let result = match request["method"].as_str().unwrap() {
                "initialize" => json!({
                    "protocolVersion": request["params"]["protocolVersion"],
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "in-memory-hook", "version": "1"}
                }),
                "tools/call" => request["params"]["arguments"]["result"].clone(),
                other => panic!("unexpected fixture request: {other}"),
            };
            let response = json!({"jsonrpc": "2.0", "id": id, "result": result});
            write
                .write_all(response.to_string().as_bytes())
                .await
                .unwrap();
            write.write_all(b"\n").await.unwrap();
        }
    });
    let running = ().serve(client).await.unwrap();
    let cfg: McpServerConfig = serde_json::from_value(json!({
        "id": "be5264bb-d0c8-4a96-9159-39741ccbd8aa",
        "name": "hook_fixture",
        "transport": {"kind": "stdio", "command": "never-spawned"},
        "callTimeoutSecs": 5
    }))
    .unwrap();
    let manager = McpManager::init_global(McpGlobalSettings::default(), vec![cfg.clone()]);
    let handle = manager.get_by_id(&cfg.id).await.unwrap();
    *handle.client.lock().await = Some(running);
    *handle.state.lock().await = ServerState::Ready {
        tools: vec![rmcp::model::Tool::new(
            "scan",
            "fixture",
            Arc::new(Default::default()),
        )],
        resources: vec![],
        prompts: vec![],
    };
    manager
        .reconcile(McpGlobalSettings::default(), vec![cfg])
        .await
        .unwrap();
    ha_mcp::wire();
    let input = HookInput::PreToolUse {
        common: CommonHookInput {
            session_id: String::new(),
            prompt_id: None,
            transcript_path: Default::default(),
            cwd: Default::default(),
            permission_mode: PermissionMode::Default,
            effort: None,
            // The protocol consumer must follow the typed variant even when
            // the common display field disagrees.
            hook_event_name: "PermissionRequest".into(),
            agent_id: None,
            agent_type: None,
        },
        tool_name: "exec".into(),
        tool_input: json!({"command": "synthetic"}),
        tool_use_id: "fixture".into(),
    };
    for (payload, expected) in [
        (
            json!({"hookSpecificOutput": {"permissionDecision": "deny", "permissionDecisionReason": "blocked"}}),
            HookDecision::Deny {
                reason: "blocked".into(),
            },
        ),
        (
            json!({"decision": "block", "reason": "blocked"}),
            HookDecision::Block {
                reason: "blocked".into(),
            },
        ),
        (
            json!({"hookSpecificOutput": {"permissionDecision": "ask"}}),
            HookDecision::Ask,
        ),
        (
            json!({"continue": false, "stopReason": "halt"}),
            HookDecision::Allow,
        ),
    ] {
        let mut responses = vec![
            json!({"structuredContent": payload, "_meta": {"private": "excluded"}}),
            serde_json::to_value(rmcp::model::CallToolResult::structured(payload.clone())).unwrap(),
            json!({"content": [{"type": "text", "text": payload.to_string()}]}),
        ];
        for supplemental in [
            json!({"risk": "high"}),
            Value::Null,
            json!(false),
            json!(42),
            json!([]),
            json!({}),
            json!({"hookSpecificOutput": {"risk": "high"}}),
            json!({"continue": "invalid type"}),
            json!({"hookSpecificOutput": {"additionalContext": "supplemental context"}}),
            json!({"hookSpecificOutput": {"permissionDecision": "allow"}}),
        ] {
            responses.push(json!({
                "content": [{"type": "text", "text": payload.to_string()}],
                "structuredContent": supplemental
            }));
        }
        for wire in responses {
            let config = serde_json::from_value(json!({
                "server": "hook_fixture", "tool": "scan", "input": {"result": wire}
            }))
            .unwrap();
            let raw = McpToolHandler::new(config)
                .run(
                    &input,
                    &ha_core::hooks::env::HookEnv::empty(),
                    Instant::now() + Duration::from_secs(5),
                )
                .await;
            assert_eq!(raw.exit_code, Some(0), "{}", raw.stderr);
            assert!(serde_json::from_str::<Value>(&raw.stdout)
                .unwrap()
                .is_object());
            assert!(!raw.stdout.contains("excluded"));
            let parsed = ha_core::hooks::parse::parse(&raw, HookEvent::PreToolUse);
            assert_eq!(parsed.decision, expected);
            assert!(!parsed.permission_allow);
            if payload.get("continue") == Some(&json!(false)) {
                assert!(!parsed.continue_execution);
                assert_eq!(parsed.stop_reason.as_deref(), Some("halt"));
            }
        }
    }
    let allow = json!({"hookSpecificOutput": {"permissionDecision": "allow"}});
    for wire in [
        json!({"structuredContent": allow}),
        serde_json::to_value(rmcp::model::CallToolResult::structured(allow.clone())).unwrap(),
        json!({"content": [{"type": "text", "text": allow.to_string()}]}),
    ] {
        let config = serde_json::from_value(json!({
            "server": "hook_fixture", "tool": "scan", "input": {"result": wire}
        }))
        .unwrap();
        let raw = McpToolHandler::new(config)
            .run(
                &input,
                &ha_core::hooks::env::HookEnv::empty(),
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        assert_eq!(raw.exit_code, Some(0));
        let parsed = ha_core::hooks::parse::parse(&raw, HookEvent::PreToolUse);
        assert!(parsed.permission_allow);
    }
    for normal in [
        json!({}),
        json!({"hookSpecificOutput": {"additionalContext": "retain normal approval"}}),
        json!({"hookSpecificOutput": {"updatedInput": {"command": "safe rewrite"}}}),
        json!({"hookSpecificOutput": {}}),
        json!({"hookSpecificOutput": null}),
        json!({"hookSpecificOutput": {"permissionDecision": null, "additionalContext": null, "updatedInput": null}}),
        json!({"decision": null, "stopReason": null, "reason": null, "terminalSequence": null, "systemMessage": null}),
        json!({"hookSpecificOutput": {"risk": "unknown"}}),
    ] {
        for (legacy, structured) in [(&normal, &allow), (&allow, &normal)] {
            let config = serde_json::from_value(json!({
                "server": "hook_fixture", "tool": "scan", "input": {"result": {
                    "content": [{"type": "text", "text": legacy.to_string()}],
                    "structuredContent": structured
                }}
            }))
            .unwrap();
            let raw = McpToolHandler::new(config)
                .run(
                    &input,
                    &ha_core::hooks::env::HookEnv::empty(),
                    Instant::now() + Duration::from_secs(5),
                )
                .await;
            assert_eq!(raw.exit_code, Some(0));
            let parsed = ha_core::hooks::parse::parse(&raw, HookEvent::PreToolUse);
            assert!(
                !parsed.permission_allow,
                "normal approval must not become explicit allow"
            );
            assert_eq!(parsed.decision, HookDecision::Allow);
            assert_eq!(
                parsed.updated_input,
                normal["hookSpecificOutput"]
                    .get("updatedInput")
                    .filter(|value| !value.is_null())
                    .cloned()
            );
            assert_eq!(
                parsed.additional_context.as_deref(),
                normal["hookSpecificOutput"]["additionalContext"].as_str()
            );
        }
    }
    let wire = json!({"content": [{"type": "text", "text": "human fallback"}], "structuredContent": {"answer": 42}});
    let body = ha_core::mcp::invoke::call_tool(
        "mcp__hook_fixture__scan",
        &json!({"result": wire}),
        &Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        body,
        "human fallback\n\n[structuredContent]\n{\"answer\":42}"
    );
    ha_mcp::client::disconnect(&handle).await.unwrap();
    server_task.await.unwrap();
}
