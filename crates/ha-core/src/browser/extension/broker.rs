use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
#[cfg(all(not(unix), not(windows)))]
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use crate::browser::backend::{ObserveEntry, ObserveKind};

use super::events::{emit_control_stopped_for_scope, BrowserControlStoppedReason};
use super::registry;

const MAX_BROKER_MESSAGE_LEN: u32 = 1024 * 1024;
const MAX_CHUNKED_RESPONSE_LEN: usize = 64 * 1024 * 1024;
const MAX_CHUNKED_RESPONSE_CHUNKS: usize = 512;
const CHUNKED_RESPONSE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_BLOB_SIZE: u64 = 256 * 1024 * 1024;
const MAX_BLOB_CHUNKS: u64 = 4096;
const BLOB_TTL: Duration = Duration::from_secs(10 * 60);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
pub const EXPECTED_EXTENSION_PROTOCOL_VERSION: u32 = 1;

static GLOBAL_BROKER: OnceLock<Arc<BrowserExtensionBroker>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserBrokerDiscovery {
    pub protocol_version: u32,
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrokerStatus {
    pub running: bool,
    pub endpoint: Option<String>,
    pub extension_connected: bool,
    pub extension_protocol_version: Option<u32>,
    pub extension_version: Option<String>,
    pub active_connection_id: Option<u64>,
    pub connected_hosts: u64,
    pub last_error: Option<String>,
}

struct BrokerState {
    running: bool,
    endpoint: Option<String>,
    token: String,
    sender: Option<mpsc::UnboundedSender<Value>>,
    active_connection_id: Option<u64>,
    connected_hosts: u64,
    extension_connected: bool,
    extension_protocol_version: Option<u32>,
    extension_version: Option<String>,
    last_error: Option<String>,
}

impl Default for BrokerState {
    fn default() -> Self {
        Self {
            running: false,
            endpoint: None,
            token: uuid::Uuid::new_v4().to_string(),
            sender: None,
            active_connection_id: None,
            connected_hosts: 0,
            extension_connected: false,
            extension_protocol_version: None,
            extension_version: None,
            last_error: None,
        }
    }
}

type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;
type ChunkMap = Arc<Mutex<HashMap<String, ChunkAssembly>>>;
type BlobMap = Arc<Mutex<BlobStore>>;

pub struct BrowserExtensionBroker {
    state: RwLock<BrokerState>,
    pending: PendingMap,
    chunks: ChunkMap,
    blobs: BlobMap,
    connection_seq: AtomicU64,
    request_seq: AtomicU64,
}

impl BrowserExtensionBroker {
    pub fn global() -> Option<Arc<Self>> {
        GLOBAL_BROKER.get().cloned()
    }

    pub fn spawn_global() -> Arc<Self> {
        let broker = GLOBAL_BROKER.get_or_init(|| Arc::new(Self::new())).clone();
        let broker_for_task = broker.clone();
        tokio::spawn(async move {
            broker_for_task.run().await;
        });
        broker
    }

    fn new() -> Self {
        Self {
            state: RwLock::new(BrokerState::default()),
            pending: Arc::new(Mutex::new(HashMap::new())),
            chunks: Arc::new(Mutex::new(HashMap::new())),
            blobs: Arc::new(Mutex::new(BlobStore::default())),
            connection_seq: AtomicU64::new(1),
            request_seq: AtomicU64::new(1),
        }
    }

    pub async fn status(&self) -> BrokerStatus {
        let state = self.state.read().await;
        status_from_state(&state)
    }

    pub fn status_snapshot(&self) -> BrokerStatus {
        match self.state.try_read() {
            Ok(state) => status_from_state(&state),
            Err(_) => BrokerStatus {
                last_error: Some("Chrome Extension broker status is busy".to_string()),
                ..BrokerStatus::default()
            },
        }
    }

    pub async fn is_extension_connected(&self) -> bool {
        self.state.read().await.extension_connected
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = format!("core-{}", self.request_seq.fetch_add(1, Ordering::Relaxed));
        let sender = {
            let state = self.state.read().await;
            state
                .sender
                .clone()
                .ok_or_else(|| anyhow!("Chrome Extension is not connected"))?
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let message = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if sender.send(message).is_err() {
            let _ = self.pending.lock().await.remove(&id);
            let _ = self.chunks.lock().await.remove(&id);
            bail!("Chrome Extension connection is closed");
        }
        let response = match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => bail!("Chrome Extension response channel closed"),
            Err(_) => {
                let _ = self.pending.lock().await.remove(&id);
                let _ = self.chunks.lock().await.remove(&id);
                bail!("Chrome Extension call timed out: {method}");
            }
        };
        let response = if is_response_blob(&response) {
            let mut blobs = self.blobs.lock().await;
            blobs.take_completed_json(&response)?
        } else {
            response
        };
        if response.get("ok").and_then(Value::as_bool) == Some(false) {
            let msg = response
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Chrome Extension command failed");
            bail!("{msg}");
        }
        Ok(response.get("result").cloned().unwrap_or(response))
    }

    pub async fn take_blob_bytes(
        &self,
        descriptor: &Value,
        expected_purpose: &str,
        allowed_mimes: &[&str],
    ) -> Result<Vec<u8>> {
        let mut blobs = self.blobs.lock().await;
        blobs.take_completed_bytes(descriptor, expected_purpose, allowed_mimes)
    }

    async fn run(self: Arc<Self>) {
        if self.state.read().await.running {
            return;
        }
        #[cfg(unix)]
        self.run_unix().await;
        #[cfg(windows)]
        self.run_windows_pipe().await;
        #[cfg(all(not(unix), not(windows)))]
        self.run_tcp().await;
    }

    #[cfg(unix)]
    async fn run_unix(self: Arc<Self>) {
        let socket_path = match crate::paths::browser_extension_broker_socket_path() {
            Ok(path) => path,
            Err(e) => {
                self.set_error(format!("broker socket path failed: {e:#}"))
                    .await;
                return;
            }
        };
        if let Some(parent) = socket_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.set_error(format!("broker socket dir create failed: {e}"))
                    .await;
                return;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        match std::fs::remove_file(&socket_path) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                self.set_error(format!("broker stale socket cleanup failed: {e}"))
                    .await;
                return;
            }
        }
        let listener = match UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            Err(e) => {
                self.set_error(format!("broker unix socket bind failed: {e}"))
                    .await;
                return;
            }
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));
        }
        let endpoint = format!("unix:{}", socket_path.to_string_lossy());
        if let Err(e) = self.publish_discovery(endpoint.clone()).await {
            self.set_error(format!("broker discovery write failed: {e:#}"))
                .await;
            return;
        }
        self.mark_running(endpoint.clone()).await;
        app_info!(
            "browser",
            "extension_broker",
            "Chrome Extension broker listening on {}",
            endpoint
        );

        loop {
            match listener.accept().await {
                Ok((stream, _peer)) => {
                    if let Err(e) = validate_unix_peer_uid(&stream) {
                        self.set_error(format!("broker peer uid validation failed: {e:#}"))
                            .await;
                        continue;
                    }
                    let connection_id = self.connection_seq.fetch_add(1, Ordering::Relaxed);
                    let broker = self.clone();
                    tokio::spawn(async move {
                        let (reader, writer) = stream.into_split();
                        if let Err(e) = broker
                            .clone()
                            .handle_connection(connection_id, reader, writer)
                            .await
                        {
                            broker
                                .set_error(format!(
                                    "broker connection {connection_id} failed: {e:#}"
                                ))
                                .await;
                        }
                    });
                }
                Err(e) => {
                    self.set_error(format!("broker accept failed: {e}")).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    #[cfg(windows)]
    async fn run_windows_pipe(self: Arc<Self>) {
        let pipe_name = windows_broker_pipe_name();
        let mut server = match create_windows_pipe_server(&pipe_name, true) {
            Ok(server) => server,
            Err(e) => {
                self.set_error(format!("broker Windows named pipe create failed: {e:#}"))
                    .await;
                return;
            }
        };
        let endpoint = format!("pipe:{pipe_name}");
        if let Err(e) = self.publish_discovery(endpoint.clone()).await {
            self.set_error(format!("broker discovery write failed: {e:#}"))
                .await;
            return;
        }
        self.mark_running(endpoint.clone()).await;
        app_info!(
            "browser",
            "extension_broker",
            "Chrome Extension broker listening on {}",
            endpoint
        );

        loop {
            match server.connect().await {
                Ok(()) => {
                    let connected = server;
                    server = match create_windows_pipe_server(&pipe_name, false) {
                        Ok(next) => next,
                        Err(e) => {
                            self.set_error(format!(
                                "broker Windows named pipe next-instance create failed: {e:#}"
                            ))
                            .await;
                            break;
                        }
                    };
                    if let Err(e) = validate_windows_pipe_peer_sid(&connected) {
                        self.set_error(format!("broker peer SID validation failed: {e:#}"))
                            .await;
                        let _ = connected.disconnect();
                        continue;
                    }
                    let connection_id = self.connection_seq.fetch_add(1, Ordering::Relaxed);
                    let broker = self.clone();
                    tokio::spawn(async move {
                        let (reader, writer) = tokio::io::split(connected);
                        if let Err(e) = broker
                            .clone()
                            .handle_connection(connection_id, reader, writer)
                            .await
                        {
                            broker
                                .set_error(format!(
                                    "broker connection {connection_id} failed: {e:#}"
                                ))
                                .await;
                        }
                    });
                }
                Err(e) => {
                    self.set_error(format!("broker Windows named pipe accept failed: {e}"))
                        .await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    #[cfg(all(not(unix), not(windows)))]
    async fn run_tcp(self: Arc<Self>) {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(e) => {
                self.set_error(format!("broker bind failed: {e}")).await;
                return;
            }
        };
        let addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(e) => {
                self.set_error(format!("broker local_addr failed: {e}"))
                    .await;
                return;
            }
        };
        let endpoint = format!("tcp:{addr}");
        if let Err(e) = self.publish_discovery(endpoint.clone()).await {
            self.set_error(format!("broker discovery write failed: {e:#}"))
                .await;
            return;
        }
        self.mark_running(endpoint.clone()).await;
        app_info!(
            "browser",
            "extension_broker",
            "Chrome Extension broker listening on {}",
            endpoint
        );

        loop {
            match listener.accept().await {
                Ok((stream, _peer)) => {
                    let connection_id = self.connection_seq.fetch_add(1, Ordering::Relaxed);
                    let broker = self.clone();
                    tokio::spawn(async move {
                        let (reader, writer) = stream.into_split();
                        if let Err(e) = broker
                            .clone()
                            .handle_connection(connection_id, reader, writer)
                            .await
                        {
                            broker
                                .set_error(format!(
                                    "broker connection {connection_id} failed: {e:#}"
                                ))
                                .await;
                        }
                    });
                }
                Err(e) => {
                    self.set_error(format!("broker accept failed: {e}")).await;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    async fn mark_running(&self, endpoint: String) {
        let mut state = self.state.write().await;
        state.running = true;
        state.endpoint = Some(endpoint);
        state.last_error = None;
    }

    async fn publish_discovery(&self, endpoint: String) -> Result<()> {
        let path = crate::paths::browser_extension_broker_discovery_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let token = self.state.read().await.token.clone();
        let discovery = BrowserBrokerDiscovery {
            protocol_version: 1,
            endpoint,
            token,
            pid: std::process::id(),
        };
        let bytes = serde_json::to_vec_pretty(&discovery)?;
        crate::platform::write_atomic(&path, &bytes)
            .with_context(|| format!("writing browser broker discovery {}", path.display()))?;
        Ok(())
    }

    async fn handle_connection(
        self: Arc<Self>,
        connection_id: u64,
        mut reader: impl AsyncRead + Unpin + Send + 'static,
        writer: impl AsyncWrite + Unpin + Send + 'static,
    ) -> Result<()> {
        let hello = read_broker_message(&mut reader)
            .await?
            .ok_or_else(|| anyhow!("host disconnected before hello"))?;
        self.validate_host_hello(&hello).await?;

        let (tx, rx) = mpsc::unbounded_channel();
        {
            let mut state = self.state.write().await;
            state.sender = Some(tx);
            state.active_connection_id = Some(connection_id);
            state.connected_hosts = state.connected_hosts.saturating_add(1);
            state.extension_connected = false;
            state.last_error = None;
        }
        tokio::spawn(write_loop(writer, rx));

        app_info!(
            "browser",
            "extension_broker",
            "Native host connected (connection_id={})",
            connection_id
        );

        while let Some(message) = read_broker_message(&mut reader).await? {
            self.handle_host_message(&message).await?;
        }

        {
            let mut state = self.state.write().await;
            if state.active_connection_id == Some(connection_id) {
                state.sender = None;
                state.active_connection_id = None;
                state.extension_connected = false;
                state.extension_protocol_version = None;
                state.extension_version = None;
            }
        }
        app_info!(
            "browser",
            "extension_broker",
            "Native host disconnected (connection_id={})",
            connection_id
        );
        Ok(())
    }

    async fn validate_host_hello(&self, message: &Value) -> Result<()> {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if method != "host.hello" {
            bail!("expected host.hello, got {method}");
        }
        let token = message.get("token").and_then(Value::as_str).unwrap_or("");
        let expected = self.state.read().await.token.clone();
        if token != expected {
            bail!("native host token mismatch");
        }
        Ok(())
    }

    async fn handle_host_message(&self, message: &Value) -> Result<()> {
        if is_response_chunk(message) {
            self.handle_response_chunk(message).await;
            return Ok(());
        }

        if let Some(id) = message.get("id").and_then(Value::as_str) {
            if let Some(tx) = self.pending.lock().await.remove(id) {
                let _ = self.chunks.lock().await.remove(id);
                let _ = tx.send(message.clone());
                return Ok(());
            }
        }

        match message.get("method").and_then(Value::as_str) {
            Some("extension.hello") | Some("hello") => {
                let protocol_version = extension_protocol_version(message);
                let extension_version = extension_version(message);
                let mut state = self.state.write().await;
                state.extension_connected = true;
                state.extension_protocol_version = protocol_version;
                state.extension_version = extension_version;
                state.last_error = None;
                drop(state);
                self.respond_to_extension(
                    message,
                    json!({
                        "ok": true,
                        "type": "hello_ack",
                        "protocolVersion": EXPECTED_EXTENSION_PROTOCOL_VERSION,
                        "coreConnected": true
                    }),
                )
                .await;
                Ok(())
            }
            Some("extension.status") | Some("status") => {
                let mut state = self.state.write().await;
                state.extension_connected = true;
                state.last_error = None;
                let status = status_from_state(&state);
                drop(state);
                self.respond_to_extension(
                    message,
                    json!({
                        "ok": true,
                        "type": "status",
                        "protocolVersion": 1,
                        "coreConnected": true,
                        "broker": status
                    }),
                )
                .await;
                Ok(())
            }
            Some("extension.user_stop") | Some("user_stop") => {
                let Some(tab_id) = message
                    .get("payload")
                    .or_else(|| message.get("params"))
                    .and_then(|payload| payload.get("tabId"))
                    .and_then(Value::as_i64)
                else {
                    self.respond_to_extension(
                        message,
                        json!({
                            "ok": false,
                            "error": { "message": "extension.user_stop requires integer tabId" }
                        }),
                    )
                    .await;
                    return Ok(());
                };

                let removed = registry::remove_tab_from_all_scopes(tab_id);
                for lease in &removed {
                    emit_control_stopped_for_scope(
                        lease.tab_id,
                        lease.scope.clone(),
                        BrowserControlStoppedReason::UserStop,
                        false,
                    );
                }
                let user_leases = removed
                    .iter()
                    .filter(|lease| lease.owner_kind == registry::TabOwnerKind::User)
                    .count();
                let agent_leases = removed.len().saturating_sub(user_leases);
                app_info!(
                    "browser",
                    "extension_broker",
                    "User stopped Chrome tab {} control; removed {} lease(s) (user={}, agent={})",
                    tab_id,
                    removed.len(),
                    user_leases,
                    agent_leases
                );
                self.respond_to_extension(
                    message,
                    json!({
                        "ok": true,
                        "type": "user_stop_ack",
                        "tabId": tab_id,
                        "removedLeases": removed.len()
                    }),
                )
                .await;
                Ok(())
            }
            Some("extension.download_completed") | Some("download_completed") => {
                let payload = message.get("payload").or_else(|| message.get("params"));
                match payload.map(handle_download_completed_payload) {
                    Some(Ok(result)) => {
                        self.respond_to_extension(
                            message,
                            json!({
                                "ok": true,
                                "type": "download_completed_ack",
                                "result": result
                            }),
                        )
                        .await;
                    }
                    Some(Err(e)) => {
                        push_download_observe(
                            "policy_error",
                            format!("download landing policy failed: {e}"),
                            None,
                        );
                        self.respond_to_extension(
                            message,
                            json!({
                                "ok": false,
                                "error": { "message": format!("download landing policy failed: {e}") }
                            }),
                        )
                        .await;
                    }
                    None => {
                        self.respond_to_extension(
                            message,
                            json!({
                                "ok": false,
                                "error": { "message": "extension.download_completed requires payload" }
                            }),
                        )
                        .await;
                    }
                }
                Ok(())
            }
            Some(method @ ("blob.begin" | "blob.chunk" | "blob.end")) => {
                let payload = message
                    .get("payload")
                    .or_else(|| message.get("params"))
                    .unwrap_or(message);
                let result = {
                    let mut blobs = self.blobs.lock().await;
                    blobs.handle_message(method, payload)
                };
                match result {
                    Ok(result) => {
                        self.respond_to_extension(
                            message,
                            json!({
                                "ok": true,
                                "type": format!("{method}.ack"),
                                "result": result
                            }),
                        )
                        .await;
                    }
                    Err(e) => {
                        self.respond_to_extension(
                            message,
                            json!({
                                "ok": false,
                                "error": { "message": format!("Chrome Extension blob failed: {e}") }
                            }),
                        )
                        .await;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn respond_to_extension(&self, request: &Value, mut response: Value) {
        let Some(id) = request.get("id").cloned() else {
            return;
        };
        response["id"] = id;
        let sender = self.state.read().await.sender.clone();
        if let Some(sender) = sender {
            let _ = sender.send(response);
        }
    }

    async fn set_error(&self, message: String) {
        app_warn!("browser", "extension_broker", "{}", message);
        let mut state = self.state.write().await;
        state.last_error = Some(message);
    }

    async fn handle_response_chunk(&self, message: &Value) {
        let Some(id) = message.get("id").and_then(Value::as_str) else {
            return;
        };
        let assembled = {
            let mut chunks = self.chunks.lock().await;
            assemble_response_chunk(&mut chunks, message)
        };
        match assembled {
            Ok(Some(response)) => {
                if let Some(tx) = self.pending.lock().await.remove(id) {
                    let _ = tx.send(response);
                }
            }
            Ok(None) => {}
            Err(e) => {
                let _ = self.chunks.lock().await.remove(id);
                if let Some(tx) = self.pending.lock().await.remove(id) {
                    let _ = tx.send(json!({
                        "id": id,
                        "ok": false,
                        "error": { "message": format!("Chrome Extension chunked response failed: {e}") }
                    }));
                }
            }
        }
    }
}

#[cfg(unix)]
fn validate_unix_peer_uid(stream: &tokio::net::UnixStream) -> Result<()> {
    match unix_peer_uid(stream)? {
        Some(peer_uid) => validate_peer_uid_values(peer_uid, current_euid()),
        None => {
            app_warn!(
                "browser",
                "extension_broker",
                "Unix peer uid validation is unsupported on this platform"
            );
            Ok(())
        }
    }
}

#[cfg(all(unix, target_os = "linux"))]
fn unix_peer_uid(stream: &tokio::net::UnixStream) -> Result<Option<u32>> {
    use std::mem::MaybeUninit;
    use std::os::unix::io::AsRawFd;

    let mut cred = MaybeUninit::<libc::ucred>::uninit();
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            cred.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("getsockopt SO_PEERCRED failed");
    }
    let cred = unsafe { cred.assume_init() };
    Ok(Some(cred.uid))
}

#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )
))]
fn unix_peer_uid(stream: &tokio::net::UnixStream) -> Result<Option<u32>> {
    use std::os::unix::io::AsRawFd;

    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("getpeereid failed");
    }
    Ok(Some(uid))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
fn unix_peer_uid(_stream: &tokio::net::UnixStream) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(unix)]
fn current_euid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn validate_peer_uid_values(peer_uid: u32, current_uid: u32) -> Result<()> {
    if peer_uid != current_uid {
        bail!("native host peer uid {peer_uid} does not match current uid {current_uid}");
    }
    Ok(())
}

#[cfg(windows)]
fn windows_broker_pipe_name() -> String {
    format!(
        r"\\.\pipe\hope-agent-browser-extension-{}",
        std::process::id()
    )
}

#[cfg(windows)]
fn create_windows_pipe_server(path: &str, first_instance: bool) -> Result<NamedPipeServer> {
    let mut security = WindowsPipeSecurity::for_current_user()
        .context("building current-user named pipe security descriptor")?;
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first_instance)
        .reject_remote_clients(true)
        .max_instances(16);
    let server = unsafe {
        options
            .create_with_security_attributes_raw(path, security.as_mut_ptr())
            .with_context(|| format!("creating Windows named pipe broker {path}"))?
    };
    Ok(server)
}

#[cfg(windows)]
fn validate_windows_pipe_peer_sid(pipe: &NamedPipeServer) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{EqualSid, TOKEN_QUERY};
    use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

    unsafe {
        if ImpersonateNamedPipeClient(pipe.as_raw_handle() as HANDLE) == 0 {
            return Err(std::io::Error::last_os_error())
                .context("impersonating Windows named pipe client");
        }
    }
    let _revert = WindowsRevertImpersonation;

    let mut client_token = 0;
    unsafe {
        if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 0, &mut client_token) == 0 {
            return Err(std::io::Error::last_os_error())
                .context("opening impersonated named pipe client token");
        }
    }
    let client_token = WindowsHandle(client_token);
    let client_sid = token_user_sid(client_token.0).context("reading named pipe client SID")?;
    let current_sid = current_process_user_sid().context("reading current process SID")?;
    let matches = unsafe { EqualSid(sid_ptr(&client_sid), sid_ptr(&current_sid)) != 0 };
    if !matches {
        bail!("native host client SID does not match current user SID");
    }
    _revert.revert()?;
    Ok(())
}

#[cfg(windows)]
struct WindowsPipeSecurity {
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    attrs: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
}

#[cfg(windows)]
impl WindowsPipeSecurity {
    fn for_current_user() -> Result<Self> {
        use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
        use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

        let sid = current_user_sid_string()?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let sddl = wide_null(&sddl);
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(std::io::Error::last_os_error())
                    .context("converting named pipe DACL SDDL");
            }
        }
        Ok(Self {
            descriptor,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    fn as_mut_ptr(&mut self) -> *mut std::ffi::c_void {
        (&mut self.attrs as *mut _) as *mut std::ffi::c_void
    }
}

#[cfg(windows)]
impl Drop for WindowsPipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                let _ = windows_sys::Win32::Foundation::LocalFree(self.descriptor as _);
            }
        }
    }
}

#[cfg(windows)]
struct WindowsHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsHandle {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct WindowsRevertImpersonation;

#[cfg(windows)]
impl WindowsRevertImpersonation {
    fn revert(self) -> Result<()> {
        let ok = unsafe { windows_sys::Win32::Security::RevertToSelf() };
        std::mem::forget(self);
        if ok == 0 {
            return Err(std::io::Error::last_os_error())
                .context("reverting named pipe impersonation");
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsRevertImpersonation {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::Security::RevertToSelf();
        }
    }
}

#[cfg(windows)]
fn current_process_user_sid() -> Result<Vec<u8>> {
    use windows_sys::Win32::Security::TOKEN_QUERY;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = 0;
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error()).context("opening current process token");
        }
    }
    let token = WindowsHandle(token);
    token_user_sid(token.0)
}

#[cfg(windows)]
fn current_user_sid_string() -> Result<String> {
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    let sid = current_process_user_sid()?;
    let mut raw = std::ptr::null_mut();
    unsafe {
        if ConvertSidToStringSidW(sid_ptr(&sid), &mut raw) == 0 {
            return Err(std::io::Error::last_os_error()).context("converting current user SID");
        }
    }
    let result = unsafe { pwstr_to_string(raw) };
    unsafe {
        let _ = windows_sys::Win32::Foundation::LocalFree(raw as _);
    }
    Ok(result)
}

#[cfg(windows)]
fn token_user_sid(token: windows_sys::Win32::Foundation::HANDLE) -> Result<Vec<u8>> {
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser};

    let mut len = 0u32;
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
    }
    if len == 0 {
        return Err(std::io::Error::last_os_error()).context("querying token user SID length");
    }
    let mut buffer = vec![0u8; len as usize];
    let ok =
        unsafe { GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("querying token user SID");
    }
    Ok(buffer)
}

