use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::process_manager::ManagedProcess;
use crate::channel::types::{ChannelId, ChatType, MsgContext};

/// Default timeout for RPC calls in milliseconds.
const RPC_TIMEOUT_MS: u64 = 10_000;

/// iMessage RPC notification payload exchanged with the imsg CLI.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct IMessagePayload {
    pub id: Option<i64>,
    pub guid: Option<String>,
    pub chat_id: Option<i64>,
    pub sender: Option<String>,
    pub is_from_me: Option<bool>,
    pub text: Option<String>,
    pub reply_to_id: Option<Value>,
    pub reply_to_text: Option<String>,
    pub reply_to_sender: Option<String>,
    pub created_at: Option<String>,
    pub attachments: Option<Vec<IMessageAttachment>>,
    pub chat_identifier: Option<String>,
    pub chat_guid: Option<String>,
    pub chat_name: Option<String>,
    pub participants: Option<Vec<String>>,
    pub is_group: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct IMessageAttachment {
    pub original_path: Option<String>,
    pub mime_type: Option<String>,
    pub missing: Option<bool>,
}

/// Manages a JSON-RPC over stdio connection to the `imsg rpc` child process.
pub struct IMessageClient {
    /// stdin writer for sending RPC requests.
    stdin_tx: mpsc::Sender<String>,
    /// Pending RPC requests awaiting responses, keyed by request ID.
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
    /// Cancel token for the notification loop.
    cancel: CancellationToken,
    /// The managed child process.
    process: Arc<Mutex<Option<ManagedProcess>>>,
    /// Monotonically increasing request ID.
    next_id: Arc<Mutex<u64>>,
}

