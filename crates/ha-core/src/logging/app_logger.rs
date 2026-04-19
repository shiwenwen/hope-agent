use std::path::PathBuf;
use std::sync::Arc;

use super::db::{should_log, LogDB};
use super::file_writer::LogFileWriter;
use super::types::*;

// ── Async Logger (non-blocking) ──────────────────────────────────

#[derive(Clone)]
pub struct AppLogger {
    sender: tokio::sync::mpsc::Sender<PendingLog>,
    config: Arc<std::sync::RwLock<LogConfig>>,
}

impl AppLogger {
    pub fn new(db: Arc<LogDB>, logs_dir: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(10000);
        let config = Arc::new(std::sync::RwLock::new(LogConfig::default()));
        let config_clone = config.clone();

        // Spawn background writer task.
        // .manage() runs before the Tokio runtime is ready, so we always use a
        // dedicated thread with its own runtime to avoid the "no reactor" panic.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create logging runtime");
            rt.block_on(Self::writer_loop(rx, db, logs_dir, config_clone));
        });

        Self { sender: tx, config }
    }

    async fn writer_loop(
        mut rx: tokio::sync::mpsc::Receiver<PendingLog>,
        db: Arc<LogDB>,
        logs_dir: PathBuf,
        config: Arc<std::sync::RwLock<LogConfig>>,
    ) {
        let file_max_mb = config.read().map(|c| c.file_max_size_mb).unwrap_or(10);
        let mut file_writer = LogFileWriter::new(logs_dir, file_max_mb);
        let mut buffer: Vec<PendingLog> = Vec::new();
        let flush_interval = tokio::time::Duration::from_millis(200);

        loop {
            // Wait for first message or timeout
            let msg = tokio::time::timeout(flush_interval, rx.recv()).await;

            match msg {
                Ok(Some(entry)) => {
                    buffer.push(entry);
                    // Drain any additional immediately available messages
                    while buffer.len() < 100 {
                        match rx.try_recv() {
                            Ok(entry) => buffer.push(entry),
                            Err(_) => break,
                        }
                    }
                }
                Ok(None) => {
                    // Channel closed, flush remaining and exit
                    if !buffer.is_empty() {
                        let _ = db.batch_insert(&buffer);
                        let file_enabled = config.read().map(|c| c.file_enabled).unwrap_or(true);
                        if file_enabled {
                            for entry in &buffer {
                                file_writer.write_entry(entry);
                            }
                        }
                    }
                    break;
                }
                Err(_) => {
                    // Timeout — flush what we have
                }
            }

            if !buffer.is_empty() {
                let _ = db.batch_insert(&buffer);

                // Dual-write to log file
                let (file_enabled, file_max_mb) = config
                    .read()
                    .map(|c| (c.file_enabled, c.file_max_size_mb))
                    .unwrap_or((true, 10));
                if file_enabled {
                    file_writer.update_max_size(file_max_mb);
                    for entry in &buffer {
                        file_writer.write_entry(entry);
                    }
                }

                buffer.clear();
            }
        }
    }

    pub fn log(
        &self,
        level: &str,
        category: &str,
        source: &str,
        message: &str,
        details: Option<String>,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) {
        // Check if enabled and level passes
        if let Ok(config) = self.config.read() {
            if !config.enabled || !should_log(level, &config.level) {
                return;
            }
        }

        let timestamp = chrono::Utc::now().to_rfc3339();

        // Dev mode: also print to stderr for console visibility
        #[cfg(debug_assertions)]
        {
            let level_upper = level.to_uppercase();
            eprintln!(
                "[{}] {} [{}] {} — {}",
                &timestamp[11..19], // HH:MM:SS
                level_upper,
                category,
                source,
                message,
            );
        }

        let entry = PendingLog {
            timestamp,
            level: level.to_string(),
            category: category.to_string(),
            source: source.to_string(),
            message: message.to_string(),
            details,
            session_id,
            agent_id,
        };
        if self.sender.try_send(entry).is_err() {
            eprintln!("[LOG] Warning: log channel full, dropping entry");
        }
    }

    pub fn update_config(&self, config: LogConfig) {
        if let Ok(mut c) = self.config.write() {
            *c = config;
        }
    }

    pub fn get_config(&self) -> LogConfig {
        self.config.read().map(|c| c.clone()).unwrap_or_default()
    }
}
