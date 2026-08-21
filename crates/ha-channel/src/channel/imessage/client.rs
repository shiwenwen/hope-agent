use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::process_manager::ManagedProcess;
use ha_core::channel::types::{ChannelId, ChatType, InboundEvent, MsgContext};

/// Default timeout for RPC calls in milliseconds.
const RPC_TIMEOUT_MS: u64 = 10_000;
const INBOUND_DEDUPE_LIMIT: usize = 4096;
const RESTART_BACKOFF_MAX_SECS: u64 = 30;

/// Protocol-v1 status fields surfaced by `imsg initialize/status`.
/// Only the compatibility-safe, non-sensitive subset is retained.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct IMessageStatus {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub protocol_version: Option<u64>,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub supported_methods: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IMessageDeliveryDisposition {
    NotStarted,
    MayHaveCompleted,
    StillInFlight,
    Unknown,
}

impl IMessageDeliveryDisposition {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("not_started") => Self::NotStarted,
            Some("may_have_completed") => Self::MayHaveCompleted,
            Some("still_in_flight") => Self::StillInFlight,
            _ => Self::Unknown,
        }
    }
}

/// Typed JSON-RPC error. In particular, `-32001` and `-32004` retain their
/// authoritative delivery disposition instead of collapsing to a string that
/// a caller might accidentally classify as retryable.
#[derive(Debug, Clone)]
pub struct IMessageRpcError {
    pub code: Option<i64>,
    pub message: String,
    pub retry_safe: bool,
    pub disposition: IMessageDeliveryDisposition,
}

impl IMessageRpcError {
    fn transport(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            retry_safe: false,
            disposition: IMessageDeliveryDisposition::Unknown,
        }
    }

    fn from_wire(error: &Value) -> Self {
        let code = error.get("code").and_then(Value::as_i64);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("imsg rpc error");
        let data = error.get("data").and_then(Value::as_object);
        let disposition = IMessageDeliveryDisposition::parse(
            data.and_then(|value| value.get("disposition"))
                .and_then(Value::as_str),
        );
        let declared_retry_safe = data
            .and_then(|value| value.get("retry_safe"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Ambiguous or still-in-flight mutation outcomes are never retryable,
        // even if a malformed sidecar claims otherwise.
        let retry_safe = declared_retry_safe
            && !matches!(code, Some(-32001 | -32004))
            && matches!(disposition, IMessageDeliveryDisposition::NotStarted);
        Self {
            code,
            message: ha_core::logging::redact_sensitive(message),
            retry_safe,
            disposition,
        }
    }

    pub fn is_legacy_protocol_unsupported(&self) -> bool {
        matches!(self.code, Some(-32601 | -32602))
    }

    pub fn user_safe_delivery_message(&self) -> &'static str {
        match (self.code, self.disposition) {
            (Some(-32004), _) | (_, IMessageDeliveryDisposition::StillInFlight) => {
                "iMessage delivery lane is blocked by an earlier uncertain operation; restart the account after checking Messages"
            }
            (Some(-32001), _) | (_, IMessageDeliveryDisposition::MayHaveCompleted) => {
                "iMessage delivery outcome is uncertain; Hope Agent will not retry automatically"
            }
            (_, IMessageDeliveryDisposition::NotStarted) if self.retry_safe => {
                "iMessage did not start the delivery operation"
            }
            _ => "iMessage delivery failed; automatic retry is disabled",
        }
    }
}

impl fmt::Display for IMessageRpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(f, "imsg rpc error {code}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for IMessageRpcError {}

type RpcResult = std::result::Result<Value, IMessageRpcError>;

#[derive(Clone)]
struct RpcHandle {
    stdin_tx: mpsc::Sender<String>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<RpcResult>>>>,
    next_id: Arc<Mutex<u64>>,
}

impl RpcHandle {
    async fn call(&self, method: &str, params: Value) -> RpcResult {
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
        let line = serde_json::to_string(&request)
            .map(|value| format!("{value}\n"))
            .map_err(|error| IMessageRpcError::transport(error.to_string()))?;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        if self.stdin_tx.send(line).await.is_err() {
            self.pending.lock().await.remove(&id);
            return Err(IMessageRpcError::transport("imsg rpc process stdin closed"));
        }
        match tokio::time::timeout(std::time::Duration::from_millis(RPC_TIMEOUT_MS), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(IMessageRpcError::transport(
                "imsg rpc response channel dropped",
            )),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(IMessageRpcError::transport(format!(
                    "imsg rpc timeout ({method})"
                )))
            }
        }
    }
}

