//! Transport-agnostic interactive PTY sessions for the embedded terminal.

use crate::event_bus::{AppEvent, BroadcastEventBus, EventBus};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use uuid::Uuid;

const MAX_TERMINAL_SESSIONS: usize = 12;
const MAX_TERMINAL_INPUT_BYTES: usize = 64 * 1024;
const MAX_TERMINAL_SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;
const READ_CHUNK_BYTES: usize = 16 * 1024;
const TERMINAL_OUTPUT_EVENT_CAPACITY: usize = 256;

pub const REMOTE_TERMINAL_ACCESS_DISABLED: &str =
    "remote terminal access is disabled; enable filesystem.allowRemoteWrites to allow it";

pub fn is_terminal_event_name(name: &str) -> bool {
    name.starts_with("terminal:")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalOrigin {
    Desktop,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalStatus {
    Running,
    Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSummary {
    pub id: String,
    pub cwd: String,
    pub shell: String,
    pub title: String,
    pub created_at: u64,
    pub status: TerminalStatus,
    pub exit_code: Option<u32>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSnapshot {
    #[serde(flatten)]
    pub terminal: TerminalSummary,
    pub output_base64: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalRequest {
    pub cwd: Option<String>,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
}

fn default_cols() -> u16 {
    100
}

fn default_rows() -> u16 {
    28
}

struct TerminalSession {
    id: String,
    cwd: String,
    shell: String,
    title: String,
    created_at: u64,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    output: Mutex<VecDeque<u8>>,
    seq: AtomicU64,
    status: Mutex<TerminalStatus>,
    exit_code: Mutex<Option<u32>>,
    size: Mutex<PtySize>,
    event_bus: Arc<dyn EventBus>,
    output_event_bus: Arc<dyn EventBus>,
    origin: TerminalOrigin,
}

impl TerminalSession {
    fn summary(&self) -> TerminalSummary {
        let status = *self.status.lock().unwrap_or_else(|e| e.into_inner());
        let exit_code = *self.exit_code.lock().unwrap_or_else(|e| e.into_inner());
        let size = *self.size.lock().unwrap_or_else(|e| e.into_inner());
        TerminalSummary {
            id: self.id.clone(),
            cwd: self.cwd.clone(),
            shell: self.shell.clone(),
            title: self.title.clone(),
            created_at: self.created_at,
            status,
            exit_code,
            cols: size.cols,
            rows: size.rows,
        }
    }

    fn snapshot(&self) -> TerminalSnapshot {
        let (output, seq) = {
            let output = self.output.lock().unwrap_or_else(|e| e.into_inner());
            (
                output.iter().copied().collect::<Vec<_>>(),
                self.seq.load(Ordering::SeqCst),
            )
        };
        TerminalSnapshot {
            terminal: self.summary(),
            output_base64: base64::engine::general_purpose::STANDARD.encode(output),
            seq,
        }
    }

    fn push_output(&self, bytes: &[u8]) {
        let seq = {
            let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
            output.extend(bytes.iter().copied());
            if output.len() > MAX_TERMINAL_SCROLLBACK_BYTES {
                let overflow = output.len() - MAX_TERMINAL_SCROLLBACK_BYTES;
                output.drain(..overflow);
            }
            self.seq.fetch_add(1, Ordering::SeqCst) + 1
        };
        // Terminal output has its own broadcast channel. A noisy PTY must not
        // evict chat, approval, or session events from the process-wide bus.
        self.output_event_bus.emit(
            "terminal:output",
            json!({
                "terminalId": self.id,
                "seq": seq,
                "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
            }),
        );
    }

    fn mark_exited(&self, exit_code: Option<u32>, error: Option<String>) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = TerminalStatus::Exited;
        *self.exit_code.lock().unwrap_or_else(|e| e.into_inner()) = exit_code;
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.killer.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.event_bus.emit(
            "terminal:exit",
            json!({
                "terminalId": self.id,
                "exitCode": exit_code,
                "error": error,
            }),
        );
    }

    fn terminate(&self) {
        self.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(mut killer) = self.killer.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = killer.kill();
        }
    }
}

/// Process-scoped registry. Hiding the panel keeps shells alive; closing a tab
/// removes and terminates its shell.
pub struct TerminalManager {
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
    event_bus: Arc<dyn EventBus>,
    output_event_bus: Arc<dyn EventBus>,
    remote_access_allowed: AtomicBool,
}

impl TerminalManager {
    pub fn new(event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            event_bus,
            output_event_bus: Arc::new(BroadcastEventBus::new(TERMINAL_OUTPUT_EVENT_CAPACITY)),
            // Runtime initialization synchronizes this with the persisted
            // config before either transport starts accepting requests.
            remote_access_allowed: AtomicBool::new(false),
        }
    }

    pub fn create(self: &Arc<Self>, request: CreateTerminalRequest) -> Result<TerminalSnapshot> {
        self.create_with_origin(request, TerminalOrigin::Desktop)
    }

    pub fn create_remote(
        self: &Arc<Self>,
        request: CreateTerminalRequest,
    ) -> Result<TerminalSnapshot> {
        self.create_with_origin(request, TerminalOrigin::Remote)
    }

    fn create_with_origin(
        self: &Arc<Self>,
        request: CreateTerminalRequest,
        origin: TerminalOrigin,
    ) -> Result<TerminalSnapshot> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        // This check deliberately happens while holding the same lock used by
        // `set_remote_access_allowed(false)`: a revocation either wins before
        // creation or waits and removes the newly-created remote session.
        if origin == TerminalOrigin::Remote && !self.remote_access_allowed.load(Ordering::Acquire) {
            return Err(anyhow!(REMOTE_TERMINAL_ACCESS_DISABLED));
        }
        if sessions.len() >= MAX_TERMINAL_SESSIONS {
            return Err(anyhow!(
                "Too many terminal sessions (maximum {MAX_TERMINAL_SESSIONS})"
            ));
        }

        let cwd = resolve_cwd(request.cwd.as_deref())?;
        let shell = resolve_shell();
        let title = shell_title(&shell);
        let size = bounded_size(request.cols, request.rows);
        let pair = native_pty_system()
            .openpty(size)
            .context("Failed to create terminal PTY")?;

        let mut command = CommandBuilder::new(&shell);
        command.cwd(&cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "HopeAgent");

        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("Failed to start shell {shell}"))?;
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to open terminal output")?;
        let writer = pair
            .master
            .take_writer()
            .context("Failed to open terminal input")?;

        let id = Uuid::new_v4().to_string();
        let session = Arc::new(TerminalSession {
            id: id.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            shell,
            title,
            created_at: now_millis(),
            master: Mutex::new(pair.master),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(Some(killer)),
            output: Mutex::new(VecDeque::new()),
            seq: AtomicU64::new(0),
            status: Mutex::new(TerminalStatus::Running),
            exit_code: Mutex::new(None),
            size: Mutex::new(size),
            event_bus: Arc::clone(&self.event_bus),
            output_event_bus: Arc::clone(&self.output_event_bus),
            origin,
        });

        sessions.insert(id, Arc::clone(&session));
        drop(sessions);
        spawn_output_reader(Arc::clone(&session), reader);
        spawn_child_waiter(Arc::clone(&session), child);

        let snapshot = session.snapshot();
        self.event_bus
            .emit("terminal:created", json!({ "terminal": snapshot.clone() }));
        Ok(snapshot)
    }

    /// Synchronize the HTTP terminal capability with the persisted config.
    /// Revocation is active: every remote-created shell is killed and removed
    /// before this method returns. Desktop-created shells are unaffected.
    pub fn set_remote_access_allowed(&self, allowed: bool) {
        self.remote_access_allowed.store(allowed, Ordering::Release);
        if allowed {
            return;
        }

        let revoked = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let ids = sessions
                .iter()
                .filter_map(|(id, session)| {
                    (session.origin == TerminalOrigin::Remote).then_some(id.clone())
                })
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| sessions.remove(&id).map(|session| (id, session)))
                .collect::<Vec<_>>()
        };

        for (id, session) in revoked {
            session.terminate();
            self.event_bus.emit(
                "terminal:closed",
                json!({ "terminalId": id, "reason": "remoteAccessRevoked" }),
            );
        }
    }

    pub fn subscribe_output_events(&self) -> broadcast::Receiver<AppEvent> {
        self.output_event_bus.subscribe()
    }

    pub fn list(&self) -> Vec<TerminalSummary> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let mut summaries = sessions
            .values()
            .map(|session| session.summary())
            .collect::<Vec<_>>();
        summaries.sort_by_key(|summary| summary.created_at);
        summaries
    }

    pub fn snapshot(&self, terminal_id: &str) -> Result<TerminalSnapshot> {
        Ok(self.session(terminal_id)?.snapshot())
    }

    pub fn write_input(&self, terminal_id: &str, data: &str) -> Result<()> {
        if data.len() > MAX_TERMINAL_INPUT_BYTES {
            return Err(anyhow!(
                "Terminal input is too large (maximum {MAX_TERMINAL_INPUT_BYTES} bytes)"
            ));
        }
        let session = self.session(terminal_id)?;
        if *session.status.lock().unwrap_or_else(|e| e.into_inner()) != TerminalStatus::Running {
            return Err(anyhow!("Terminal session has exited"));
        }
        let mut writer = session.writer.lock().unwrap_or_else(|e| e.into_inner());
        let writer = writer
            .as_mut()
            .ok_or_else(|| anyhow!("Terminal input is closed"))?;
        writer
            .write_all(data.as_bytes())
            .context("Failed to write terminal input")?;
        writer.flush().context("Failed to flush terminal input")
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<()> {
        let session = self.session(terminal_id)?;
        let size = bounded_size(cols, rows);
        session
            .master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resize(size)
            .context("Failed to resize terminal")?;
        *session.size.lock().unwrap_or_else(|e| e.into_inner()) = size;
        Ok(())
    }

    pub fn close(&self, terminal_id: &str) -> Result<()> {
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(terminal_id)
            .ok_or_else(|| anyhow!("Terminal session not found"))?;
        session.terminate();
        self.event_bus
            .emit("terminal:closed", json!({ "terminalId": terminal_id }));
        Ok(())
    }

    fn session(&self, terminal_id: &str) -> Result<Arc<TerminalSession>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(terminal_id)
            .cloned()
            .ok_or_else(|| anyhow!("Terminal session not found"))
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        for session in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            session.terminate();
        }
    }
}

