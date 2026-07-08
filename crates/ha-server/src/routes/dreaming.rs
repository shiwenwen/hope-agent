//! Dreaming HTTP routes (Phase B3).
//!
//! Thin wrappers around `ha_core::memory::dreaming`. The heavy logic
//! lives in the core; these handlers only translate between JSON and
//! the internal types.

use axum::{
    extract::{Path, Query},
    Json,
};
use ha_core::memory::dreaming;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Default, Deserialize)]
pub struct ListDiariesQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListRunsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceQuoteQuery {
    pub session_id: String,
    pub message_id: Option<i64>,
}

use crate::error::AppError;
use ha_core::blocking::run_blocking;

/// `POST /api/dreaming/run` — kick off a cycle inline (trigger=manual).
pub async fn run_now() -> Result<Json<dreaming::DreamReport>, AppError> {
    Ok(Json(
        dreaming::manual_run(dreaming::DreamTrigger::Manual).await,
    ))
}

/// `POST /api/dreaming/resolver` — run one Deep resolver cycle (expire / merge
/// / conflict) over active claims (trigger=manual).
pub async fn run_resolver() -> Result<Json<dreaming::ResolverReport>, AppError> {
    Ok(Json(
        dreaming::run_resolver_cycle(dreaming::DreamTrigger::Manual).await,
    ))
}

/// `POST /api/dreaming/profile/run` — run one Memory Profile synthesis cycle
/// (manual = LLM rewrite) over active claims (trigger=manual).
pub async fn run_profile() -> Result<Json<dreaming::ProfileReport>, AppError> {
    Ok(Json(
        dreaming::run_profile_synthesis_cycle(dreaming::DreamTrigger::Manual).await,
    ))
}

/// `GET /api/dreaming/profile` — latest Memory Profile snapshot per scope
/// (read-only profile view). Owner-plane.
pub async fn list_profile_snapshots() -> Result<Json<Vec<dreaming::ProfileSnapshotRecord>>, AppError>
{
    Ok(Json(run_blocking(dreaming::list_profile_snapshots).await?))
}

/// `GET /api/dreaming/diaries?limit=N` — list available Dream Diary
/// files, newest first, optionally capped at `limit`.
pub async fn list_diaries(
    Query(q): Query<ListDiariesQuery>,
) -> Result<Json<Vec<dreaming::DiaryEntry>>, AppError> {
    Ok(Json(dreaming::list_diaries(q.limit)?))
}

/// `GET /api/dreaming/diaries/{filename}` — fetch the markdown of a
/// single diary file.
pub async fn read_diary(Path(filename): Path<String>) -> Result<Json<Value>, AppError> {
    let content = dreaming::read_diary(&filename)?;
    Ok(Json(json!({ "filename": filename, "content": content })))
}

/// `GET /api/dreaming/status` — report whether a cycle is currently in
/// progress (for the "Run now" button UI).
pub async fn status() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "running": dreaming::dreaming_running() })))
}

/// `GET /api/dreaming/last-report` — snapshot of the most recent
/// in-process cycle report (null before the first cycle).
pub async fn last_report() -> Result<Json<Option<dreaming::DreamReport>>, AppError> {
    Ok(Json(dreaming::last_report_snapshot()))
}

/// `GET /api/dreaming/idle-status` — last activity timestamp and the
/// configured idle threshold so the GUI can render a countdown.
pub async fn idle_status() -> Result<Json<Value>, AppError> {
    let cfg = ha_core::config::cached_config();
    Ok(Json(json!({
        "lastActivityEpochSecs": dreaming::last_activity_epoch_secs(),
        "idleMinutes": cfg.dreaming.idle_trigger.idle_minutes,
    })))
}

/// `GET /api/dreaming/runs?limit=N&offset=M` — durable run history, newest
/// first. Survives restart, unlike `/last-report`.
pub async fn list_runs(
    Query(q): Query<ListRunsQuery>,
) -> Result<Json<Vec<dreaming::DreamingRunRecord>>, AppError> {
    Ok(Json(
        run_blocking(move || dreaming::list_runs(q.limit, q.offset)).await?,
    ))
}

/// `GET /api/dreaming/runs/{id}` — a single run plus its decision log.
/// Returns `null` when the id is unknown (mirrors the Tauri command).
pub async fn get_run(
    Path(id): Path<String>,
) -> Result<Json<Option<dreaming::DreamingRunDetail>>, AppError> {
    Ok(Json(run_blocking(move || dreaming::get_run(&id)).await?))
}

/// `GET /api/dreaming/evidence/quote?sessionId=&messageId=` — resolve a
/// redacted, length-capped excerpt for an evidence ref (Evidence Layer).
///
/// Owner-plane (API-key trust, like `/api/sessions/{id}/messages`); it
/// surfaces a strict subset of that data. The incognito gate lives in the
/// core so expansion can't be unlocked by the frontend alone (design
/// §5.3 / §8.1) — incognito sources come back `available: false`.
pub async fn evidence_quote(
    Query(q): Query<EvidenceQuoteQuery>,
) -> Result<Json<dreaming::EvidenceQuote>, AppError> {
    Ok(Json(
        run_blocking(move || dreaming::evidence_quote(&q.session_id, q.message_id)).await,
    ))
}
