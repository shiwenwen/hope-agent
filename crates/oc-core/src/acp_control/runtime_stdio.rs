//! ACP Control Plane — Stdio-based ACP runtime.
//!
//! Manages external ACP agent processes via stdin/stdout NDJSON (JSON-RPC 2.0).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};

use super::health;
use super::types::*;

/// Stdio-based ACP runtime — spawns an external ACP agent as a child process
/// and communicates over stdin/stdout using NDJSON (JSON-RPC 2.0).
pub struct StdioAcpRuntime {
    id: String,
    name: String,
    binary_path: String,
    acp_args: Vec<String>,
    env_overrides: HashMap<String, String>,
    /// Active child processes keyed by local session_id.
    children: Arc<Mutex<HashMap<String, ChildHandle>>>,
}

struct ChildHandle {
    child: Child,
    stdin_open: bool,
    external_session_id: Option<String>,
}

impl StdioAcpRuntime {
    pub fn new(
        id: String,
        name: String,
        binary_path: String,
        acp_args: Vec<String>,
        env_overrides: HashMap<String, String>,
    ) -> Self {
        Self {
            id,
            name,
            binary_path,
            acp_args,
            env_overrides,
            children: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn the child process in ACP mode.
    fn spawn_child(&self, cwd: Option<&str>) -> anyhow::Result<Child> {
        let mut cmd = Command::new(&self.binary_path);

        // If binary appears to support a known ACP sub-command, use it
        // Common patterns: `claude acp`, `codex --acp`, or custom args from config
        if self.acp_args.is_empty() {
            // Default: try the common pattern `<binary> acp`
            cmd.arg("acp");
        } else {
            cmd.args(&self.acp_args);
        }

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        // Environment: inherit + filter sensitive vars + apply overrides
        cmd.envs(&self.env_overrides);

        // Stdio: pipe all three
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Prevent the child from becoming a zombie
        #[cfg(unix)]
        {
            #[allow(unused_imports)]
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    // Create a new process group so we can kill the whole tree
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
        }

        let child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to spawn ACP backend '{}' ({}): {}",
                self.id,
                self.binary_path,
                e
            )
        })?;

        Ok(child)
    }

    /// Send a JSON-RPC request to the child's stdin and read the response from stdout.
    async fn send_request(
        child: &mut ChildHandle,
        method: &str,
        params: serde_json::Value,
        id: u64,
    ) -> anyhow::Result<serde_json::Value> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let stdin = child
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Child stdin closed"))?;

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;

        // Read response lines from stdout until we get a response with our ID
        let stdout = child
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Child stdout closed"))?;

        let mut reader = BufReader::new(stdout);
        let mut buf = String::new();

        loop {
            buf.clear();
            let n = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                reader.read_line(&mut buf),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Timeout waiting for ACP response"))?
            .map_err(|e| anyhow::anyhow!("Read error: {}", e))?;

            if n == 0 {
                return Err(anyhow::anyhow!("Child process closed stdout unexpectedly"));
            }

            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(trimmed) {
                // Check if this is a response (has "id" field matching ours)
                if let Some(resp_id) = msg.get("id").and_then(|v| v.as_u64()) {
                    if resp_id == id {
                        if let Some(error) = msg.get("error") {
                            let error_msg = error
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("Unknown error");
                            return Err(anyhow::anyhow!("ACP error: {}", error_msg));
                        }
                        return Ok(msg
                            .get("result")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null));
                    }
                }
                // Otherwise it's a notification — ignore during handshake
            }
        }
    }
}

#[async_trait]
impl AcpRuntime for StdioAcpRuntime {
    fn backend_id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    async fn is_available(&self) -> bool {
        tokio::fs::metadata(&self.binary_path).await.is_ok()
    }

    async fn get_version(&self) -> anyhow::Result<String> {
        let output = tokio::process::Command::new(&self.binary_path)
            .arg("--version")
            .output()
            .await?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(text)
    }

