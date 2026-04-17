//! Dreaming — offline memory consolidation (Phase B3, Light phase).
//!
//! Periodically (when the app is idle, on a cron schedule, or on explicit
//! user request) the Dreaming pipeline scans recent memories, asks an LLM
//! to pick the entries worth promoting into pinned core memory, and writes
//! a human-readable "Dream Diary" markdown file.
//!
//! Design goals:
//! - **Offline**: never blocks chat; runs via `tokio::spawn_blocking` plus
//!   a dedicated async task. Serialised by a global `DREAMING_RUNNING`
//!   flag so overlapping triggers are safe.
//! - **Cheap**: complete cycle is one side_query call against the active
//!   agent's cached prompt prefix. No new embeddings, no new schema.
//! - **Observable**: every cycle writes a dated markdown file under
//!   `~/.opencomputer/memory/dreams/` and emits a `dreaming:cycle_complete`
//!   event on the EventBus for the Dashboard to refresh.
//! - **Opt-out friendly**: the global config switch defaults to enabled
//!   but individual triggers (idle / cron / manual) can be toggled
//!   independently and the feature as a whole can be disabled.
//!
//! Current scope is the **Light** phase only — conservative promotion
//! with relatively low thresholds and a short scan window. Deep and REM
//! phases (pattern recognition, long-window consolidation) are deferred.

mod config;
mod narrative;
mod pipeline;
mod promotion;
mod scanner;
mod scoring;
mod triggers;
mod types;

pub use config::{CronTriggerConfig, DreamingConfig, IdleTriggerConfig, PromotionThresholds};
pub use pipeline::run_cycle;
pub use triggers::{
    check_idle_trigger, dreaming_running, manual_run, touch_activity, DreamTrigger,
};
pub use types::{DreamReport, PromotionRecord};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Diary listing entry — returned by the Dashboard "Dream Diary" tab so
/// the user can pick a date without reading the files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryEntry {
    pub filename: String,
    pub modified: String,
    pub size_bytes: u64,
}

/// List Dream Diary markdown files in descending filename order (newest
/// first). When `limit` is provided, returns at most that many entries —
/// protects the Dashboard from blowing up after months of daily cycles.
///
/// Filenames are generated from local time (see `narrative::write_diary`),
/// so lexical sort == reverse chronological.
pub fn list_diaries(limit: Option<usize>) -> Result<Vec<DiaryEntry>> {
    let dir = crate::paths::dreams_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<DiaryEntry> = Vec::new();
    for entry in std::fs::read_dir(&dir).context("reading dreams_dir")? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let filename = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let modified = meta
            .modified()
            .ok()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_default();
        entries.push(DiaryEntry {
            filename,
            modified,
            size_bytes: meta.len(),
        });
    }
    entries.sort_by(|a, b| b.filename.cmp(&a.filename));
    if let Some(max) = limit {
        entries.truncate(max);
    }
    Ok(entries)
}

/// Read the markdown contents of a single diary file. Rejects any
/// filename that doesn't match `YYYY-MM-DD*.md` (or the simplified
/// lexical prefix) to prevent directory traversal.
pub fn read_diary(filename: &str) -> Result<String> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        anyhow::bail!("invalid diary filename");
    }
    if !filename.ends_with(".md") {
        anyhow::bail!("diary filename must end with .md");
    }
    let dir = crate::paths::dreams_dir()?;
    let path = dir.join(filename);
    std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
}