fn spawn_output_reader(session: Arc<TerminalSession>, mut reader: Box<dyn Read + Send>) {
    let _ = std::thread::Builder::new()
        .name(format!("terminal-output-{}", &session.id[..8]))
        .spawn(move || {
            let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => session.push_output(&buffer[..read]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    // PTY backends commonly return EIO when the slave exits.
                    Err(_) => break,
                }
            }
        });
}

fn spawn_child_waiter(
    session: Arc<TerminalSession>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
) {
    let _ = std::thread::Builder::new()
        .name(format!("terminal-wait-{}", &session.id[..8]))
        .spawn(move || match child.wait() {
            Ok(status) => session.mark_exited(Some(status.exit_code()), None),
            Err(error) => session.mark_exited(None, Some(error.to_string())),
        });
}

fn resolve_cwd(requested: Option<&str>) -> Result<PathBuf> {
    let candidate = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("Cannot resolve a terminal working directory"))?;
    let canonical = candidate.canonicalize().with_context(|| {
        format!(
            "Terminal working directory does not exist: {}",
            candidate.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(anyhow!(
            "Terminal working directory is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn resolve_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn shell_title(shell: &str) -> String {
    Path::new(shell)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(shell)
        .to_string()
}

fn bounded_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols: cols.clamp(2, 500),
        rows: rows.clamp(2, 500),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
