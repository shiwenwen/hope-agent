use anyhow::Result;
use std::io::Write;
use std::path::PathBuf;

use super::types::PendingLog;

// ── Log File Writer ──────────────────────────────────────────────

/// Writes plain text log lines to date-based files under ~/.opencomputer/logs/
/// Format: `[TIMESTAMP] LEVEL [CATEGORY] SOURCE — MESSAGE`
/// Files named: `opencomputer-YYYY-MM-DD.log`, auto-rotate by date and size.
pub(super) struct LogFileWriter {
    logs_dir: PathBuf,
    current_file: Option<std::fs::File>,
    current_date: String,
    current_size: u64,
    max_size_bytes: u64,
}

impl LogFileWriter {
    pub(super) fn new(logs_dir: PathBuf, max_size_mb: u32) -> Self {
        Self {
            logs_dir,
            current_file: None,
            current_date: String::new(),
            current_size: 0,
            max_size_bytes: max_size_mb as u64 * 1024 * 1024,
        }
    }

    pub(super) fn update_max_size(&mut self, max_size_mb: u32) {
        self.max_size_bytes = max_size_mb as u64 * 1024 * 1024;
    }

    pub(super) fn write_entry(&mut self, entry: &PendingLog) {
        let date = &entry.timestamp[..10]; // "2026-03-21"

        // Rotate if date changed or file exceeded max size
        if date != self.current_date || self.current_size >= self.max_size_bytes {
            self.current_file = None;
        }

        if self.current_file.is_none() {
            if let Err(e) = self.open_file(date) {
                eprintln!("Failed to open log file: {}", e);
                return;
            }
        }

        if let Some(ref mut file) = self.current_file {
            // Format: [2026-03-21T10:30:00Z] INFO [agent] agent::run — Starting chat session
            let line = if let Some(ref details) = entry.details {
                format!(
                    "[{}] {} [{}] {} — {} | {}\n",
                    entry.timestamp,
                    entry.level.to_uppercase(),
                    entry.category,
                    entry.source,
                    entry.message,
                    details
                )
            } else {
                format!(
                    "[{}] {} [{}] {} — {}\n",
                    entry.timestamp,
                    entry.level.to_uppercase(),
                    entry.category,
                    entry.source,
                    entry.message
                )
            };
            let bytes = line.as_bytes();
            if file.write_all(bytes).is_ok() {
                self.current_size += bytes.len() as u64;
            }
        }
    }

    fn open_file(&mut self, date: &str) -> Result<()> {
        std::fs::create_dir_all(&self.logs_dir)?;
        self.current_date = date.to_string();

        // Find a non-full file for this date
        let base_name = format!("opencomputer-{}.log", date);
        let path = self.logs_dir.join(&base_name);

        // If base file exists and is over max size, use numbered suffix
        let (final_path, existing_size) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size >= self.max_size_bytes {
                // Find next available numbered file
                let mut n = 1u32;
                loop {
                    let numbered = self
                        .logs_dir
                        .join(format!("opencomputer-{}.{}.log", date, n));
                    if !numbered.exists() {
                        break (numbered, 0);
                    }
                    let s = std::fs::metadata(&numbered).map(|m| m.len()).unwrap_or(0);
                    if s < self.max_size_bytes {
                        break (numbered, s);
                    }
                    n += 1;
                }
            } else {
                (path, size)
            }
        } else {
            (path, 0)
        };

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&final_path)?;
        self.current_file = Some(file);
        self.current_size = existing_size;
        Ok(())
    }
}
