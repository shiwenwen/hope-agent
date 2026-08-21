//! ACP Control Plane — Stdio-based ACP runtime.
//!
//! Manages external ACP agent processes via stdin/stdout NDJSON (JSON-RPC 2.0).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, Mutex};

use super::health;
use super::types::*;
use crate::acp_control::config::AcpBackendProtocol;

/// Stdio-based ACP runtime — spawns an external ACP agent as a child process
/// and communicates over stdin/stdout using NDJSON (JSON-RPC 2.0).
pub struct StdioAcpRuntime {
    id: String,
    name: String,
    binary_path: String,
    acp_args: Vec<String>,
    protocol: AcpBackendProtocol,
    env_overrides: HashMap<String, String>,
    /// Active child processes keyed by local session_id.
    children: Arc<Mutex<HashMap<String, ChildHandle>>>,
}

struct ChildHandle {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    external_session_id: Option<String>,
}

impl StdioAcpRuntime {
    pub fn new(
        id: String,
        name: String,
        binary_path: String,
        acp_args: Vec<String>,
        protocol: AcpBackendProtocol,
        env_overrides: HashMap<String, String>,
    ) -> Self {
        Self {
            id,
            name,
            binary_path,
            acp_args,
            protocol,
            env_overrides,
            children: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn the child process in ACP mode.
    fn spawn_child(&self, cwd: Option<&str>) -> anyhow::Result<Child> {
        let mut cmd = Command::new(&self.binary_path);

        // Launch arguments are distribution data. An empty list explicitly
        // means the configured adapter binary itself speaks ACP.
        cmd.args(&self.acp_args);

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        // Environment: inherit + filter sensitive vars + apply overrides
        cmd.envs(&self.env_overrides);

        // Stdio: pipe all three
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Setup has several fallible steps before the child enters the
            // runtime registry. Keep this as a last-resort backstop in case an
            // unexpected early return bypasses explicit termination below.
            .kill_on_drop(true);

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

        // Never flash a console window when launching the ACP backend on Windows.
        ha_core::platform::hide_console_tokio(&mut cmd);

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
        child: &ChildHandle,
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

        {
            let mut stdin = child.stdin.lock().await;
            let mut line = serde_json::to_string(&request)?;
            line.push('\n');
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }

        // Read response lines from stdout until we get a response with our ID
        let mut reader = child.stdout.lock().await;
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

    async fn terminate_unregistered_child(child: &mut Child, backend_id: &str) {
        // No session owns this process yet, so a setup failure must make it
        // terminal before returning to the caller. `wait` reaps the process;
        // kill_on_drop remains armed if the bounded wait itself cannot finish.
        if let Some(pid) = child.id() {
            ha_core::blocking::run_blocking(move || {
                ha_core::platform::terminate_process_tree(pid);
            })
            .await;
        } else {
            let _ = child.start_kill();
        }
        match tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                ha_core::app_warn!(
                    "acp_control",
                    "setup_cleanup",
                    "Failed to reap rejected ACP backend child: backend_id={}, error={}",
                    backend_id,
                    error
                );
            }
            Err(_) => {
                ha_core::app_warn!(
                    "acp_control",
                    "setup_cleanup",
                    "Timed out reaping rejected ACP backend child: backend_id={}",
                    backend_id
                );
            }
        }
    }

    fn prompt_content(&self, prompt: &str) -> serde_json::Value {
        match self.protocol {
            AcpBackendProtocol::V1 => serde_json::json!([{
                "type": "text",
                "text": prompt
            }]),
            AcpBackendProtocol::Legacy02 => serde_json::json!([{
                "type": "user_message_chunk",
                "content": prompt
            }]),
        }
    }

    fn session_start_request(
        &self,
        cwd: &str,
        resume_session_id: Option<&str>,
    ) -> (&'static str, serde_json::Value) {
        match self.protocol {
            AcpBackendProtocol::V1 => match resume_session_id {
                Some(resume_id) => (
                    "session/load",
                    serde_json::json!({
                        "sessionId": resume_id,
                        "cwd": cwd,
                        "mcpServers": []
                    }),
                ),
                None => (
                    "session/new",
                    serde_json::json!({
                        "cwd": cwd,
                        "mcpServers": []
                    }),
                ),
            },
            AcpBackendProtocol::Legacy02 => {
                let mut params = serde_json::json!({"cwd": cwd});
                if let Some(resume_id) = resume_session_id {
                    params["resumeSessionId"] = serde_json::json!(resume_id);
                }
                ("session/new", params)
            }
        }
    }

    fn session_update<'a>(&self, params: &'a serde_json::Value) -> Option<&'a serde_json::Value> {
        match self.protocol {
            AcpBackendProtocol::V1 => params.get("update"),
            AcpBackendProtocol::Legacy02 => params.get("sessionUpdate"),
        }
    }

    fn parse_usage_update(&self, update: &serde_json::Value) -> Option<ParsedUsage> {
        match self.protocol {
            AcpBackendProtocol::V1 => {
                let used = update.get("used")?.as_u64()?;
                let size = update.get("size")?.as_u64()?;
                Some(ParsedUsage {
                    // ACP v1 exposes context occupancy rather than an input/output
                    // split. Preserve its exact total in the legacy accounting
                    // projection; a backend-specific PromptResponse.usage below
                    // replaces this fallback when an exact split is available.
                    input_tokens: used,
                    output_tokens: 0,
                    context_used: Some(used),
                    context_size: Some(size),
                })
            }
            AcpBackendProtocol::Legacy02 => {
                let input_tokens = update.get("inputTokens").and_then(|value| value.as_u64());
                let output_tokens = update.get("outputTokens").and_then(|value| value.as_u64());
                if input_tokens.is_none() && output_tokens.is_none() {
                    return None;
                }
                Some(ParsedUsage {
                    input_tokens: input_tokens.unwrap_or(0),
                    output_tokens: output_tokens.unwrap_or(0),
                    context_used: None,
                    context_size: None,
                })
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedUsage {
    input_tokens: u64,
    output_tokens: u64,
    context_used: Option<u64>,
    context_size: Option<u64>,
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
        let mut cmd = tokio::process::Command::new(&self.binary_path);
        cmd.arg("--version");
        ha_core::platform::hide_console_tokio(&mut cmd);
        let output = cmd.output().await?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(text)
    }

    async fn create_session(&self, params: AcpCreateParams) -> anyhow::Result<AcpExternalSession> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let timeout_secs = params.timeout_secs.unwrap_or_else(|| {
            ha_core::config::cached_config()
                .acp_control
                .default_timeout_secs
        });

        let effective_cwd = match params.cwd.as_deref() {
            Some(cwd) if std::path::Path::new(cwd).is_absolute() => cwd.to_string(),
            Some(cwd) if self.protocol == AcpBackendProtocol::V1 => {
                anyhow::bail!("ACP v1 working directory must be absolute: {cwd}")
            }
            Some(cwd) => cwd.to_string(),
            None => std::env::current_dir()
                .map_err(|error| {
                    anyhow::anyhow!("Failed to resolve ACP working directory: {error}")
                })?
                .to_string_lossy()
                .into_owned(),
        };

        let mut child = self.spawn_child(Some(&effective_cwd))?;
        let pid = child.id();
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                Self::terminate_unregistered_child(&mut child, &self.id).await;
                anyhow::bail!("Child stdin unavailable");
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                Self::terminate_unregistered_child(&mut child, &self.id).await;
                anyhow::bail!("Child stdout unavailable");
            }
        };

        let mut handle = ChildHandle {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            external_session_id: None,
        };

        // Step 1: initialize
        let requested_protocol = match self.protocol {
            AcpBackendProtocol::V1 => serde_json::json!(1),
            AcpBackendProtocol::Legacy02 => serde_json::json!("0.2"),
        };
        let init_result = match Self::send_request(
            &handle,
            "initialize",
            serde_json::json!({
                "protocolVersion": requested_protocol,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                },
                "clientInfo": {
                    "name": "hope-agent-acp-control",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            1,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                Self::terminate_unregistered_child(&mut handle.child, &self.id).await;
                return Err(error);
            }
        };
        if init_result.get("protocolVersion") != Some(&requested_protocol) {
            let actual = init_result
                .get("protocolVersion")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let error = anyhow::anyhow!(
                "ACP backend '{}' negotiated incompatible protocol: requested {}, received {}",
                self.id,
                requested_protocol,
                actual
            );
            Self::terminate_unregistered_child(&mut handle.child, &self.id).await;
            return Err(error);
        }

        // Step 2: session/new
        let (session_method, new_params) =
            self.session_start_request(&effective_cwd, params.resume_session_id.as_deref());

        let session_result = match Self::send_request(&handle, session_method, new_params, 2).await
        {
            Ok(result) => result,
            Err(error) => {
                Self::terminate_unregistered_child(&mut handle.child, &self.id).await;
                return Err(error);
            }
        };

        let external_sid = if self.protocol == AcpBackendProtocol::V1 {
            params.resume_session_id.clone().or_else(|| {
                session_result
                    .get("sessionId")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
        } else {
            session_result
                .get("sessionId")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        };

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
            timeout_secs,
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
        let (stdin, stdout, ext_sid) = {
            let children = self.children.lock().await;
            let handle = children
                .get(&session.session_id)
                .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session.session_id))?;
            (
                handle.stdin.clone(),
                handle.stdout.clone(),
                handle
                    .external_session_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            )
        };

        // Send session/prompt
        let prompt_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "session/prompt",
            "params": {
                "sessionId": ext_sid,
                "prompt": self.prompt_content(prompt)
            }
        });

        {
            let mut stdin = stdin.lock().await;
            let mut line = serde_json::to_string(&prompt_request)?;
            line.push('\n');
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }

        // Read events until we get the prompt response
        let mut reader = stdout.lock().await;
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
            let read_line = reader.read_line(&mut buf);
            let n = if session.timeout_secs == 0 {
                read_line.await?
            } else {
                tokio::time::timeout(
                    std::time::Duration::from_secs(session.timeout_secs),
                    read_line,
                )
                .await
                .map_err(|_| anyhow::anyhow!("Turn timed out after {}s", session.timeout_secs))??
            };

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
                    // Some v1 adapters expose the protocol's unstable usage
                    // extension on PromptResponse. Prefer that exact split over
                    // the context-occupancy fallback from usage_update.
                    if let Some(usage) = result.get("usage") {
                        if let (Some(input), Some(output)) = (
                            usage.get("inputTokens").and_then(|value| value.as_u64()),
                            usage.get("outputTokens").and_then(|value| value.as_u64()),
                        ) {
                            total_input = input;
                            total_output = output;
                        }
                    }
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
                    if let Some(update) = self.session_update(params) {
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
                                    .map(|s| ha_core::truncate_utf8(s, 2048).to_string());

                                let _ = event_tx
                                    .send(AcpStreamEvent::ToolResult {
                                        tool_call_id: call_id,
                                        status,
                                        result_preview: preview,
                                    })
                                    .await;
                            }
                            "usage_update" => {
                                if let Some(usage) = self.parse_usage_update(update) {
                                    total_input = usage.input_tokens;
                                    total_output = usage.output_tokens;
                                    let _ = event_tx
                                        .send(AcpStreamEvent::Usage {
                                            input_tokens: usage.input_tokens,
                                            output_tokens: usage.output_tokens,
                                            context_used: usage.context_used,
                                            context_size: usage.context_size,
                                        })
                                        .await;
                                }
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
            // Unix: SIGTERM to -pgid reaches any tools the ACP backend
            // spawned (child was started with setpgid(0,0) in pre_exec).
            // Windows: direct-pid taskkill only; ACP backends in practice
            // (claude / codex) don't fork subprocesses, so the narrower
            // semantics are fine.
            if let Some(pid) = handle.child.id() {
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGTERM);
                    }
                }
                #[cfg(not(unix))]
                {
                    ha_core::platform::send_graceful_stop(pid);
                }
            }
        }
        Ok(())
    }

    async fn close_session(&self, session: &AcpExternalSession) -> anyhow::Result<()> {
        let mut children = self.children.lock().await;
        if let Some(mut handle) = children.remove(&session.session_id) {
            // Try graceful close first
            {
                let mut stdin = handle.stdin.lock().await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(protocol: AcpBackendProtocol) -> StdioAcpRuntime {
        StdioAcpRuntime::new(
            "test".into(),
            "Test".into(),
            "test-acp".into(),
            vec![],
            protocol,
            HashMap::new(),
        )
    }

    #[test]
    fn v1_prompt_uses_text_content_block() {
        assert_eq!(
            runtime(AcpBackendProtocol::V1).prompt_content("hello"),
            serde_json::json!([{"type": "text", "text": "hello"}])
        );
    }

    #[test]
    fn legacy_prompt_keeps_user_message_chunk_shape() {
        assert_eq!(
            runtime(AcpBackendProtocol::Legacy02).prompt_content("hello"),
            serde_json::json!([{"type": "user_message_chunk", "content": "hello"}])
        );
    }

    #[test]
    fn v1_new_session_includes_required_workspace_fields() {
        assert_eq!(
            runtime(AcpBackendProtocol::V1).session_start_request("/workspace", None),
            (
                "session/new",
                serde_json::json!({"cwd": "/workspace", "mcpServers": []})
            )
        );
    }

    #[test]
    fn v1_resume_uses_load_session_contract() {
        assert_eq!(
            runtime(AcpBackendProtocol::V1)
                .session_start_request("/workspace", Some("external-session"),),
            (
                "session/load",
                serde_json::json!({
                    "sessionId": "external-session",
                    "cwd": "/workspace",
                    "mcpServers": []
                })
            )
        );
    }

    #[test]
    fn v1_usage_update_preserves_context_occupancy() {
        let runtime = runtime(AcpBackendProtocol::V1);
        let params = serde_json::json!({
            "sessionId": "session-1",
            "update": {"sessionUpdate": "usage_update", "used": 23116, "size": 200000}
        });
        let update = runtime.session_update(&params).expect("v1 update");

        assert_eq!(
            runtime.parse_usage_update(update),
            Some(ParsedUsage {
                input_tokens: 23116,
                output_tokens: 0,
                context_used: Some(23116),
                context_size: Some(200000),
            })
        );
    }

    #[test]
    fn legacy_usage_update_keeps_split_token_fields() {
        let runtime = runtime(AcpBackendProtocol::Legacy02);
        let params = serde_json::json!({
            "sessionId": "session-1",
            "sessionUpdate": {
                "sessionUpdate": "usage_update",
                "inputTokens": 120,
                "outputTokens": 8
            }
        });
        let update = runtime.session_update(&params).expect("legacy update");

        assert_eq!(
            runtime.parse_usage_update(update),
            Some(ParsedUsage {
                input_tokens: 120,
                output_tokens: 8,
                context_used: None,
                context_size: None,
            })
        );
    }

    #[tokio::test]
    async fn rejected_setup_child_is_terminated_and_reaped() {
        #[cfg(unix)]
        let runtime = StdioAcpRuntime::new(
            "test".into(),
            "Test".into(),
            "sleep".into(),
            vec!["30".into()],
            AcpBackendProtocol::V1,
            HashMap::new(),
        );
        #[cfg(windows)]
        let runtime = StdioAcpRuntime::new(
            "test".into(),
            "Test".into(),
            "ping.exe".into(),
            vec!["-n".into(), "30".into(), "127.0.0.1".into()],
            AcpBackendProtocol::V1,
            HashMap::new(),
        );
        let mut child = runtime.spawn_child(None).expect("spawn long-running child");

        StdioAcpRuntime::terminate_unregistered_child(&mut child, "test").await;

        assert!(child.try_wait().expect("inspect child status").is_some());
    }
}