    async fn create_session(&self, params: AcpCreateParams) -> anyhow::Result<AcpExternalSession> {
        let session_id = uuid::Uuid::new_v4().to_string();

        let child = self.spawn_child(params.cwd.as_deref())?;
        let pid = child.id();

        let mut handle = ChildHandle {
            child,
            stdin_open: true,
            external_session_id: None,
        };

        // Step 1: initialize
        let _init_result = Self::send_request(
            &mut handle,
            "initialize",
            serde_json::json!({
                "protocolVersion": "0.2",
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                },
                "clientInfo": {
                    "name": "opencomputer-acp-control",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            1,
        )
        .await?;

        // Step 2: session/new
        let mut new_params = serde_json::json!({});
        if let Some(cwd) = &params.cwd {
            new_params["cwd"] = serde_json::json!(cwd);
        }
        if let Some(resume_id) = &params.resume_session_id {
            new_params["resumeSessionId"] = serde_json::json!(resume_id);
        }

        let session_result = Self::send_request(&mut handle, "session/new", new_params, 2).await?;

        let external_sid = session_result
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        handle.external_session_id = external_sid.clone();

        self.children
            .lock()
            .await
            .insert(session_id.clone(), handle);

        Ok(AcpExternalSession {
            session_id,
            backend_id: self.id.clone(),
            external_session_id: external_sid,
            pid,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn run_turn(
        &self,
        session: &AcpExternalSession,
        prompt: &str,
        event_tx: mpsc::Sender<AcpStreamEvent>,
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<AcpTurnResult> {
        let mut children = self.children.lock().await;
        let handle = children
            .get_mut(&session.session_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session.session_id))?;

        let ext_sid = handle.external_session_id.as_deref().unwrap_or("unknown");

        // Send session/prompt
        let prompt_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "session/prompt",
            "params": {
                "sessionId": ext_sid,
                "prompt": [{
                    "type": "user_message_chunk",
                    "content": prompt
                }]
            }
        });

        let stdin = handle
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stdin closed"))?;

        let mut line = serde_json::to_string(&prompt_request)?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;

        // Read events until we get the prompt response
        let stdout = handle
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stdout closed"))?;

        let mut reader = BufReader::new(stdout);
        let mut buf = String::new();
        let mut accumulated_text = String::new();
        let mut tool_calls = Vec::new();
        let mut total_input = 0u64;
        let mut total_output = 0u64;
        let mut stop_reason = "end_turn".to_string();

        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = event_tx
                    .send(AcpStreamEvent::Done {
                        stop_reason: "cancelled".into(),
                    })
                    .await;
                return Ok(AcpTurnResult {
                    stop_reason: "cancelled".into(),
                    response_text: accumulated_text,
                    input_tokens: Some(total_input),
                    output_tokens: Some(total_output),
                    tool_calls,
                });
            }

            buf.clear();
            let n = tokio::time::timeout(
                std::time::Duration::from_secs(600),
                reader.read_line(&mut buf),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Turn timed out after 600s"))??;

            if n == 0 {
                break; // EOF
            }

            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }

            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Check if this is the prompt response (id: 100)
            if msg.get("id").and_then(|v| v.as_u64()) == Some(100) {
                if let Some(result) = msg.get("result") {
                    stop_reason = result
                        .get("stopReason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("end_turn")
                        .to_string();
                }
                let _ = event_tx
                    .send(AcpStreamEvent::Done {
                        stop_reason: stop_reason.clone(),
                    })
                    .await;
                break;
            }

            // It's a notification — parse session/update
            if msg.get("method").and_then(|v| v.as_str()) == Some("session/update") {
                if let Some(params) = msg.get("params") {
                    if let Some(update) = params.get("sessionUpdate") {
                        let update_type = update
                            .get("sessionUpdate")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        match update_type {
                            "agent_message_chunk" => {
                                if let Some(text) = update
                                    .get("content")
                                    .and_then(|c| c.get("text"))
                                    .and_then(|t| t.as_str())
                                {
                                    accumulated_text.push_str(text);
                                    let _ = event_tx
                                        .send(AcpStreamEvent::TextDelta {
                                            content: text.to_string(),
                                        })
                                        .await;
                                }
                            }
                            "agent_thought_chunk" => {
                                if let Some(text) = update
                                    .get("content")
                                    .and_then(|c| c.get("text"))
                                    .and_then(|t| t.as_str())
                                {
                                    let _ = event_tx
                                        .send(AcpStreamEvent::ThinkingDelta {
                                            content: text.to_string(),
                                        })
                                        .await;
                                }
                            }
                            "tool_call" => {
                                let call_id = update
                                    .get("toolCallId")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let name = update
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let status = update
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("in_progress")
                                    .to_string();

                                let _ = event_tx
                                    .send(AcpStreamEvent::ToolCall {
                                        tool_call_id: call_id.clone(),
                                        name: name.clone(),
                                        status: status.clone(),
                                        arguments: None,
                                    })
                                    .await;

                                if status == "in_progress" {
                                    tool_calls.push(AcpToolCallSummary {
                                        name,
                                        status,
                                        duration_ms: None,
                                    });
                                }
                            }
                            "tool_call_update" => {
                                let call_id = update
                                    .get("toolCallId")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let status = update
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("completed")
                                    .to_string();
                                let preview = update
                                    .get("content")
                                    .and_then(|c| c.as_array())
                                    .and_then(|arr| arr.first())
                                    .and_then(|item| item.get("content"))
                                    .and_then(|c| c.get("text"))
                                    .and_then(|t| t.as_str())
                                    .map(|s| crate::truncate_utf8(s, 2048).to_string());

                                let _ = event_tx
                                    .send(AcpStreamEvent::ToolResult {
                                        tool_call_id: call_id,
                                        status,
                                        result_preview: preview,
                                    })
                                    .await;
                            }
                            "usage_update" => {
                                let input = update
                                    .get("inputTokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let output = update
                                    .get("outputTokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                total_input = input;
                                total_output = output;
                                let _ = event_tx
                                    .send(AcpStreamEvent::Usage {
                                        input_tokens: input,
                                        output_tokens: output,
                                    })
                                    .await;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(AcpTurnResult {
            stop_reason,
            response_text: accumulated_text,
            input_tokens: Some(total_input),
            output_tokens: Some(total_output),
            tool_calls,
        })
    }

    async fn cancel_turn(&self, session: &AcpExternalSession) -> anyhow::Result<()> {
        let mut children = self.children.lock().await;
        if let Some(handle) = children.get_mut(&session.session_id) {
            // Send SIGTERM to the process group
            if let Some(pid) = handle.child.id() {
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGTERM);
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = handle.child.kill().await;
                }
            }
        }
        Ok(())
    }

    async fn close_session(&self, session: &AcpExternalSession) -> anyhow::Result<()> {
        let mut children = self.children.lock().await;
        if let Some(mut handle) = children.remove(&session.session_id) {
            // Try graceful close first
            if handle.stdin_open {
                if let Some(stdin) = handle.child.stdin.as_mut() {
                    let close_req = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 999,
                        "method": "session/close",
                        "params": {
                            "sessionId": handle.external_session_id.as_deref().unwrap_or("")
                        }
                    });
                    let mut line = serde_json::to_string(&close_req).unwrap_or_default();
                    line.push('\n');
                    let _ = stdin.write_all(line.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
            }

            // Wait briefly, then force kill
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(3), handle.child.wait()).await;

            let _ = handle.child.kill().await;
        }
        Ok(())
    }

    fn capabilities(&self) -> AcpRuntimeCapabilities {
        AcpRuntimeCapabilities {
            supports_images: true,
            supports_thinking: true,
            supports_tool_approval: false,
            supports_session_resume: true,
            max_context_window: None,
        }
    }

    async fn health_check(&self) -> AcpHealthStatus {
        health::probe_binary(&self.binary_path).await
    }
}