#[cfg(windows)]
fn sid_ptr(token_user: &[u8]) -> windows_sys::Win32::Foundation::PSID {
    let user = token_user.as_ptr() as *const windows_sys::Win32::Security::TOKEN_USER;
    unsafe { (*user).User.Sid }
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
unsafe fn pwstr_to_string(ptr: windows_sys::core::PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) }.as_ref())
}

#[derive(Debug)]
struct ChunkAssembly {
    chunks: Vec<Option<String>>,
    received: usize,
    total_len: usize,
    sha256: Option<String>,
    created_at: Instant,
    updated_at: Instant,
}

fn is_response_chunk(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some("response.chunk")
}

fn is_response_blob(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some("response.blob")
}

fn assemble_response_chunk(
    chunks: &mut HashMap<String, ChunkAssembly>,
    message: &Value,
) -> Result<Option<Value>> {
    assemble_response_chunk_at(chunks, message, Instant::now())
}

fn assemble_response_chunk_at(
    chunks: &mut HashMap<String, ChunkAssembly>,
    message: &Value,
    now: Instant,
) -> Result<Option<Value>> {
    prune_expired_chunk_assemblies(chunks, now);
    let id = message
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("response chunk is missing id"))?;
    let index = message
        .get("index")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("response chunk is missing index"))? as usize;
    let total = message
        .get("total")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("response chunk is missing total"))? as usize;
    let data = message
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("response chunk is missing data"))?;
    let sha256 = message
        .get("sha256")
        .and_then(Value::as_str)
        .map(normalize_sha256_hex)
        .transpose()?;
    if total == 0 || total > MAX_CHUNKED_RESPONSE_CHUNKS {
        bail!("response chunk total {total} is out of range");
    }
    if index >= total {
        bail!("response chunk index {index} is out of range for total {total}");
    }

    let assembly = chunks
        .entry(id.to_string())
        .or_insert_with(|| ChunkAssembly {
            chunks: vec![None; total],
            received: 0,
            total_len: 0,
            sha256: sha256.clone(),
            created_at: now,
            updated_at: now,
        });
    if assembly.chunks.len() != total {
        bail!(
            "response chunk total changed from {} to {}",
            assembly.chunks.len(),
            total
        );
    }
    if let Some(expected) = &assembly.sha256 {
        if let Some(actual) = &sha256 {
            if expected != actual {
                bail!("response chunk sha256 changed during assembly");
            }
        }
    } else if let Some(actual) = sha256 {
        assembly.sha256 = Some(actual);
    }
    assembly.updated_at = now;
    match &assembly.chunks[index] {
        Some(existing) if existing != data => {
            bail!("response chunk {index} was received twice with different data");
        }
        Some(_) => {}
        None => {
            assembly.received += 1;
            assembly.total_len = assembly.total_len.saturating_add(data.len());
            if assembly.total_len > MAX_CHUNKED_RESPONSE_LEN {
                bail!(
                    "chunked response length {} exceeds max {}",
                    assembly.total_len,
                    MAX_CHUNKED_RESPONSE_LEN
                );
            }
            assembly.chunks[index] = Some(data.to_string());
        }
    }

    if assembly.received != total {
        return Ok(None);
    }

    let assembly = chunks
        .remove(id)
        .ok_or_else(|| anyhow!("completed response chunk assembly disappeared"))?;
    let mut encoded = String::with_capacity(assembly.total_len);
    for chunk in assembly.chunks {
        let Some(chunk) = chunk else {
            bail!("chunked response completed with missing chunk");
        };
        encoded.push_str(&chunk);
    }
    if let Some(expected) = &assembly.sha256 {
        let actual = sha256_hex(encoded.as_bytes());
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!("chunked response sha256 mismatch");
        }
    }
    serde_json::from_str(&encoded)
        .map(Some)
        .context("decoding chunked response JSON")
}

