use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use uuid::Uuid;

// ── Process Session ───────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProcessSession {
    pub id: String,
    pub command: String,
    pub pid: Option<u32>,
    pub cwd: String,
    pub started_at: u64,
    pub exited: bool,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<String>,
    pub status: ProcessStatus,
    pub backgrounded: bool,
    pub aggregated_output: String,
    pub tail: String,
    pub truncated: bool,
    pub max_output_chars: usize,
    /// Pending stdout since last drain
    pub pending_stdout: String,
    /// Pending stderr since last drain
    pub pending_stderr: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessStatus::Running => write!(f, "running"),
            ProcessStatus::Completed => write!(f, "completed"),
            ProcessStatus::Failed => write!(f, "failed"),
        }
    }
}

// ── Process Registry (global singleton) ───────────────────────────

pub struct ProcessRegistry {
    sessions: HashMap<String, ProcessSession>,
}

impl ProcessRegistry {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn add_session(&mut self, session: ProcessSession) {
        self.sessions.insert(session.id.clone(), session);
    }

    pub fn get_session(&self, id: &str) -> Option<&ProcessSession> {
        self.sessions.get(id)
    }

    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut ProcessSession> {
        self.sessions.get_mut(id)
    }

    #[allow(dead_code)]
    pub fn list_running(&self) -> Vec<&ProcessSession> {
        self.sessions
            .values()
            .filter(|s| !s.exited)
            .collect()
    }

    #[allow(dead_code)]
    pub fn list_finished(&self) -> Vec<&ProcessSession> {
        self.sessions
            .values()
            .filter(|s| s.exited)
            .collect()
    }

    pub fn list_all(&self) -> Vec<&ProcessSession> {
        self.sessions.values().collect()
    }

    pub fn mark_exited(&mut self, id: &str, exit_code: Option<i32>, exit_signal: Option<String>, status: ProcessStatus) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.exited = true;
            session.exit_code = exit_code;
            session.exit_signal = exit_signal;
            session.status = status;
        }
    }

    pub fn append_output(&mut self, id: &str, stream: &str, data: &str) {
        if let Some(session) = self.sessions.get_mut(id) {
            // Accumulate to aggregated output
            if session.aggregated_output.len() < session.max_output_chars {
                let remaining = session.max_output_chars - session.aggregated_output.len();
                if data.len() <= remaining {
                    session.aggregated_output.push_str(data);
                } else {
                    session.aggregated_output.push_str(&data[..remaining]);
                    session.truncated = true;
                }
            }

            // Update tail (keep last 2000 chars)
            session.tail.push_str(data);
            const MAX_TAIL: usize = 2000;
            if session.tail.len() > MAX_TAIL {
                let start = session.tail.len() - MAX_TAIL;
                session.tail = session.tail[start..].to_string();
            }

            // Accumulate to pending for drain
            match stream {
                "stdout" => session.pending_stdout.push_str(data),
                "stderr" => session.pending_stderr.push_str(data),
                _ => {}
            }
        }
    }

    /// Drain pending stdout/stderr (returns and clears pending buffers)
    pub fn drain_output(&mut self, id: &str) -> (String, String) {
        if let Some(session) = self.sessions.get_mut(id) {
            let stdout = std::mem::take(&mut session.pending_stdout);
            let stderr = std::mem::take(&mut session.pending_stderr);
            (stdout, stderr)
        } else {
            (String::new(), String::new())
        }
    }

    pub fn remove_session(&mut self, id: &str) -> Option<ProcessSession> {
        self.sessions.remove(id)
    }

    /// Cleanup finished sessions older than ttl_ms
    #[allow(dead_code)]
    pub fn cleanup_old_sessions(&mut self, ttl_ms: u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let to_remove: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.exited && (now - s.started_at) > ttl_ms)
            .map(|(id, _)| id.clone())
            .collect();

        for id in to_remove {
            self.sessions.remove(&id);
        }
    }
}

// Global registry
static REGISTRY: OnceLock<Mutex<ProcessRegistry>> = OnceLock::new();

pub fn get_registry() -> &'static Mutex<ProcessRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(ProcessRegistry::new()))
}

// ── Helper Functions ──────────────────────────────────────────────

/// Generate a short session ID (8 hex chars)
pub fn create_session_id() -> String {
    Uuid::new_v4().to_string()[..8].to_string()
}

/// Get current timestamp in milliseconds
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Derive a short name from a command for display
pub fn derive_session_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() <= 60 {
        trimmed.to_string()
    } else {
        format!("{}…", &trimmed[..57])
    }
}

/// Format duration in compact form
pub fn format_duration_compact(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else {
        format!("{:.1}h", ms as f64 / 3_600_000.0)
    }
}