impl IMessageClient {
    /// Start the `imsg rpc` process and set up the stdin/stdout communication channels.
    ///
    /// - `imsg_path`: Path to the `imsg` binary (defaults to "imsg" if empty).
    /// - `db_path`: Optional path to the iMessage database.
    pub fn start(imsg_path: &str, db_path: Option<&str>) -> Result<Self> {
        let binary = if imsg_path.is_empty() {
            "imsg"
        } else {
            imsg_path
        };

        let mut args = vec!["rpc"];
        // db_path needs to live long enough for the borrow
        let db_path_owned = db_path.map(|s| s.to_string());
        if let Some(ref db) = db_path_owned {
            if !db.is_empty() {
                args.push("--db");
                args.push(db.as_str());
            }
        }

        let process = ManagedProcess::spawn(binary, &args)
            .with_context(|| format!("Failed to start imsg rpc process at '{}'", binary))?;

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let cancel = CancellationToken::new();

        // Create a channel for stdin writes so we can write from async context
        let (stdin_tx, stdin_rx) = mpsc::channel::<String>(64);

        let process = Arc::new(Mutex::new(Some(process)));

        // Spawn stdin writer task
        {
            let process = process.clone();
            let cancel = cancel.clone();
            tokio::spawn(Self::stdin_writer_loop(process, stdin_rx, cancel));
        }

        Ok(Self {
            stdin_tx,
            pending,
            cancel,
            process,
            next_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Background task that writes queued messages to the process's stdin.
    async fn stdin_writer_loop(
        process: Arc<Mutex<Option<ManagedProcess>>>,
        mut rx: mpsc::Receiver<String>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                msg = rx.recv() => {
                    match msg {
                        Some(line) => {
                            let mut guard = process.lock().await;
                            if let Some(ref mut proc) = *guard {
                                if let Some(stdin) = proc.stdin() {
                                    if let Err(e) = stdin.write_all(line.as_bytes()).await {
                                        app_error!(
                                            "channel",
                                            "imessage",
                                            "Failed to write to imsg stdin: {}",
                                            e
                                        );
                                        break;
                                    }
                                    if let Err(e) = stdin.flush().await {
                                        app_error!(
                                            "channel",
                                            "imessage",
                                            "Failed to flush imsg stdin: {}",
                                            e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    }

    /// Run the notification listener loop.
    ///
    /// Reads lines from stdout, dispatches RPC responses to pending callers,
    /// and converts server-initiated notifications (newMessage) into `MsgContext`
    /// sent via `inbound_tx`.
    ///
    /// `ready_tx` 在 spawn 出来的 task 进入接收循环（即真正可以处理 RPC
    /// response）之后立即 send。caller 必须 await 这个 oneshot 之后再调任何
    /// RPC（如 watch_subscribe）——否则 RPC response 会在 task 启动之前回来，
    /// pending oneshot 没人接，超时失败。
    pub async fn run_notification_loop(
        &self,
        account_id: String,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
        ready_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let process = self.process.clone();
        let pending = self.pending.clone();
        let stderr_cancel = cancel.clone();
        let ready_tx = std::sync::Mutex::new(Some(ready_tx));

        tokio::spawn(async move {
            // 进入循环前发 ready 信号
            if let Some(tx) = ready_tx.lock().ok().and_then(|mut g| g.take()) {
                let _ = tx.send(());
            }
            loop {
                let line = {
                    let mut guard = process.lock().await;
                    match guard.as_mut() {
                        Some(proc) => {
                            tokio::select! {
                                _ = cancel.cancelled() => break,
                                line = proc.stdout_rx.recv() => line,
                            }
                        }
                        None => break,
                    }
                };

                match line {
                    Some(raw_line) => {
                        let trimmed = raw_line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let parsed: Value = match serde_json::from_str(trimmed) {
                            Ok(v) => v,
                            Err(e) => {
                                app_warn!(
                                    "channel",
                                    "imessage",
                                    "Failed to parse RPC line: {} (line: {})",
                                    e,
                                    crate::truncate_utf8(trimmed, 200)
                                );
                                continue;
                            }
                        };

                        // Check if this is a response (has "id" field) or a notification
                        if let Some(id) = parsed.get("id").and_then(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .or_else(|| v.as_i64().map(|n| n.to_string()))
                                .or_else(|| v.as_u64().map(|n| n.to_string()))
                        }) {
                            // This is a response to a pending request
                            let mut map = pending.lock().await;
                            if let Some(sender) = map.remove(&id) {
                                if let Some(err) = parsed.get("error") {
                                    let msg = err
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("imsg rpc error");
                                    let _ = sender.send(Err(anyhow::anyhow!("{}", msg)));
                                } else {
                                    let result =
                                        parsed.get("result").cloned().unwrap_or(Value::Null);
                                    let _ = sender.send(Ok(result));
                                }
                            }
                        } else if let Some(method) = parsed.get("method").and_then(|v| v.as_str()) {
                            // Server-initiated notification (no "id")
                            if method == "message" || method == "newMessage" {
                                if let Some(params) = parsed.get("params") {
                                    match Self::parse_notification(params, &account_id) {
                                        Some(msg_ctx) => {
                                            if inbound_tx.send(msg_ctx).await.is_err() {
                                                app_warn!(
                                                    "channel",
                                                    "imessage",
                                                    "Inbound channel closed, stopping notification loop"
                                                );
                                                break;
                                            }
                                        }
                                        None => {
                                            app_debug!(
                                                "channel",
                                                "imessage",
                                                "Dropping malformed notification"
                                            );
                                        }
                                    }
                                }
                            } else if method == "error" {
                                app_warn!(
                                    "channel",
                                    "imessage",
                                    "imsg rpc error notification: {}",
                                    parsed.get("params").unwrap_or(&Value::Null)
                                );
                            }
                        }
                    }
                    None => {
                        app_info!(
                            "channel",
                            "imessage",
                            "imsg stdout closed, notification loop ending"
                        );
                        // Fail all pending requests
                        let mut map = pending.lock().await;
                        for (_, sender) in map.drain() {
                            let _ = sender.send(Err(anyhow::anyhow!("imsg rpc process exited")));
                        }
                        break;
                    }
                }
            }
        });

        // Spawn stderr logger
        {
            let process = self.process.clone();
            tokio::spawn(async move {
                loop {
                    let line = {
                        let mut guard = process.lock().await;
                        match guard.as_mut() {
                            Some(proc) => {
                                tokio::select! {
                                    _ = stderr_cancel.cancelled() => break,
                                    line = proc.stderr_rx.recv() => line,
                                }
                            }
                            None => break,
                        }
                    };
                    match line {
                        Some(err_line) => {
                            if !err_line.trim().is_empty() {
                                app_warn!(
                                    "channel",
                                    "imessage",
                                    "imsg stderr: {}",
                                    crate::truncate_utf8(&err_line, 500)
                                );
                            }
                        }
                        None => break,
                    }
                }
            });
        }
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn rpc_call(&self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id.to_string()
        };

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let line = format!("{}\n", serde_json::to_string(&request)?);

        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut map = self.pending.lock().await;
            map.insert(id.clone(), tx);
        }

        // Send the request via stdin
        if self.stdin_tx.send(line).await.is_err() {
            let mut map = self.pending.lock().await;
            map.remove(&id);
            return Err(anyhow::anyhow!("imsg rpc process stdin closed"));
        }

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_millis(RPC_TIMEOUT_MS), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow::anyhow!("imsg rpc response channel dropped")),
            Err(_) => {
                // Timeout: remove from pending
                let mut map = self.pending.lock().await;
                map.remove(&id);
                Err(anyhow::anyhow!("imsg rpc timeout ({})", method))
            }
        }
    }

    /// Send a text message to a chat.
    pub async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<Value> {
        let mut params = serde_json::json!({
            "to": chat_id,
            "text": text,
            "service": "auto",
        });
        if let Some(reply_id) = reply_to {
            params["reply_to"] = Value::String(reply_id.to_string());
        }
        self.rpc_call("send", params).await
    }

    /// Send a typing indicator to a chat.
    pub async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let params = serde_json::json!({
            "to": chat_id,
        });
        self.rpc_call("sendTyping", params).await?;
        Ok(())
    }

    /// List conversations (used for probe).
    pub async fn list_conversations(&self) -> Result<Value> {
        let params = serde_json::json!({ "limit": 5 });
        self.rpc_call("chats.list", params).await
    }

    /// Subscribe to new message notifications.
    pub async fn watch_subscribe(&self) -> Result<Value> {
        self.rpc_call("watch.subscribe", serde_json::json!({}))
            .await
    }

    /// Stop the RPC client and shut down the child process.
    pub async fn stop(&self) {
        self.cancel.cancel();

        // Fail all pending requests
        {
            let mut map = self.pending.lock().await;
            for (_, sender) in map.drain() {
                let _ = sender.send(Err(anyhow::anyhow!("imsg rpc client stopped")));
            }
        }

        // Shutdown the child process
        let mut guard = self.process.lock().await;
        if let Some(ref mut proc) = *guard {
            proc.shutdown(std::time::Duration::from_millis(2000)).await;
        }
        *guard = None;
    }

    /// Parse a JSON-RPC notification into a `MsgContext`.
    ///
    /// The notification `params` should contain a `message` object with the
    /// `IMessagePayload` fields.
    fn parse_notification(params: &Value, account_id: &str) -> Option<MsgContext> {
        // The params may contain a "message" wrapper or be the message directly
        let message_val = params.get("message").unwrap_or(params);

        let payload: IMessagePayload = match serde_json::from_value(message_val.clone()) {
            Ok(p) => p,
            Err(e) => {
                app_debug!(
                    "channel",
                    "imessage",
                    "Failed to parse notification payload: {}",
                    e
                );
                return None;
            }
        };

        // Skip messages from self
        if payload.is_from_me == Some(true) {
            return None;
        }

        let sender = payload.sender.as_deref().unwrap_or("").trim();
        if sender.is_empty() {
            return None;
        }

        // Determine chat type: group if is_group=true or has multiple participants
        let is_group = payload.is_group.unwrap_or(false)
            || payload
                .participants
                .as_ref()
                .map(|p| p.len() > 2)
                .unwrap_or(false);

        let chat_type = if is_group {
            ChatType::Group
        } else {
            ChatType::Dm
        };

        // 解析 chat_id 优先级：chat_guid > chat_identifier > chat_id（数字）
        // > sender。chat_guid 是 iMessage 协议层最稳定的会话标识（形如
        // `iMessage;-;chat...`），跨消息复现一致；numeric chat_id 在 imsg
        // 不同推送中可能缺失，导致同一会话有时映射到 numeric、有时到 guid，
        // worker 侧把它们看作两个不同会话 → 群聊跨消息历史断裂、记忆丢失。
        let chat_id = if let Some(ref guid) = payload.chat_guid {
            guid.clone()
        } else if let Some(ref identifier) = payload.chat_identifier {
            identifier.clone()
        } else if let Some(cid) = payload.chat_id {
            cid.to_string()
        } else {
            sender.to_string()
        };

        // Message GUID
        let message_id = payload
            .guid
            .clone()
            .or_else(|| payload.id.map(|id| id.to_string()))
            .unwrap_or_default();

        // Sender display name: use chat_name for groups, sender handle otherwise
        let sender_name = if is_group {
            // In groups, sender is the handle; chat_name is the group name
            Some(sender.to_string())
        } else {
            Some(sender.to_string())
        };

        // Chat title for groups
        let chat_title = if is_group {
            payload.chat_name.clone()
        } else {
            None
        };

        // Parse reply_to_id
        let reply_to = payload.reply_to_id.as_ref().and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });

        // Parse timestamp
        let timestamp = payload
            .created_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        Some(MsgContext {
            channel_id: ChannelId::IMessage,
            account_id: account_id.to_string(),
            sender_id: sender.to_string(),
            sender_name,
            sender_username: Some(sender.to_string()),
            chat_id,
            chat_type,
            chat_title,
            thread_id: None,
            message_id,
            text: payload.text,
            media: Vec::new(),
            reply_to_message_id: reply_to,
            timestamp,
            was_mentioned: false, // iMessage doesn't have @mentions
            raw: message_val.clone(),
        })
    }
}