fn prune_expired_chunk_assemblies(chunks: &mut HashMap<String, ChunkAssembly>, now: Instant) {
    chunks.retain(|_, assembly| {
        now.duration_since(assembly.updated_at) <= CHUNKED_RESPONSE_TTL
            && now.duration_since(assembly.created_at) <= CHUNKED_RESPONSE_TTL
    });
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_sha256_hex(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("response chunk sha256 must be 64 hex characters");
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn handle_download_completed_payload(payload: &Value) -> Result<Value> {
    let download_id = payload
        .get("id")
        .or_else(|| payload.get("downloadId"))
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("download payload is missing integer id"))?;
    let filename = payload
        .get("filename")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("download payload is missing filename"))?;
    let url = payload
        .get("finalUrl")
        .or_else(|| payload.get("url"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let source = PathBuf::from(filename);
    let dest_dir = crate::paths::browser_downloads_dir()?;
    let dest = move_download_to_managed_dir(&source, &dest_dir)?;
    push_download_observe(
        "managed",
        format!(
            "download {download_id} moved to Hope Agent downloads: {}",
            dest.to_string_lossy()
        ),
        url.clone(),
    );
    Ok(json!({
        "downloadId": download_id,
        "managedPath": dest.to_string_lossy(),
        "url": url
    }))
}

fn move_download_to_managed_dir(source: &Path, dest_dir: &Path) -> Result<PathBuf> {
    if !source.is_absolute() {
        bail!("download filename must be absolute: {}", source.display());
    }
    if !source.is_file() {
        bail!(
            "download file does not exist or is not a regular file: {}",
            source.display()
        );
    }
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("creating browser downloads dir {}", dest_dir.display()))?;
    let source_canonical = source
        .canonicalize()
        .with_context(|| format!("canonicalizing download file {}", source.display()))?;
    let dest_dir_canonical = dest_dir.canonicalize().with_context(|| {
        format!(
            "canonicalizing browser downloads dir {}",
            dest_dir.display()
        )
    })?;
    if source_canonical
        .parent()
        .is_some_and(|parent| parent == dest_dir_canonical)
    {
        return Ok(source_canonical);
    }
    let file_name = source
        .file_name()
        .ok_or_else(|| anyhow!("download filename has no file name: {}", source.display()))?;
    let dest = uniquify_path(dest_dir.join(file_name));
    match std::fs::rename(source, &dest) {
        Ok(_) => Ok(dest),
        Err(e) if crate::platform::is_cross_device_rename(&e) => {
            std::fs::copy(source, &dest).with_context(|| {
                format!(
                    "copying download {} to {}",
                    source.display(),
                    dest.display()
                )
            })?;
            std::fs::remove_file(source)
                .with_context(|| format!("removing original download {}", source.display()))?;
            Ok(dest)
        }
        Err(e) => Err(e)
            .with_context(|| format!("moving download {} to {}", source.display(), dest.display())),
    }
}

fn uniquify_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("download");
    let ext = path.extension().and_then(|s| s.to_str());
    for index in 1..10_000 {
        let file_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem} ({index}).{ext}"),
            _ => format!("{stem} ({index})"),
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

fn push_download_observe(level: &str, text: String, url: Option<String>) {
    crate::browser::observe_buffer::push(
        ObserveKind::Downloads,
        ObserveEntry {
            at: chrono::Utc::now().timestamp_millis(),
            level: level.to_string(),
            text,
            url,
        },
    );
}

#[derive(Debug)]
struct BlobStore {
    root_dir: PathBuf,
    blobs: HashMap<String, BlobAssembly>,
    completed: HashMap<String, CompletedBlob>,
}

#[derive(Debug)]
struct BlobAssembly {
    blob_id: String,
    path: PathBuf,
    mime: Option<String>,
    purpose: Option<String>,
    total_size: u64,
    sha256: String,
    received_bytes: u64,
    chunks: HashSet<u64>,
    ranges: Vec<(u64, u64)>,
    created_at: Instant,
    updated_at: Instant,
}

#[derive(Debug)]
struct CompletedBlob {
    path: PathBuf,
    mime: Option<String>,
    purpose: Option<String>,
    total_size: u64,
    sha256: String,
    created_at: Instant,
}

impl Default for BlobStore {
    fn default() -> Self {
        let root_dir = crate::paths::browser_extension_blobs_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("hope-agent-browser-extension-blobs"));
        Self::new(root_dir)
    }
}

