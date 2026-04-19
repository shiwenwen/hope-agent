//! Dreaming HTTP routes (Phase B3).
//!
//! Thin wrappers around `oc_core::memory::dreaming`. The heavy logic
//! lives in the core; these handlers only translate between JSON and
//! the internal types.

use axum::{
    extract::{Path, Query},
    Json,
};
use oc_core::memory::dreaming;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Default, Deserialize)]
pub struct ListDiariesQuery {
    pub limit: Option<usize>,
}

use crate::error::AppError;

/// `POST /api/dreaming/run` — kick off a cycle inline (trigger=manual).
pub async fn run_now() -> Result<Json<dreaming::DreamReport>, AppError> {
    Ok(Json(
        dreaming::manual_run(dreaming::DreamTrigger::Manual).await,
    ))
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
