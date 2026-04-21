use std::path::PathBuf;
use std::sync::Arc;

use super::db::{should_log, LogDB};
use super::file_ops::cleanup_old_log_files;
use super::file_writer::LogFileWriter;
use super::types::*;

// ── Async Logger (non-blocking) ──────────────────────────────────

/// Interval for periodic retention enforcement (age + size + file cleanup).
/// Long-running daemons (`hope-agent server`) can run for weeks without a
/// restart, so relying solely on the startup cleanup in `app_init.rs` is
/// not enough — this keeps the configured `max_age_days` / `max_size_mb`
/// honoured while the process is alive.
const CLEANUP_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_secs(6 * 3600);

#[derive(Clone)]
pub struct AppLogger {
    sender: tokio::sync::mpsc::Sender<PendingLog>,
    config: Arc<std::sync::RwLock<LogConfig>>,
}

impl AppLogger {
    pub fn new(db: Arc<LogDB>, logs_dir: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(10000);
        let config = Arc::new(std::sync::RwLock::new(LogConfig::default()));
        let config_writer = config.clone();
        let config_cleanup = config.clone();
        let db_writer = db.clone();

        // Spawn background writer task.
        // .manage() runs before the Tokio runtime is ready, so we always use a
        // dedicated thread with its own runtime to avoid the "no reactor" panic.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create logging runtime");
            rt.block_on(async move {
                tokio::spawn(Self::cleanup_loop(db, config_cleanup));
                Self::writer_loop(rx, db_writer, logs_dir, config_writer).await;
            });
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

    async fn cleanup_loop(db: Arc<LogDB>, config: Arc<std::sync::RwLock<LogConfig>>) {
        // interval's first tick fires immediately, so cleanup runs right after
        // startup without blocking `init_app_state`, then settles into the 6h
        // cadence for the lifetime of the process.
        let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            ticker.tick().await;

            let (enabled, max_age_days, max_size_mb) = match config.read() {
                Ok(c) => (c.enabled, c.max_age_days, c.max_size_mb),
                Err(_) => continue,
            };
            if !enabled {
                continue;
            }

            let db = db.clone();
            // One spawn_blocking per tick: keeps the Mutex + VACUUM off the
            // async worker and lets all three steps share the same thread.
            let _ = tokio::task::spawn_blocking(move || {
                run_cleanup_once(&db, max_age_days, max_size_mb);
            })
            .await;
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

fn run_cleanup_once(db: &LogDB, max_age_days: u32, max_size_mb: u32) {
    log_cleanup("cleanup_old", db.cleanup_old(max_age_days), || {
        format!("older than {} days", max_age_days)
    });
    log_cleanup(
        "cleanup_old_log_files",
        cleanup_old_log_files(max_age_days),
        || format!("older than {} days", max_age_days),
    );
    log_cleanup("cleanup_by_size", db.cleanup_by_size(max_size_mb), || {
        format!("to stay under {}MB", max_size_mb)
    });
}

fn log_cleanup<F: FnOnce() -> String>(action: &str, result: anyhow::Result<u64>, describe: F) {
    match result {
        Ok(n) if n > 0 => {
            crate::app_info!(
                "logging",
                "cleanup",
                "{} removed {} ({})",
                action,
                n,
                describe()
            );
        }
        Err(e) => {
            crate::app_warn!("logging", "cleanup", "{} failed: {}", action, e);
        }
        _ => {}
    }
}
