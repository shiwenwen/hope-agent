use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 50;

// ── Data Structures ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashJournal {
    pub crashes: Vec<CrashEntry>,
    pub total_crashes: u64,
    #[serde(default)]
    pub last_backup: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashEntry {
    pub timestamp: String,
    pub exit_code: i32,
    #[serde(default)]
    pub signal: Option<String>,
    pub crash_count_session: u32,
    #[serde(default)]
    pub diagnosis_run: bool,
    #[serde(default)]
    pub diagnosis_result: Option<DiagnosisResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisResult {
    pub cause: String,
    pub severity: String,
    pub user_actionable: bool,
    pub recommendations: Vec<String>,
    #[serde(default)]
    pub auto_fix_applied: Vec<String>,
    #[serde(default)]
    pub provider_used: Option<String>,
}

// ── Signal Mapping (Unix) ──────────────────────────────────────────

/// Map exit code to signal name on Unix (exit code 128+N = signal N)
pub fn signal_name_from_exit_code(exit_code: i32) -> Option<String> {
    if exit_code > 128 {
        let sig = exit_code - 128;
        let name = match sig {
            1 => "SIGHUP",
            2 => "SIGINT",
            3 => "SIGQUIT",
            4 => "SIGILL",
            5 => "SIGTRAP",
            6 => "SIGABRT",
            7 => "SIGBUS",
            8 => "SIGFPE",
            9 => "SIGKILL",
            10 => "SIGUSR1",
            11 => "SIGSEGV",
            13 => "SIGPIPE",
            14 => "SIGALRM",
            15 => "SIGTERM",
            _ => return Some(format!("SIG{}", sig)),
        };
        Some(name.to_string())
    } else {
        None
    }
}

// ── Journal Operations ─────────────────────────────────────────────

impl CrashJournal {
    pub fn new() -> Self {
        Self {
            crashes: Vec::new(),
            total_crashes: 0,
            last_backup: None,
        }
    }

    /// Load journal from file, or create a new one if missing/corrupt
    pub fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| Self::new()),
            Err(_) => Self::new(),
        }
    }

    /// Save journal to file
    pub fn save(&self, path: &PathBuf) -> Result<(), String> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize crash journal: {}", e))?;
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write crash journal: {}", e))?;
        Ok(())
    }

    /// Add a new crash entry, trimming old entries if needed
    pub fn add_crash(&mut self, exit_code: i32, crash_count_session: u32) {
        let entry = CrashEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            exit_code,
            signal: signal_name_from_exit_code(exit_code),
            crash_count_session,
            diagnosis_run: false,
            diagnosis_result: None,
        };
        self.crashes.push(entry);
        self.total_crashes += 1;

        // Trim to MAX_ENTRIES
        if self.crashes.len() > MAX_ENTRIES {
            let excess = self.crashes.len() - MAX_ENTRIES;
            self.crashes.drain(0..excess);
        }
    }

    /// Update the last crash entry with diagnosis result
    pub fn set_last_diagnosis(&mut self, result: DiagnosisResult) {
        if let Some(last) = self.crashes.last_mut() {
            last.diagnosis_run = true;
            last.diagnosis_result = Some(result);
        }
    }

    /// Update the last backup timestamp
    pub fn set_last_backup(&mut self, timestamp: String) {
        self.last_backup = Some(timestamp);
    }

    /// Clear all crash entries
    pub fn clear(&mut self) {
        self.crashes.clear();
        self.total_crashes = 0;
    }
}