impl BlobStore {
    fn new(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            blobs: HashMap::new(),
            completed: HashMap::new(),
        }
    }

    fn handle_message(&mut self, method: &str, payload: &Value) -> Result<Value> {
        let now = Instant::now();
        self.prune_expired(now);
        match method {
            "blob.begin" => self.begin(payload, now),
            "blob.chunk" => self.chunk(payload, now),
            "blob.end" => self.end(payload, now),
            _ => bail!("unsupported blob method {method}"),
        }
    }

    fn begin(&mut self, payload: &Value, now: Instant) -> Result<Value> {
        let blob_id = parse_blob_id(payload)?;
        let total_size = payload
            .get("totalSize")
            .or_else(|| payload.get("total_size"))
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("blob.begin requires totalSize"))?;
        if total_size == 0 || total_size > MAX_BLOB_SIZE {
            bail!("blob totalSize {total_size} is out of range");
        }
        let sha256 = payload
            .get("sha256")
            .and_then(Value::as_str)
            .map(normalize_sha256_hex)
            .transpose()?
            .ok_or_else(|| anyhow!("blob.begin requires sha256"))?;
        std::fs::create_dir_all(&self.root_dir).with_context(|| {
            format!(
                "creating browser extension blob dir {}",
                self.root_dir.display()
            )
        })?;
        if let Some(old) = self.blobs.remove(&blob_id) {
            let _ = std::fs::remove_file(old.path);
        }
        if let Some(old) = self.completed.remove(&blob_id) {
            let _ = std::fs::remove_file(old.path);
        }
        let path = self.root_dir.join(format!("{blob_id}.part"));
        let _ = std::fs::remove_file(&path);
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("creating blob file {}", path.display()))?;
        file.set_len(total_size)
            .with_context(|| format!("sizing blob file {}", path.display()))?;
        self.blobs.insert(
            blob_id.clone(),
            BlobAssembly {
                blob_id: blob_id.clone(),
                path: path.clone(),
                mime: payload
                    .get("mime")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                purpose: payload
                    .get("purpose")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                total_size,
                sha256,
                received_bytes: 0,
                chunks: HashSet::new(),
                ranges: Vec::new(),
                created_at: now,
                updated_at: now,
            },
        );
        Ok(json!({
            "blobId": blob_id,
            "path": path.to_string_lossy(),
            "totalSize": total_size
        }))
    }

    fn chunk(&mut self, payload: &Value, now: Instant) -> Result<Value> {
        let blob_id = parse_blob_id(payload)?;
        let assembly = self
            .blobs
            .get_mut(&blob_id)
            .ok_or_else(|| anyhow!("unknown blobId {blob_id}"))?;
        assembly.updated_at = now;
        let index = payload
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("blob.chunk requires index"))?;
        if index >= MAX_BLOB_CHUNKS {
            bail!("blob chunk index {index} is out of range");
        }
        if assembly.chunks.contains(&index) {
            bail!("blob chunk index {index} was received twice");
        }
        let offset = payload
            .get("offset")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("blob.chunk requires offset"))?;
        let data_b64 = payload
            .get("base64")
            .or_else(|| payload.get("data"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("blob.chunk requires base64"))?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(data_b64.as_bytes())
            .context("decoding blob chunk base64")?;
        if data.is_empty() {
            bail!("blob chunk data is empty");
        }
        if let Some(expected) = payload.get("sha256").and_then(Value::as_str) {
            let expected = normalize_sha256_hex(expected)?;
            let actual = sha256_hex(&data);
            if expected != actual {
                bail!("blob chunk sha256 mismatch");
            }
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| anyhow!("blob chunk offset overflow"))?;
        if end > assembly.total_size {
            bail!("blob chunk exceeds totalSize");
        }
        if assembly
            .ranges
            .iter()
            .any(|(start, existing_end)| offset < *existing_end && end > *start)
        {
            bail!("blob chunk range overlaps an existing chunk");
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&assembly.path)
            .with_context(|| format!("opening blob file {}", assembly.path.display()))?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&data)?;
        file.flush()?;
        assembly.chunks.insert(index);
        assembly.ranges.push((offset, end));
        assembly.received_bytes = assembly.received_bytes.saturating_add(data.len() as u64);
        Ok(json!({
            "blobId": blob_id,
            "receivedBytes": assembly.received_bytes,
            "totalSize": assembly.total_size
        }))
    }

    fn end(&mut self, payload: &Value, now: Instant) -> Result<Value> {
        let blob_id = parse_blob_id(payload)?;
        let Some(assembly) = self.blobs.get_mut(&blob_id) else {
            bail!("unknown blobId {blob_id}");
        };
        assembly.updated_at = now;
        let total_chunks = payload
            .get("totalChunks")
            .or_else(|| payload.get("total_chunks"))
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("blob.end requires totalChunks"))?;
        if total_chunks as usize != assembly.chunks.len() {
            bail!(
                "blob.end totalChunks {} does not match received {}",
                total_chunks,
                assembly.chunks.len()
            );
        }
        let expected = payload
            .get("sha256")
            .and_then(Value::as_str)
            .map(normalize_sha256_hex)
            .transpose()?
            .unwrap_or_else(|| assembly.sha256.clone());
        if expected != assembly.sha256 {
            bail!("blob.end sha256 changed from blob.begin");
        }
        if assembly.received_bytes != assembly.total_size {
            bail!(
                "blob received {} bytes but expected {}",
                assembly.received_bytes,
                assembly.total_size
            );
        }
        let actual = sha256_file_hex(&assembly.path)?;
        if actual != assembly.sha256 {
            bail!("blob sha256 mismatch");
        }
        let assembly = self
            .blobs
            .remove(&blob_id)
            .ok_or_else(|| anyhow!("completed blob assembly disappeared"))?;
        let completed_path = self.root_dir.join(format!("{blob_id}.blob"));
        let _ = std::fs::remove_file(&completed_path);
        std::fs::rename(&assembly.path, &completed_path).with_context(|| {
            format!(
                "publishing blob {} to {}",
                assembly.blob_id,
                completed_path.display()
            )
        })?;
        self.completed.insert(
            assembly.blob_id.clone(),
            CompletedBlob {
                path: completed_path.clone(),
                mime: assembly.mime.clone(),
                purpose: assembly.purpose.clone(),
                total_size: assembly.total_size,
                sha256: assembly.sha256.clone(),
                created_at: now,
            },
        );
        Ok(json!({
            "blobId": assembly.blob_id,
            "path": completed_path.to_string_lossy(),
            "mime": assembly.mime,
            "purpose": assembly.purpose,
            "totalSize": assembly.total_size,
            "sha256": assembly.sha256
        }))
    }

    fn take_completed_json(&mut self, payload: &Value) -> Result<Value> {
        self.prune_expired(Instant::now());
        let blob_id = parse_blob_id(payload)?;
        let expected_size = payload
            .get("totalSize")
            .or_else(|| payload.get("total_size"))
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("response.blob requires totalSize"))?;
        let expected_sha = payload
            .get("sha256")
            .and_then(Value::as_str)
            .map(normalize_sha256_hex)
            .transpose()?
            .ok_or_else(|| anyhow!("response.blob requires sha256"))?;
        let completed = self
            .completed
            .remove(&blob_id)
            .ok_or_else(|| anyhow!("unknown completed blobId {blob_id}"))?;
        let result = (|| {
            if !completed
                .mime
                .as_deref()
                .is_none_or(|mime| mime.eq_ignore_ascii_case("application/json"))
            {
                bail!("response.blob mime must be application/json");
            }
            if !completed
                .purpose
                .as_deref()
                .is_none_or(|purpose| purpose == "response")
            {
                bail!("response.blob purpose must be response");
            }
            if completed.total_size != expected_size {
                bail!(
                    "response.blob totalSize changed from {} to {}",
                    completed.total_size,
                    expected_size
                );
            }
            if completed.sha256 != expected_sha {
                bail!("response.blob sha256 changed from blob.end");
            }
            let actual = sha256_file_hex(&completed.path)?;
            if actual != expected_sha {
                bail!("response.blob sha256 mismatch");
            }
            let bytes = std::fs::read(&completed.path)
                .with_context(|| format!("reading response blob {}", completed.path.display()))?;
            if bytes.len() as u64 != expected_size {
                bail!("response.blob file size mismatch");
            }
            serde_json::from_slice(&bytes).context("decoding response blob JSON")
        })();
        let _ = std::fs::remove_file(completed.path);
        result
    }

    fn take_completed_bytes(
        &mut self,
        payload: &Value,
        expected_purpose: &str,
        allowed_mimes: &[&str],
    ) -> Result<Vec<u8>> {
        self.prune_expired(Instant::now());
        let blob_id = parse_blob_id(payload)?;
        let expected_size = payload
            .get("totalSize")
            .or_else(|| payload.get("total_size"))
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("binary blob requires totalSize"))?;
        let expected_sha = payload
            .get("sha256")
            .and_then(Value::as_str)
            .map(normalize_sha256_hex)
            .transpose()?
            .ok_or_else(|| anyhow!("binary blob requires sha256"))?;
        let expected_mime = payload.get("mime").and_then(Value::as_str);
        if let Some(mime) = expected_mime {
            if !allowed_mimes
                .iter()
                .any(|allowed| mime.eq_ignore_ascii_case(allowed))
            {
                bail!("binary blob mime {mime} is not allowed for {expected_purpose}");
            }
        }
        if let Some(purpose) = payload.get("purpose").and_then(Value::as_str) {
            if purpose != expected_purpose {
                bail!("binary blob purpose {purpose} does not match expected {expected_purpose}");
            }
        }
        let completed = self
            .completed
            .remove(&blob_id)
            .ok_or_else(|| anyhow!("unknown completed blobId {blob_id}"))?;
        let result = (|| {
            if completed.purpose.as_deref() != Some(expected_purpose) {
                bail!(
                    "binary blob purpose {:?} does not match expected {}",
                    completed.purpose,
                    expected_purpose
                );
            }
            let Some(completed_mime) = completed.mime.as_deref() else {
                bail!("binary blob mime is missing");
            };
            if !allowed_mimes
                .iter()
                .any(|allowed| completed_mime.eq_ignore_ascii_case(allowed))
            {
                bail!(
                    "binary blob mime {} is not allowed for {}",
                    completed_mime,
                    expected_purpose
                );
            }
            if let Some(expected_mime) = expected_mime {
                if !completed_mime.eq_ignore_ascii_case(expected_mime) {
                    bail!("binary blob mime changed from descriptor");
                }
            }
            if completed.total_size != expected_size {
                bail!(
                    "binary blob totalSize changed from {} to {}",
                    completed.total_size,
                    expected_size
                );
            }
            if completed.sha256 != expected_sha {
                bail!("binary blob sha256 changed from blob.end");
            }
            let actual = sha256_file_hex(&completed.path)?;
            if actual != expected_sha {
                bail!("binary blob sha256 mismatch");
            }
            let bytes = std::fs::read(&completed.path)
                .with_context(|| format!("reading binary blob {}", completed.path.display()))?;
            if bytes.len() as u64 != expected_size {
                bail!("binary blob file size mismatch");
            }
            Ok(bytes)
        })();
        let _ = std::fs::remove_file(completed.path);
        result
    }

    fn prune_expired(&mut self, now: Instant) {
        let mut expired = Vec::new();
        self.blobs.retain(|blob_id, assembly| {
            let keep = now.duration_since(assembly.updated_at) <= BLOB_TTL
                && now.duration_since(assembly.created_at) <= BLOB_TTL;
            if !keep {
                expired.push((blob_id.clone(), assembly.path.clone()));
            }
            keep
        });
        for (_, path) in expired {
            let _ = std::fs::remove_file(path);
        }
        let mut expired_completed = Vec::new();
        self.completed.retain(|blob_id, completed| {
            let keep = now.duration_since(completed.created_at) <= BLOB_TTL;
            if !keep {
                expired_completed.push((blob_id.clone(), completed.path.clone()));
            }
            keep
        });
        for (_, path) in expired_completed {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn parse_blob_id(payload: &Value) -> Result<String> {
    let blob_id = payload
        .get("blobId")
        .or_else(|| payload.get("blob_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("blob message requires blobId"))?;
    validate_blob_id(blob_id)?;
    Ok(blob_id.to_string())
}

fn validate_blob_id(blob_id: &str) -> Result<()> {
    if blob_id.is_empty() || blob_id.len() > 128 {
        bail!("invalid blobId length");
    }
    if !blob_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        bail!("invalid blobId characters");
    }
    Ok(())
}

fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening blob for sha256 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn status_from_state(state: &BrokerState) -> BrokerStatus {
    BrokerStatus {
        running: state.running,
        endpoint: state.endpoint.clone(),
        extension_connected: state.extension_connected,
        extension_protocol_version: state.extension_protocol_version,
        extension_version: state.extension_version.clone(),
        active_connection_id: state.active_connection_id,
        connected_hosts: state.connected_hosts,
        last_error: state.last_error.clone(),
    }
}

fn extension_protocol_version(message: &Value) -> Option<u32> {
    message
        .get("protocolVersion")
        .or_else(|| message.get("protocol_version"))
        .or_else(|| {
            message
                .get("payload")
                .and_then(|p| p.get("protocolVersion"))
        })
        .or_else(|| message.get("params").and_then(|p| p.get("protocolVersion")))
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
}

fn extension_version(message: &Value) -> Option<String> {
    message
        .get("extensionVersion")
        .or_else(|| message.get("extension_version"))
        .or_else(|| {
            message
                .get("payload")
                .and_then(|p| p.get("extensionVersion"))
        })
        .or_else(|| {
            message
                .get("payload")
                .and_then(|p| p.get("extension_version"))
        })
        .or_else(|| {
            message
                .get("params")
                .and_then(|p| p.get("extensionVersion"))
        })
        .or_else(|| {
            message
                .get("params")
                .and_then(|p| p.get("extension_version"))
        })
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

async fn write_loop(mut writer: impl AsyncWrite + Unpin, mut rx: mpsc::UnboundedReceiver<Value>) {
    while let Some(message) = rx.recv().await {
        if let Err(e) = write_broker_message(&mut writer, &message).await {
            app_warn!(
                "browser",
                "extension_broker",
                "failed to write native host message: {}",
                e
            );
            break;
        }
    }
}

async fn read_broker_message(reader: &mut (impl AsyncRead + Unpin)) -> Result<Option<Value>> {
    let mut len_buf = [0u8; 4];
    let mut read = 0usize;
    while read < len_buf.len() {
        match reader.read(&mut len_buf[read..]).await {
            Ok(0) if read == 0 => return Ok(None),
            Ok(0) => bail!("truncated broker message length header"),
            Ok(n) => read += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e).context("reading broker message length"),
        }
    }

    let len = u32::from_le_bytes(len_buf);
    if len == 0 {
        bail!("broker message length must be greater than zero");
    }
    if len > MAX_BROKER_MESSAGE_LEN {
        bail!(
            "broker message length {} exceeds max {}",
            len,
            MAX_BROKER_MESSAGE_LEN
        );
    }

    let mut payload = vec![0u8; len as usize];
    reader
        .read_exact(&mut payload)
        .await
        .context("reading broker message payload")?;
    serde_json::from_slice(&payload)
        .map(Some)
        .context("decoding broker message JSON")
}

async fn write_broker_message(writer: &mut (impl AsyncWrite + Unpin), value: &Value) -> Result<()> {
    let payload = serde_json::to_vec(value).context("encoding broker message JSON")?;
    if payload.len() > MAX_BROKER_MESSAGE_LEN as usize {
        bail!(
            "broker message length {} exceeds max {}",
            payload.len(),
            MAX_BROKER_MESSAGE_LEN
        );
    }
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await
        .context("writing broker message length")?;
    writer
        .write_all(&payload)
        .await
        .context("writing broker message payload")?;
    writer.flush().await.context("flushing broker message")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn peer_uid_values_must_match() {
        assert!(validate_peer_uid_values(501, 501).is_ok());
        assert!(validate_peer_uid_values(501, 502).is_err());
    }

    #[test]
    fn extracts_extension_hello_version_metadata() {
        let message = json!({
            "method": "extension.hello",
            "protocolVersion": 7,
            "payload": {
                "extensionVersion": "0.1.2"
            }
        });
        assert_eq!(extension_protocol_version(&message), Some(7));
        assert_eq!(extension_version(&message).as_deref(), Some("0.1.2"));
    }

    #[test]
    fn broker_status_carries_extension_version_metadata() {
        let state = BrokerState {
            running: true,
            endpoint: Some("unix:/tmp/test.sock".to_string()),
            token: "token".to_string(),
            sender: None,
            active_connection_id: Some(9),
            connected_hosts: 1,
            extension_connected: true,
            extension_protocol_version: Some(EXPECTED_EXTENSION_PROTOCOL_VERSION),
            extension_version: Some("0.1.0".to_string()),
            last_error: None,
        };
        let status = status_from_state(&state);
        assert_eq!(
            status.extension_protocol_version,
            Some(EXPECTED_EXTENSION_PROTOCOL_VERSION)
        );
        assert_eq!(status.extension_version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn assembles_chunked_response_in_order() {
        let response = json!({
            "id": "core-1",
            "ok": true,
            "result": { "data": "hello world" }
        });
        let encoded = serde_json::to_string(&response).unwrap();
        let split = encoded.len() / 2;
        let mut chunks = HashMap::new();

        let first = assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-1",
                "type": "response.chunk",
                "index": 0,
                "total": 2,
                "data": &encoded[..split],
            }),
        )
        .unwrap();
        assert!(first.is_none());

        let complete = assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-1",
                "type": "response.chunk",
                "index": 1,
                "total": 2,
                "data": &encoded[split..],
            }),
        )
        .unwrap();
        assert_eq!(complete, Some(response));
        assert!(chunks.is_empty());
    }

    #[test]
    fn rejects_inconsistent_chunk_total() {
        let mut chunks = HashMap::new();
        assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-2",
                "type": "response.chunk",
                "index": 0,
                "total": 2,
                "data": "{\"id\"",
            }),
        )
        .unwrap();

        let err = assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-2",
                "type": "response.chunk",
                "index": 1,
                "total": 3,
                "data": ":1}",
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("total changed"));
    }

    #[test]
    fn rejects_chunked_response_sha256_mismatch() {
        let response = json!({
            "id": "core-3",
            "ok": true,
            "result": { "data": "hello world" }
        });
        let encoded = serde_json::to_string(&response).unwrap();
        let split = encoded.len() / 2;
        let wrong_sha = "0".repeat(64);
        let mut chunks = HashMap::new();

        assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-3",
                "type": "response.chunk",
                "index": 0,
                "total": 2,
                "sha256": wrong_sha,
                "data": &encoded[..split],
            }),
        )
        .unwrap();

        let err = assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-3",
                "type": "response.chunk",
                "index": 1,
                "total": 2,
                "sha256": wrong_sha,
                "data": &encoded[split..],
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("sha256 mismatch"));
        assert!(chunks.is_empty());
    }

    #[test]
    fn rejects_conflicting_duplicate_chunk() {
        let mut chunks = HashMap::new();
        assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-4",
                "type": "response.chunk",
                "index": 0,
                "total": 2,
                "data": "{\"id\"",
            }),
        )
        .unwrap();

        let err = assemble_response_chunk(
            &mut chunks,
            &json!({
                "id": "core-4",
                "type": "response.chunk",
                "index": 0,
                "total": 2,
                "data": "{\"other\"",
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("received twice"));
    }

    #[test]
    fn prunes_expired_chunk_assembly() {
        let response = json!({
            "id": "core-5",
            "ok": true,
            "result": { "data": "hello world" }
        });
        let encoded = serde_json::to_string(&response).unwrap();
        let split = encoded.len() / 2;
        let old = Instant::now();
        let later = old + CHUNKED_RESPONSE_TTL + Duration::from_secs(1);
        let mut chunks = HashMap::new();

        assemble_response_chunk_at(
            &mut chunks,
            &json!({
                "id": "core-5",
                "type": "response.chunk",
                "index": 0,
                "total": 2,
                "data": &encoded[..split],
            }),
            old,
        )
        .unwrap();
        assert!(chunks.contains_key("core-5"));

        let next = assemble_response_chunk_at(
            &mut chunks,
            &json!({
                "id": "core-5",
                "type": "response.chunk",
                "index": 1,
                "total": 2,
                "data": &encoded[split..],
            }),
            later,
        )
        .unwrap();
        assert!(next.is_none());
        assert_eq!(chunks.get("core-5").map(|a| a.received), Some(1));
    }

    #[test]
    fn moves_download_to_managed_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = tmp.path().join("chrome-downloads");
        let managed_dir = tmp.path().join("hope-downloads");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("report.txt");
        std::fs::write(&source, b"hello").unwrap();

        let dest = move_download_to_managed_dir(&source, &managed_dir).unwrap();
        assert_eq!(dest, managed_dir.join("report.txt"));
        assert!(!source.exists());
        assert_eq!(std::fs::read(dest).unwrap(), b"hello");
    }

    #[test]
    fn download_policy_uniquifies_existing_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let source_dir = tmp.path().join("chrome-downloads");
        let managed_dir = tmp.path().join("hope-downloads");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::create_dir_all(&managed_dir).unwrap();
        let source = source_dir.join("report.txt");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(managed_dir.join("report.txt"), b"old").unwrap();

        let dest = move_download_to_managed_dir(&source, &managed_dir).unwrap();
        assert_eq!(dest, managed_dir.join("report (1).txt"));
        assert_eq!(std::fs::read(dest).unwrap(), b"new");
        assert_eq!(
            std::fs::read(managed_dir.join("report.txt")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn blob_store_accepts_chunked_blob_with_sha256() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = BlobStore::new(tmp.path().join("blobs"));
        let data = b"hello world";
        let sha = sha256_hex(data);
        store
            .handle_message(
                "blob.begin",
                &json!({
                    "blobId": "blob-1",
                    "mime": "text/plain",
                    "purpose": "test",
                    "totalSize": data.len(),
                    "sha256": sha,
                }),
            )
            .unwrap();
        let first = &data[..5];
        let second = &data[5..];
        store
            .handle_message(
                "blob.chunk",
                &json!({
                    "blobId": "blob-1",
                    "index": 0,
                    "offset": 0,
                    "base64": base64::engine::general_purpose::STANDARD.encode(first),
                    "sha256": sha256_hex(first),
                }),
            )
            .unwrap();
        store
            .handle_message(
                "blob.chunk",
                &json!({
                    "blobId": "blob-1",
                    "index": 1,
                    "offset": first.len(),
                    "base64": base64::engine::general_purpose::STANDARD.encode(second),
                    "sha256": sha256_hex(second),
                }),
            )
            .unwrap();
        let result = store
            .handle_message(
                "blob.end",
                &json!({
                    "blobId": "blob-1",
                    "totalChunks": 2,
                    "sha256": sha,
                }),
            )
            .unwrap();
        let path = PathBuf::from(result.get("path").and_then(Value::as_str).unwrap());
        assert_eq!(std::fs::read(path).unwrap(), data);
    }

    #[test]
    fn blob_store_takes_completed_response_json() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = BlobStore::new(tmp.path().join("blobs"));
        let response = json!({
            "id": "core-9",
            "ok": true,
            "result": { "data": "large response" }
        });
        let data = serde_json::to_vec(&response).unwrap();
        let sha = sha256_hex(&data);
        store
            .handle_message(
                "blob.begin",
                &json!({
                    "blobId": "response-core-9",
                    "mime": "application/json",
                    "purpose": "response",
                    "totalSize": data.len(),
                    "sha256": sha,
                }),
            )
            .unwrap();
        store
            .handle_message(
                "blob.chunk",
                &json!({
                    "blobId": "response-core-9",
                    "index": 0,
                    "offset": 0,
                    "base64": base64::engine::general_purpose::STANDARD.encode(&data),
                    "sha256": sha256_hex(&data),
                }),
            )
            .unwrap();
        let result = store
            .handle_message(
                "blob.end",
                &json!({
                    "blobId": "response-core-9",
                    "totalChunks": 1,
                    "sha256": sha,
                }),
            )
            .unwrap();
        let path = PathBuf::from(result.get("path").and_then(Value::as_str).unwrap());
        assert!(path.exists());

        let hydrated = store
            .take_completed_json(&json!({
                "type": "response.blob",
                "blobId": "response-core-9",
                "totalSize": data.len(),
                "sha256": sha,
            }))
            .unwrap();
        assert_eq!(hydrated, response);
        assert!(!path.exists());
    }

    #[test]
    fn blob_store_takes_completed_binary_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = BlobStore::new(tmp.path().join("blobs"));
        let data = b"\x89PNG\r\nbinary screenshot";
        let sha = sha256_hex(data);
        store
            .handle_message(
                "blob.begin",
                &json!({
                    "blobId": "screenshot-1",
                    "mime": "image/png",
                    "purpose": "screenshot",
                    "totalSize": data.len(),
                    "sha256": sha,
                }),
            )
            .unwrap();
        store
            .handle_message(
                "blob.chunk",
                &json!({
                    "blobId": "screenshot-1",
                    "index": 0,
                    "offset": 0,
                    "base64": base64::engine::general_purpose::STANDARD.encode(data),
                    "sha256": sha256_hex(data),
                }),
            )
            .unwrap();
        let result = store
            .handle_message(
                "blob.end",
                &json!({
                    "blobId": "screenshot-1",
                    "totalChunks": 1,
                    "sha256": sha,
                }),
            )
            .unwrap();
        let path = PathBuf::from(result.get("path").and_then(Value::as_str).unwrap());
        assert!(path.exists());

        let bytes = store
            .take_completed_bytes(
                &json!({
                    "blobId": "screenshot-1",
                    "mime": "image/png",
                    "purpose": "screenshot",
                    "totalSize": data.len(),
                    "sha256": sha,
                }),
                "screenshot",
                &["image/png"],
            )
            .unwrap();
        assert_eq!(bytes, data);
        assert!(!path.exists());
    }

    #[test]
    fn blob_store_rejects_overlapping_chunk_ranges() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = BlobStore::new(tmp.path().join("blobs"));
        let data = b"abcdef";
        store
            .handle_message(
                "blob.begin",
                &json!({
                    "blobId": "blob-2",
                    "totalSize": data.len(),
                    "sha256": sha256_hex(data),
                }),
            )
            .unwrap();
        store
            .handle_message(
                "blob.chunk",
                &json!({
                    "blobId": "blob-2",
                    "index": 0,
                    "offset": 0,
                    "base64": base64::engine::general_purpose::STANDARD.encode(&data[..4]),
                }),
            )
            .unwrap();
        let err = store
            .handle_message(
                "blob.chunk",
                &json!({
                    "blobId": "blob-2",
                    "index": 1,
                    "offset": 3,
                    "base64": base64::engine::general_purpose::STANDARD.encode(&data[3..]),
                }),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("overlaps"));
    }

    #[test]
    fn blob_store_prunes_expired_partial_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = BlobStore::new(tmp.path().join("blobs"));
        let data = b"hello";
        store
            .handle_message(
                "blob.begin",
                &json!({
                    "blobId": "blob-3",
                    "totalSize": data.len(),
                    "sha256": sha256_hex(data),
                }),
            )
            .unwrap();
        let path = store.blobs.get("blob-3").unwrap().path.clone();
        assert!(path.exists());
        let later = Instant::now() + BLOB_TTL + Duration::from_secs(1);
        store.prune_expired(later);
        assert!(!store.blobs.contains_key("blob-3"));
        assert!(!path.exists());
    }
}
