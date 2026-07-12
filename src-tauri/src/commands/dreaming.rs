//! Tauri commands wiring the Dreaming pipeline (Phase B3) to the
//! frontend. All heavy work happens inside ha-core; these commands are
//! thin error-translating shells.

use crate::commands::CmdError;
use ha_core::memory::dreaming;

/// Run an offline consolidation cycle synchronously and return the report.
/// Maps to `POST /api/dreaming/run` on the HTTP side.
#[tauri::command]
pub async fn dreaming_run_now() -> Result<dreaming::DreamReport, CmdError> {
    Ok(dreaming::manual_run(dreaming::DreamTrigger::Manual).await)
}

/// Run one Deep resolver cycle (expire / merge / conflict) over active claims
/// and return its summary. Maps to `POST /api/dreaming/resolver`.
#[tauri::command]
pub async fn dreaming_run_resolver() -> Result<dreaming::ResolverReport, CmdError> {
    Ok(dreaming::run_resolver_cycle(dreaming::DreamTrigger::Manual).await)
}

/// Read-only Deep resolver preflight for the Dashboard. Maps to
/// `GET /api/dreaming/resolver/preflight` on the HTTP side.
#[tauri::command]
pub async fn dreaming_resolver_preflight() -> Result<dreaming::ResolverPreflightReport, CmdError> {
    Ok(ha_core::blocking::run_blocking(dreaming::resolver_preflight).await)
}

/// Run one Memory Profile synthesis cycle (manual = LLM rewrite) and return
/// its summary. Maps to `POST /api/dreaming/profile/run`.
#[tauri::command]
pub async fn dreaming_run_profile() -> Result<dreaming::ProfileReport, CmdError> {
    Ok(dreaming::run_profile_synthesis_cycle(dreaming::DreamTrigger::Manual).await)
}

/// Latest Memory Profile snapshot per scope (read-only profile view). Maps to
/// `GET /api/dreaming/profile`.
#[tauri::command]
pub async fn dreaming_list_profile_snapshots(
) -> Result<Vec<dreaming::ProfileSnapshotRecord>, CmdError> {
    ha_core::blocking::run_blocking(dreaming::list_profile_snapshots)
        .await
        .map_err(Into::into)
}

/// List Dream Diary markdown files (newest first). `limit` caps the
/// returned set so the Dashboard stays responsive after months of daily
/// cycles; omitting it returns the full set.
#[tauri::command]
pub async fn dreaming_list_diaries(
    limit: Option<usize>,
) -> Result<Vec<dreaming::DiaryEntry>, CmdError> {
    ha_core::blocking::run_blocking(move || dreaming::list_diaries(limit))
        .await
        .map_err(Into::into)
}

/// Read the markdown for a single diary file.
#[tauri::command]
pub async fn dreaming_read_diary(filename: String) -> Result<String, CmdError> {
    ha_core::blocking::run_blocking(move || dreaming::read_diary(&filename))
        .await
        .map_err(Into::into)
}

/// Lightweight status probe so the Dashboard can grey out the "Run now"
/// button while a cycle is already in progress.
#[tauri::command]
pub async fn dreaming_is_running() -> Result<bool, CmdError> {
    Ok(dreaming::dreaming_running())
}

/// Snapshot of the most recent in-process `DreamReport`. Returns `null`
/// before the first cycle of this process. Used by the Settings panel
/// status row.
#[tauri::command]
pub async fn dreaming_last_report() -> Result<Option<dreaming::DreamReport>, CmdError> {
    Ok(dreaming::last_report_snapshot())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamingIdleStatus {
    pub last_activity_epoch_secs: i64,
    pub idle_minutes: u32,
}

/// Returns the wall-clock timestamp of the last user-facing activity and
/// the configured idle threshold so the GUI can render a "fires in N
/// minutes" countdown without polling backend state.
#[tauri::command]
pub async fn dreaming_idle_status() -> Result<DreamingIdleStatus, CmdError> {
    let cfg = ha_core::config::cached_config();
    Ok(DreamingIdleStatus {
        last_activity_epoch_secs: dreaming::last_activity_epoch_secs(),
        idle_minutes: cfg.dreaming.idle_trigger.idle_minutes,
    })
}

/// List durable run records, newest first. Survives restart (unlike
/// `dreaming_last_report`, which is process-local). Maps to
/// `GET /api/dreaming/runs`.
#[tauri::command]
pub async fn dreaming_list_runs(
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<dreaming::DreamingRunRecord>, CmdError> {
    ha_core::blocking::run_blocking(move || dreaming::list_runs(limit, offset))
        .await
        .map_err(Into::into)
}

/// Fetch a single run plus its decision log. Returns `null` if the id is
/// unknown. Maps to `GET /api/dreaming/runs/{id}`.
#[tauri::command]
pub async fn dreaming_get_run(id: String) -> Result<Option<dreaming::DreamingRunDetail>, CmdError> {
    ha_core::blocking::run_blocking(move || dreaming::get_run(&id))
        .await
        .map_err(Into::into)
}

/// Query durable decision rows directly. Owner-plane Review History helper;
/// maps to `GET /api/dreaming/decisions`.
#[tauri::command]
pub async fn dreaming_list_decisions(
    limit: Option<usize>,
    offset: Option<usize>,
    query: Option<String>,
    decision_type: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    since: Option<String>,
    target_type: Option<String>,
) -> Result<Vec<dreaming::DreamingDecisionListItem>, CmdError> {
    let filter = dreaming::DreamingDecisionListFilter {
        limit,
        offset,
        query,
        decision_type,
        scope_type,
        scope_id,
        since,
        target_type,
    };
    ha_core::blocking::run_blocking(move || dreaming::list_decisions(filter))
        .await
        .map_err(Into::into)
}

/// Query durable decision rows with total-match metadata. Owner-plane Review
/// History helper; maps to `GET /api/dreaming/decisions/page`.
#[tauri::command]
pub async fn dreaming_list_decisions_page(
    limit: Option<usize>,
    offset: Option<usize>,
    query: Option<String>,
    decision_type: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    since: Option<String>,
    target_type: Option<String>,
) -> Result<dreaming::DreamingDecisionListResponse, CmdError> {
    let filter = dreaming::DreamingDecisionListFilter {
        limit,
        offset,
        query,
        decision_type,
        scope_type,
        scope_id,
        since,
        target_type,
    };
    ha_core::blocking::run_blocking(move || dreaming::list_decisions_page(filter))
        .await
        .map_err(Into::into)
}

/// Resolve a redacted, length-capped excerpt for an evidence ref (Evidence
/// Layer). Incognito sources are never expandable; the backend gate can't
/// be bypassed from the frontend. Maps to
/// `GET /api/dreaming/evidence/quote`.
#[tauri::command]
pub async fn dreaming_evidence_quote(
    session_id: String,
    message_id: Option<i64>,
) -> Result<dreaming::EvidenceQuote, CmdError> {
    Ok(
        ha_core::blocking::run_blocking(move || dreaming::evidence_quote(&session_id, message_id))
            .await,
    )
}