#[derive(Debug, Default)]
struct RuntimeState {
    status: Option<IMessageStatus>,
    last_rowid: i64,
    degraded_error: Option<String>,
}

#[derive(Default)]
struct InboundDeduper {
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl InboundDeduper {
    fn admit(&mut self, value: &Value) -> bool {
        let Some(key) = message_dedupe_key(value) else {
            return true;
        };
        if !self.ids.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        while self.order.len() > INBOUND_DEDUPE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.ids.remove(&oldest);
            }
        }
        true
    }
}

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
    rpc: RpcHandle,
    /// Cancel token for the notification loop.
    cancel: CancellationToken,
    /// The managed child process.
    process: Arc<Mutex<Option<ManagedProcess>>>,
    stdout_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    stderr_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    binary: String,
    args: Vec<String>,
    runtime: Arc<Mutex<RuntimeState>>,
    deduper: Arc<Mutex<InboundDeduper>>,
    protocol_v1: Arc<AtomicBool>,
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

        let mut args = vec!["rpc".to_string()];
        if let Some(db) = db_path.map(str::to_string) {
            if !db.is_empty() {
                args.push("--db".to_string());
                args.push(db);
            }
        }

        let (process, stdout_rx, stderr_rx) = spawn_managed_process(binary, &args)
            .with_context(|| format!("Failed to start imsg rpc process at '{}'", binary))?;

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<RpcResult>>>> =
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
            rpc: RpcHandle {
                stdin_tx,
                pending,
                next_id: Arc::new(Mutex::new(1)),
            },
            cancel,
            process,
            stdout_rx: Arc::new(Mutex::new(stdout_rx)),
            stderr_rx: Arc::new(Mutex::new(stderr_rx)),
            binary: binary.to_string(),
            args,
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
            deduper: Arc::new(Mutex::new(InboundDeduper::default())),
            protocol_v1: Arc::new(AtomicBool::new(false)),
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
                                        // Do not replay a possibly partially written
                                        // mutation. Keep the writer alive so a supervised
                                        // child restart can accept later requests.
                                        continue;
                                    }
                                    if let Err(e) = stdin.flush().await {
                                        app_error!(
                                            "channel",
                                            "imessage",
                                            "Failed to flush imsg stdin: {}",
                                            e
                                        );
                                        continue;
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
        inbound_tx: mpsc::Sender<InboundEvent>,
        cancel: CancellationToken,
        ready_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let (recovery_tx, recovery_rx) = mpsc::channel::<i64>(4);
        tokio::spawn(recovery_loop(
            self.rpc.clone(),
            account_id.clone(),
            inbound_tx.clone(),
            self.runtime.clone(),
            self.deduper.clone(),
            cancel.clone(),
            self.cancel.clone(),
            recovery_rx,
        ));

        let stdout_rx = self.stdout_rx.clone();
        let stderr_rx = self.stderr_rx.clone();
        let restart_stderr_rx = stderr_rx.clone();
        let process = self.process.clone();
        let pending = self.rpc.pending.clone();
        let rpc = self.rpc.clone();
        let runtime = self.runtime.clone();
        let deduper = self.deduper.clone();
        let binary = self.binary.clone();
        let args = self.args.clone();
        let protocol_v1 = self.protocol_v1.clone();
        let client_cancel = self.cancel.clone();
        let stdout_cancel = cancel.clone();
        let stdout_account_id = account_id.clone();
        let stdout_inbound_tx = inbound_tx.clone();

        tokio::spawn(async move {
            let _ = ready_tx.send(());
            let mut restart_backoff_secs = 1u64;
            let mut protocol_recovery_cancel = CancellationToken::new();
            loop {
                let line = tokio::select! {
                    _ = stdout_cancel.cancelled() => break,
                    _ = client_cancel.cancelled() => break,
                    line = async {
                        let mut receiver = stdout_rx.lock().await;
                        receiver.recv().await
                    } => line,
                };

                if let Some(raw_line) = line {
                    let trimmed = raw_line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let parsed: Value = match serde_json::from_str(trimmed) {
                        Ok(value) => value,
                        Err(error) => {
                            // Never echo the raw line: a malformed notification
                            // can still contain private message text.
                            app_warn!(
                                "channel",
                                "imessage",
                                "Failed to parse imsg RPC line: {}",
                                error
                            );
                            continue;
                        }
                    };
                    if let Some(id) = rpc_response_id(&parsed) {
                        if let Some(sender) = pending.lock().await.remove(&id) {
                            let result = if let Some(error) = parsed.get("error") {
                                Err(IMessageRpcError::from_wire(error))
                            } else {
                                Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
                            };
                            let _ = sender.send(result);
                        }
                        continue;
                    }
                    match parsed.get("method").and_then(Value::as_str) {
                        Some("message" | "newMessage") => {
                            if let Some(params) = parsed.get("params") {
                                if !deliver_inbound_value(
                                    params,
                                    &stdout_account_id,
                                    &stdout_inbound_tx,
                                    &runtime,
                                    &deduper,
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        Some("watch.overflow") => {
                            let resume_after_rowid = parsed
                                .get("params")
                                .and_then(|value| value.get("resume_after_rowid"))
                                .and_then(Value::as_i64);
                            if let Some(cursor) = resume_after_rowid {
                                runtime.lock().await.degraded_error = Some(
                                    "iMessage watch overflow; deterministic catch-up in progress"
                                        .to_string(),
                                );
                                match recovery_tx.try_send(cursor) {
                                    Ok(()) => {}
                                    Err(mpsc::error::TrySendError::Full(_)) => {
                                        // An active or queued catch-up starts from an
                                        // earlier cursor and resubscribes from its final
                                        // rowid, so another overflow cursor is redundant.
                                        // Never block the stdout listener: it must keep
                                        // dispatching the recovery RPC responses.
                                        app_warn!(
                                            "channel",
                                            "imessage",
                                            "Coalescing iMessage watch overflow while recovery is already queued"
                                        );
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                                }
                            } else {
                                runtime.lock().await.degraded_error = Some(
                                    "iMessage watch overflow omitted its recovery cursor"
                                        .to_string(),
                                );
                                app_warn!(
                                    "channel",
                                    "imessage",
                                    "watch.overflow omitted resume_after_rowid; account is degraded"
                                );
                            }
                        }
                        Some("error") => {
                            app_warn!(
                                "channel",
                                "imessage",
                                "imsg emitted a redacted error notification"
                            );
                        }
                        _ => {}
                    }
                    continue;
                }

                if stdout_cancel.is_cancelled() || client_cancel.is_cancelled() {
                    break;
                }
                protocol_recovery_cancel.cancel();
                runtime.lock().await.degraded_error =
                    Some("imsg process exited; supervised restart pending".to_string());
                app_warn!(
                    "channel",
                    "imessage",
                    "imsg process exited; entering supervised restart"
                );
                for (_, sender) in pending.lock().await.drain() {
                    let _ = sender.send(Err(IMessageRpcError::transport(
                        "imsg rpc process exited; mutation outcome may be unknown",
                    )));
                }

                let restarted = loop {
                    tokio::select! {
                        _ = stdout_cancel.cancelled() => break false,
                        _ = client_cancel.cancelled() => break false,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(restart_backoff_secs)) => {}
                    }
                    match spawn_managed_process(&binary, &args) {
                        Ok((new_process, new_stdout, new_stderr)) => {
                            *process.lock().await = Some(new_process);
                            *stdout_rx.lock().await = new_stdout;
                            *restart_stderr_rx.lock().await = new_stderr;
                            runtime.lock().await.degraded_error = Some(
                                "imsg process restarted; protocol/watch recovery pending"
                                    .to_string(),
                            );
                            restart_backoff_secs = 1;
                            protocol_recovery_cancel = CancellationToken::new();
                            let restart_rpc = rpc.clone();
                            let restart_runtime = runtime.clone();
                            let restart_protocol_v1 = protocol_v1.clone();
                            let restart_cancel = protocol_recovery_cancel.clone();
                            let restart_client_cancel = client_cancel.clone();
                            tokio::spawn(restore_protocol_watch_until_ready(
                                restart_rpc,
                                restart_runtime,
                                restart_protocol_v1.load(Ordering::Acquire),
                                restart_cancel,
                                restart_client_cancel,
                                std::time::Duration::from_secs(1),
                            ));
                            app_info!(
                                "channel",
                                "imessage",
                                "imsg process restarted; restoring protocol and watch state"
                            );
                            break true;
                        }
                        Err(error) => {
                            app_warn!(
                                "channel",
                                "imessage",
                                "imsg restart failed; retrying in {}s: {}",
                                restart_backoff_secs,
                                ha_core::logging::redact_sensitive(&error.to_string())
                            );
                            restart_backoff_secs =
                                (restart_backoff_secs * 2).min(RESTART_BACKOFF_MAX_SECS);
                        }
                    }
                };
                if !restarted {
                    break;
                }
            }
            protocol_recovery_cancel.cancel();
        });

        let stderr_cancel = cancel;
        let stderr_client_cancel = self.cancel.clone();
        tokio::spawn(async move {
            loop {
                let line = tokio::select! {
                    _ = stderr_cancel.cancelled() => break,
                    _ = stderr_client_cancel.cancelled() => break,
                    line = async {
                        let mut receiver = stderr_rx.lock().await;
                        receiver.recv().await
                    } => line,
                };
                match line {
                    Some(value) if !value.trim().is_empty() => app_warn!(
                        "channel",
                        "imessage",
                        "imsg stderr: {}",
                        ha_core::truncate_utf8(
                            &ha_core::logging::redact_sensitive(value.trim()),
                            500
                        )
                    ),
                    Some(_) => {}
                    None => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                }
            }
        });
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn rpc_call(&self, method: &str, params: Value) -> RpcResult {
        self.rpc.call(method, params).await
    }

    /// Negotiate protocol v1 and cache the non-sensitive runtime status.
    /// Older `imsg` builds fall back to the legacy surface only when they
    /// explicitly reject `initialize` as unknown/unsupported.
    pub async fn negotiate_protocol_v1(
        &self,
        enabled: bool,
    ) -> std::result::Result<Option<IMessageStatus>, IMessageRpcError> {
        self.protocol_v1.store(enabled, Ordering::Release);
        if !enabled {
            return Ok(None);
        }
        match negotiate_status(&self.rpc).await {
            Ok(status) => {
                self.runtime.lock().await.status = Some(status.clone());
                Ok(Some(status))
            }
            Err(error) if error.is_legacy_protocol_unsupported() => {
                self.protocol_v1.store(false, Ordering::Release);
                self.runtime.lock().await.status = None;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Send a text message to a chat.
    pub async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> RpcResult {
        let params = build_send_params(chat_id, Some(text), None, reply_to);
        self.rpc_call("send", params).await
    }

    /// Send a file attachment, optionally with adjacent text.
    pub async fn send_file(
        &self,
        chat_id: &str,
        text: Option<&str>,
        file: &str,
        reply_to: Option<&str>,
    ) -> RpcResult {
        let params = build_send_params(chat_id, text, Some(file), reply_to);
        self.rpc_call("send", params).await
    }

    /// List conversations (used for probe).
    pub async fn list_conversations(&self) -> RpcResult {
        let params = serde_json::json!({ "limit": 5 });
        self.rpc_call("chats.list", params).await
    }

    /// Subscribe to new message notifications.
    pub async fn watch_subscribe(&self) -> RpcResult {
        let cursor = self.runtime.lock().await.last_rowid;
        let since = (self.protocol_v1.load(Ordering::Acquire) && cursor > 0).then_some(cursor);
        watch_subscribe(&self.rpc, since).await
    }

    pub async fn status_snapshot(&self) -> (Option<IMessageStatus>, Option<String>) {
        let runtime = self.runtime.lock().await;
        (runtime.status.clone(), runtime.degraded_error.clone())
    }

    /// Stop the RPC client and shut down the child process.
    pub async fn stop(&self) {
        self.cancel.cancel();

        // Fail all pending requests
        {
            let mut map = self.rpc.pending.lock().await;
            for (_, sender) in map.drain() {
                let _ = sender.send(Err(IMessageRpcError::transport("imsg rpc client stopped")));
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

        // `imsg` exposes an explicit public `is_group` boolean. Do not infer
        // groupness from participants: the local user is implicit and the
        // participant count is not a stable group/direct discriminator.
        let is_group = payload.is_group.unwrap_or(false);

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
            sender_tenant_id: None,
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

fn spawn_managed_process(
    binary: &str,
    args: &[String],
) -> Result<(
    ManagedProcess,
    mpsc::Receiver<String>,
    mpsc::Receiver<String>,
)> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut process = ManagedProcess::spawn(binary, &refs)?;
    let stdout = process.take_stdout_rx();
    let stderr = process.take_stderr_rx();
    Ok((process, stdout, stderr))
}

fn rpc_response_id(value: &Value) -> Option<String> {
    value.get("id").and_then(|id| {
        id.as_str()
            .map(str::to_string)
            .or_else(|| id.as_i64().map(|number| number.to_string()))
            .or_else(|| id.as_u64().map(|number| number.to_string()))
    })
}

fn message_value(value: &Value) -> &Value {
    value.get("message").unwrap_or(value)
}

fn message_rowid(value: &Value) -> Option<i64> {
    message_value(value).get("id").and_then(Value::as_i64)
}

fn message_dedupe_key(value: &Value) -> Option<String> {
    let value = message_value(value);
    value
        .get("guid")
        .and_then(Value::as_str)
        .filter(|guid| !guid.is_empty())
        .map(|guid| format!("guid:{guid}"))
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| format!("rowid:{id}"))
        })
}

async fn deliver_inbound_value(
    value: &Value,
    account_id: &str,
    inbound_tx: &mpsc::Sender<InboundEvent>,
    runtime: &Arc<Mutex<RuntimeState>>,
    deduper: &Arc<Mutex<InboundDeduper>>,
) -> bool {
    if let Some(rowid) = message_rowid(value) {
        let mut state = runtime.lock().await;
        state.last_rowid = state.last_rowid.max(rowid);
    }
    if !deduper.lock().await.admit(value) {
        return true;
    }
    let Some(message) = IMessageClient::parse_notification(value, account_id) else {
        return true;
    };
    if inbound_tx
        .send(InboundEvent::Message(message))
        .await
        .is_err()
    {
        app_warn!(
            "channel",
            "imessage",
            "Inbound channel closed, stopping iMessage notification loop"
        );
        return false;
    }
    true
}

async fn negotiate_status(rpc: &RpcHandle) -> RpcResultStatus {
    let initialized = rpc
        .call("initialize", serde_json::json!({ "protocol_version": 1 }))
        .await?;
    let initialized_status: IMessageStatus =
        serde_json::from_value(initialized).map_err(|error| {
            IMessageRpcError::transport(format!("invalid initialize result: {error}"))
        })?;
    if initialized_status.protocol_version != Some(1) {
        return Err(IMessageRpcError::transport(
            "imsg initialize did not confirm protocol version 1",
        ));
    }
    let status_value = rpc.call("status", serde_json::json!({})).await?;
    let status: IMessageStatus = serde_json::from_value(status_value)
        .map_err(|error| IMessageRpcError::transport(format!("invalid status result: {error}")))?;
    if status.protocol_version != Some(1) {
        return Err(IMessageRpcError::transport(
            "imsg status did not confirm protocol version 1",
        ));
    }
    Ok(status)
}

type RpcResultStatus = std::result::Result<IMessageStatus, IMessageRpcError>;

async fn watch_subscribe(rpc: &RpcHandle, since_rowid: Option<i64>) -> RpcResult {
    let params = since_rowid
        .map(|cursor| serde_json::json!({ "since_rowid": cursor }))
        .unwrap_or_else(|| serde_json::json!({}));
    rpc.call("watch.subscribe", params).await
}

async fn negotiate_and_subscribe(
    rpc: &RpcHandle,
    runtime: &Arc<Mutex<RuntimeState>>,
    protocol_v1: bool,
    resume: bool,
) -> std::result::Result<(), IMessageRpcError> {
    let mut negotiated = false;
    if protocol_v1 {
        match negotiate_status(rpc).await {
            Ok(status) => {
                runtime.lock().await.status = Some(status);
                negotiated = true;
            }
            Err(error) if error.is_legacy_protocol_unsupported() => {
                runtime.lock().await.status = None;
            }
            Err(error) => return Err(error),
        }
    }
    let cursor = runtime.lock().await.last_rowid;
    let since = (resume && negotiated && cursor > 0).then_some(cursor);
    watch_subscribe(rpc, since).await?;
    runtime.lock().await.degraded_error = None;
    Ok(())
}

async fn restore_protocol_watch_until_ready(
    rpc: RpcHandle,
    runtime: Arc<Mutex<RuntimeState>>,
    protocol_v1: bool,
    restart_cancel: CancellationToken,
    client_cancel: CancellationToken,
    initial_backoff: std::time::Duration,
) {
    let mut backoff = initial_backoff;
    loop {
        let attempt = tokio::select! {
            _ = restart_cancel.cancelled() => return,
            _ = client_cancel.cancelled() => return,
            result = negotiate_and_subscribe(&rpc, &runtime, protocol_v1, true) => result,
        };
        match attempt {
            Ok(()) => {
                app_info!(
                    "channel",
                    "imessage",
                    "Restarted imsg protocol and watch state recovered"
                );
                return;
            }
            Err(error) => {
                runtime.lock().await.degraded_error = Some(error.to_string());
                app_warn!(
                    "channel",
                    "imessage",
                    "Restarted imsg but protocol/watch recovery failed; retrying in {}s: {}",
                    backoff.as_secs_f64(),
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
            }
        }
        tokio::select! {
            _ = restart_cancel.cancelled() => return,
            _ = client_cancel.cancelled() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(std::time::Duration::from_secs(RESTART_BACKOFF_MAX_SECS));
    }
}

async fn recovery_loop(
    rpc: RpcHandle,
    account_id: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
    runtime: Arc<Mutex<RuntimeState>>,
    deduper: Arc<Mutex<InboundDeduper>>,
    cancel: CancellationToken,
    client_cancel: CancellationToken,
    recovery_rx: mpsc::Receiver<i64>,
) {
    recovery_loop_with_backoff(
        rpc,
        account_id,
        inbound_tx,
        runtime,
        deduper,
        cancel,
        client_cancel,
        recovery_rx,
        std::time::Duration::from_secs(1),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn recovery_loop_with_backoff(
    rpc: RpcHandle,
    account_id: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
    runtime: Arc<Mutex<RuntimeState>>,
    deduper: Arc<Mutex<InboundDeduper>>,
    cancel: CancellationToken,
    client_cancel: CancellationToken,
    mut recovery_rx: mpsc::Receiver<i64>,
    initial_backoff: std::time::Duration,
) {
    loop {
        let resume_after = tokio::select! {
            _ = cancel.cancelled() => break,
            _ = client_cancel.cancelled() => break,
            value = recovery_rx.recv() => match value {
                Some(value) => value,
                None => break,
            }
        };
        let mut cursor = resume_after;
        let mut backoff = initial_backoff;
        loop {
            let recovery = async {
                loop {
                    let page = rpc
                        .call(
                            "messages.after",
                            serde_json::json!({
                                "since_rowid": cursor,
                                "limit": 500,
                                "attachments": true,
                            }),
                        )
                        .await?;
                    let messages =
                        page.get("messages")
                            .and_then(Value::as_array)
                            .ok_or_else(|| {
                                IMessageRpcError::transport(
                                    "messages.after result omitted messages array",
                                )
                            })?;
                    for message in messages {
                        if !deliver_inbound_value(
                            message,
                            &account_id,
                            &inbound_tx,
                            &runtime,
                            &deduper,
                        )
                        .await
                        {
                            return Err(IMessageRpcError::transport(
                                "iMessage inbound channel closed during catch-up",
                            ));
                        }
                    }
                    cursor = page
                        .get("next_rowid")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| {
                            IMessageRpcError::transport("messages.after result omitted next_rowid")
                        })?;
                    runtime.lock().await.last_rowid = cursor;
                    if !page
                        .get("has_more")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
                watch_subscribe(&rpc, Some(cursor)).await?;
                Ok::<(), IMessageRpcError>(())
            }
            .await;
            match recovery {
                Ok(()) => {
                    runtime.lock().await.degraded_error = None;
                    app_info!(
                        "channel",
                        "imessage",
                        "iMessage watch overflow catch-up completed at rowid {}",
                        cursor
                    );
                    break;
                }
                Err(error) => {
                    runtime.lock().await.degraded_error = Some(error.to_string());
                    app_warn!(
                        "channel",
                        "imessage",
                        "iMessage watch overflow catch-up failed; retrying in {}s: {}",
                        backoff.as_secs_f64(),
                        ha_core::logging::redact_sensitive(&error.to_string())
                    );
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = client_cancel.cancelled() => return,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(std::time::Duration::from_secs(RESTART_BACKOFF_MAX_SECS));
        }
    }
}

fn build_send_params(
    target: &str,
    text: Option<&str>,
    file: Option<&str>,
    reply_to: Option<&str>,
) -> Value {
    let mut params = serde_json::Map::new();
    let trimmed_target = target.trim();

    let numeric_chat_id = (!trimmed_target.is_empty()
        && trimmed_target.chars().all(|c| c.is_ascii_digit()))
    .then(|| trimmed_target.parse::<i64>())
    .and_then(Result::ok);

    if let Some(chat_id) = numeric_chat_id {
        params.insert("chat_id".to_string(), Value::Number(chat_id.into()));
    } else if trimmed_target.starts_with("iMessage;") || trimmed_target.starts_with("SMS;") {
        params.insert(
            "chat_guid".to_string(),
            Value::String(trimmed_target.to_string()),
        );
    } else {
        params.insert("to".to_string(), Value::String(trimmed_target.to_string()));
        params.insert("service".to_string(), Value::String("auto".to_string()));
    }

    if let Some(text) = text.map(str::trim).filter(|s| !s.is_empty()) {
        params.insert("text".to_string(), Value::String(text.to_string()));
    }
    if let Some(file) = file.map(str::trim).filter(|s| !s.is_empty()) {
        params.insert("file".to_string(), Value::String(file.to_string()));
    }
    if let Some(reply_id) = reply_to.map(str::trim).filter(|s| !s.is_empty()) {
        params.insert("reply_to".to_string(), Value::String(reply_id.to_string()));
    }

    Value::Object(params)
}

#[cfg(test)]
mod tests {
    use super::{
        build_send_params, recovery_loop_with_backoff, restore_protocol_watch_until_ready,
        IMessageClient, IMessageDeliveryDisposition, IMessageRpcError, InboundDeduper, RpcHandle,
        RuntimeState,
    };
    use ha_core::channel::types::ChatType;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{mpsc, Mutex};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn build_send_params_uses_chat_id_for_numeric_targets() {
        let params = build_send_params("42", Some("hi"), Some("/tmp/a.png"), None);
        assert_eq!(params["chat_id"].as_i64(), Some(42));
        assert_eq!(params["text"].as_str(), Some("hi"));
        assert_eq!(params["file"].as_str(), Some("/tmp/a.png"));
        assert!(params.get("service").is_none());
    }

    #[test]
    fn build_send_params_uses_chat_guid_for_messages_handles() {
        let params = build_send_params("iMessage;+;chat123", None, Some("/tmp/a.png"), None);
        assert_eq!(params["chat_guid"].as_str(), Some("iMessage;+;chat123"));
        assert!(params.get("to").is_none());
        assert!(params.get("service").is_none());
    }

    #[test]
    fn build_send_params_uses_direct_target_with_service_auto() {
        let params = build_send_params("+14155551212", Some("hi"), None, Some("GUID"));
        assert_eq!(params["to"].as_str(), Some("+14155551212"));
        assert_eq!(params["service"].as_str(), Some("auto"));
        assert_eq!(params["reply_to"].as_str(), Some("GUID"));
    }

    #[test]
    fn parse_notification_trusts_explicit_is_group_false() {
        let msg = IMessageClient::parse_notification(
            &serde_json::json!({
                "message": {
                    "id": 1,
                    "guid": "MSG-1",
                    "chat_id": 42,
                    "sender": "+14155551212",
                    "is_from_me": false,
                    "text": "hi",
                    "created_at": "2026-05-20T08:00:00Z",
                    "participants": ["a", "b", "c"],
                    "is_group": false
                }
            }),
            "acc",
        )
        .unwrap();

        assert_eq!(msg.chat_type, ChatType::Dm);
        assert_eq!(msg.chat_id, "42");
        assert_eq!(msg.chat_title, None);
    }

    #[test]
    fn parse_notification_trusts_explicit_is_group_true() {
        let msg = IMessageClient::parse_notification(
            &serde_json::json!({
                "message": {
                    "id": 2,
                    "guid": "MSG-2",
                    "chat_id": 43,
                    "sender": "+14155551212",
                    "is_from_me": false,
                    "text": "hi",
                    "created_at": "2026-05-20T08:00:00Z",
                    "participants": ["a"],
                    "is_group": true,
                    "chat_name": "Crew"
                }
            }),
            "acc",
        )
        .unwrap();

        assert_eq!(msg.chat_type, ChatType::Group);
        assert_eq!(msg.chat_id, "43");
        assert_eq!(msg.chat_title.as_deref(), Some("Crew"));
    }

    #[test]
    fn ambiguous_delivery_codes_are_never_retry_safe() {
        for (code, disposition) in [(-32001, "may_have_completed"), (-32004, "still_in_flight")] {
            let error = IMessageRpcError::from_wire(&serde_json::json!({
                "code": code,
                "message": "synthetic",
                "data": {
                    "retry_safe": true,
                    "disposition": disposition,
                }
            }));
            assert!(!error.retry_safe);
            assert_eq!(error.code, Some(code));
        }
    }

    #[test]
    fn not_started_is_retry_safe_only_when_sidecar_proves_it() {
        let error = IMessageRpcError::from_wire(&serde_json::json!({
            "code": -32603,
            "message": "synthetic",
            "data": {
                "retry_safe": true,
                "disposition": "not_started",
            }
        }));
        assert!(error.retry_safe);
        assert_eq!(error.disposition, IMessageDeliveryDisposition::NotStarted);
    }

    #[test]
    fn overflow_replay_deduplicates_guid_then_rowid() {
        let mut deduper = InboundDeduper::default();
        let by_guid = serde_json::json!({"id": 7, "guid": "G-7"});
        assert!(deduper.admit(&by_guid));
        assert!(!deduper.admit(&by_guid));

        let by_rowid = serde_json::json!({"id": 8});
        assert!(deduper.admit(&by_rowid));
        assert!(!deduper.admit(&by_rowid));
    }

    #[tokio::test]
    async fn restarted_watch_retries_until_subscription_recovers() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel(4);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let rpc = RpcHandle {
            stdin_tx,
            pending: pending.clone(),
            next_id: Arc::new(Mutex::new(1)),
        };
        let response_pending = pending.clone();
        let responder = tokio::spawn(async move {
            for attempt in 0..2 {
                let line = stdin_rx.recv().await.expect("watch.subscribe request");
                let request: Value = serde_json::from_str(&line).expect("valid JSON-RPC request");
                assert_eq!(request["method"].as_str(), Some("watch.subscribe"));
                let id = request["id"].as_str().expect("string request id");
                let sender = response_pending
                    .lock()
                    .await
                    .remove(id)
                    .expect("pending JSON-RPC response");
                let response = if attempt == 0 {
                    Err(IMessageRpcError::transport(
                        "transient watch.subscribe failure",
                    ))
                } else {
                    Ok(serde_json::json!({}))
                };
                sender.send(response).expect("RPC caller remains active");
            }
        });
        let runtime = Arc::new(Mutex::new(RuntimeState {
            degraded_error: Some("restart pending".to_string()),
            ..RuntimeState::default()
        }));

        tokio::time::timeout(
            Duration::from_secs(1),
            restore_protocol_watch_until_ready(
                rpc,
                runtime.clone(),
                false,
                CancellationToken::new(),
                CancellationToken::new(),
                Duration::from_millis(5),
            ),
        )
        .await
        .expect("watch recovery should complete after retry");
        responder.await.expect("response task");

        assert!(runtime.lock().await.degraded_error.is_none());
    }

    #[tokio::test]
    async fn overflow_recovery_retries_catch_up_and_resubscribe_after_transient_failure() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel(8);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let rpc = RpcHandle {
            stdin_tx,
            pending: pending.clone(),
            next_id: Arc::new(Mutex::new(1)),
        };
        let response_pending = pending.clone();
        let responder = tokio::spawn(async move {
            for attempt in 0..3 {
                let line = stdin_rx.recv().await.expect("overflow recovery request");
                let request: Value = serde_json::from_str(&line).expect("valid JSON-RPC request");
                let id = request["id"].as_str().expect("string request id");
                let sender = response_pending
                    .lock()
                    .await
                    .remove(id)
                    .expect("pending JSON-RPC response");
                match attempt {
                    0 => {
                        assert_eq!(request["method"].as_str(), Some("messages.after"));
                        sender
                            .send(Err(IMessageRpcError::transport(
                                "transient messages.after failure",
                            )))
                            .expect("RPC caller remains active");
                    }
                    1 => {
                        assert_eq!(request["method"].as_str(), Some("messages.after"));
                        assert_eq!(request["params"]["since_rowid"].as_i64(), Some(10));
                        sender
                            .send(Ok(serde_json::json!({
                                "messages": [],
                                "next_rowid": 12,
                                "has_more": false,
                            })))
                            .expect("RPC caller remains active");
                    }
                    _ => {
                        assert_eq!(request["method"].as_str(), Some("watch.subscribe"));
                        assert_eq!(request["params"]["since_rowid"].as_i64(), Some(12));
                        sender
                            .send(Ok(serde_json::json!({})))
                            .expect("RPC caller remains active");
                    }
                }
            }
        });
        let runtime = Arc::new(Mutex::new(RuntimeState {
            degraded_error: Some("overflow pending".to_string()),
            ..RuntimeState::default()
        }));
        let deduper = Arc::new(Mutex::new(InboundDeduper::default()));
        let (inbound_tx, _inbound_rx) = mpsc::channel(4);
        let (recovery_tx, recovery_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(recovery_loop_with_backoff(
            rpc,
            "account".to_string(),
            inbound_tx,
            runtime.clone(),
            deduper,
            cancel.clone(),
            CancellationToken::new(),
            recovery_rx,
            Duration::from_millis(5),
        ));
        recovery_tx.send(10).await.expect("queue overflow cursor");

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let state = runtime.lock().await;
                if state.degraded_error.is_none() && state.last_rowid == 12 {
                    break;
                }
                drop(state);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("overflow recovery should retry to completion");
        responder.await.expect("response task");
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("recovery loop should observe cancellation")
            .expect("recovery task");
    }
}
