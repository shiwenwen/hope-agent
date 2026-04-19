//! Public command-style API for `/recap`, shared by the Tauri command layer
//! and the oc-server HTTP routes. Takes only simple (Serializable) inputs
//! and returns Serializable results.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

use crate::globals::{get_app_state, AppState};
use crate::recap::types::{GenerateMode, RecapProgress, RecapReport, RecapReportSummary};
use crate::recap::{generate_report, render_html, RecapContext, RecapDb};

fn get_state() -> Result<Arc<AppState>> {
    get_app_state()
        .cloned()
        .ok_or_else(|| anyhow!("AppState not initialized"))
}

/// Process-wide RecapDb handle. Opens once on first access and runs
/// `CREATE TABLE IF NOT EXISTS` only that first time; subsequent calls reuse
/// the same connection.
static RECAP_DB: OnceLock<Arc<RecapDb>> = OnceLock::new();

pub fn recap_db() -> Result<Arc<RecapDb>> {
    if let Some(db) = RECAP_DB.get() {
        return Ok(db.clone());
    }
    let db = Arc::new(RecapDb::open_default()?);
    let _ = RECAP_DB.set(db.clone());
    Ok(RECAP_DB.get().cloned().unwrap_or(db))
}

/// Generate a new report synchronously. Emits `recap_progress` events via the
/// global EventBus (if initialised) so clients can stream progress.
pub async fn generate(mode: GenerateMode) -> Result<RecapReport> {
    let state = get_state()?;
    let cancel = CancellationToken::new();
    let ctx = RecapContext::from_app_state(&state, cancel).await?;
    let event_bus = crate::get_event_bus().cloned();
    let report_id = uuid::Uuid::new_v4().to_string();
    let report_id_for_events = report_id.clone();

    let emit = move |p: RecapProgress| {
        if let Some(bus) = event_bus.as_ref() {
            bus.emit(
                "recap_progress",
                serde_json::json!({
                    "reportId": report_id_for_events,
                    "progress": p,
                }),
            );
        }
    };

    generate_report(&ctx, mode, report_id, emit).await
}

/// List most recent reports.
pub fn list_reports(limit: u32) -> Result<Vec<RecapReportSummary>> {
    recap_db()?.list_reports(limit)
}

/// Load a single report by ID.
pub fn get_report(id: &str) -> Result<Option<RecapReport>> {
    recap_db()?.get_report(id)
}

/// Delete a report.
pub fn delete_report(id: &str) -> Result<()> {
    recap_db()?.delete_report(id)
}

/// Render a saved report to HTML and write it to `output_path`.
///
/// If `output_path` is empty, a path under `~/.opencomputer/reports/` is
/// used. Returns the final file path on success.
pub fn export_html(id: &str, output_path: Option<String>) -> Result<String> {
    let db = recap_db()?;
    let report = db
        .get_report(id)?
        .ok_or_else(|| anyhow!("report not found: {}", id))?;
    let html = render_html(&report);

    let path = match output_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            let dir = crate::paths::reports_dir()?;
            std::fs::create_dir_all(&dir)?;
            dir.join(format!(
                "recap-{}.html",
                report
                    .meta
                    .generated_at
                    .replace([':', '.'], "-")
                    .chars()
                    .take(20)
                    .collect::<String>()
            ))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Err(e) = std::fs::write(&path, html.as_bytes()) {
        app_warn!("recap", "api", "failed to write report {}: {}", id, e);
        return Err(e.into());
    }
    let path_str = path.to_string_lossy().to_string();
    db.set_html_path(id, &path_str)?;
    Ok(path_str)
}
