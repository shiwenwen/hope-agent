use axum::extract::{Path, Request};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use oc_core::paths;

use crate::error::AppError;
use crate::routes::file_serve::{
    apply_inline_media_headers, contained_canonical, resolve_mime_for_path,
    validate_safe_rest_path, HeaderOpts, MimeOpts,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotBody {
    pub data_url: Option<String>,
    pub error: Option<String>,
}

/// `POST /api/canvas/snapshot/{request_id}`
pub async fn canvas_submit_snapshot(
    Path(request_id): Path<String>,
    Json(body): Json<SnapshotBody>,
) -> Result<Json<Value>, AppError> {
    oc_core::tools::canvas::canvas_submit_snapshot(request_id, body.data_url, body.error)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct EvalBody {
    pub result: Option<String>,
    pub error: Option<String>,
}

/// `POST /api/canvas/eval/{request_id}`
pub async fn canvas_submit_eval_result(
    Path(request_id): Path<String>,
    Json(body): Json<EvalBody>,
) -> Result<Json<Value>, AppError> {
    oc_core::tools::canvas::canvas_submit_eval_result(request_id, body.result, body.error)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/canvas/show` — desktop-only: ask the shell to focus the canvas
/// panel for a given project. Server mode has no window to show, so this
/// just acknowledges the request.
pub async fn show_canvas_panel(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "ok": true, "note": "desktop-only" })))
}

/// `GET /api/canvas/by-session/{session_id}` — list canvas projects bound to a session.
pub async fn list_canvas_projects_by_session(
    Path(session_id): Path<String>,
) -> Result<Json<Vec<oc_core::tools::canvas::CanvasProjectView>>, AppError> {
    let projects = oc_core::tools::canvas::list_canvas_projects_by_session(session_id)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(projects))
}

// ── Canvas project CRUD (mirror of Tauri commands) ─────────────────

/// `GET /api/canvas/projects` — list all canvas projects.
pub async fn list_canvas_projects() -> Result<Json<Value>, AppError> {
    let raw = oc_core::tools::canvas::list_canvas_projects()
        .await
        .map_err(AppError::internal)?;
    let parsed: Value = serde_json::from_str(&raw).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(parsed))
}

/// `GET /api/canvas/projects/{project_id}` — fetch a single canvas project.
pub async fn get_canvas_project(
    Path(project_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    validate_canvas_project_id(&project_id)?;
    let raw = oc_core::tools::canvas::get_canvas_project(project_id)
        .await
        .map_err(AppError::internal)?;
    let parsed: Value = serde_json::from_str(&raw).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(parsed))
}

/// `DELETE /api/canvas/projects/{project_id}` — remove a canvas project
/// (DB row + on-disk working dir).
pub async fn delete_canvas_project(
    Path(project_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    validate_canvas_project_id(&project_id)?;
    oc_core::tools::canvas::delete_canvas_project(project_id)
        .await
        .map_err(AppError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/canvas/projects/{project_id}/{*rest}` — serve a file from a
/// canvas project working directory. The iframe in `CanvasPanel.tsx`
/// loads `…/index.html` through this route (plus sibling CSS/JS/images
/// via relative URLs).
pub async fn serve_canvas_project_file(
    Path((project_id, rest)): Path<(String, String)>,
    request: Request,
) -> Result<Response, AppError> {
    validate_canvas_project_id(&project_id)?;
    validate_safe_rest_path(&rest)?;

    let base_dir = paths::canvas_project_dir(&project_id)
        .map_err(|e| AppError::internal(e.to_string()))?;
    let candidate = base_dir.join(&rest);
    let file_canon = contained_canonical(&base_dir, &candidate).await?;

    let mime = resolve_mime_for_path(
        &file_canon,
        MimeOpts {
            html_charset: true,
            sniff_fallback: false,
        },
    )
    .await;

    let mut response = ServeFile::new(&file_canon)
        .oneshot(request)
        .await
        .map_err(|e| AppError::internal(format!("serve canvas file: {}", e)))?
        .into_response();

    apply_inline_media_headers(
        &mut response,
        HeaderOpts {
            mime: &mime,
            cache_secs: 60,
            disposition: "inline",
            no_referrer: true,
        },
    );

    Ok(response)
}

/// Canvas project IDs are UUID-ish generated IDs. Accept alphanumerics +
/// `-` / `_` only, bounded length — tight enough to rule out `..`, `/`,
/// and any shell metacharacters.
fn validate_canvas_project_id(id: &str) -> Result<(), AppError> {
    if id.is_empty() || id.len() > 128 {
        return Err(AppError::bad_request("invalid canvas project id"));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AppError::bad_request("invalid canvas project id"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_id_accepts_typical() {
        assert!(validate_canvas_project_id("abc-123").is_ok());
        assert!(validate_canvas_project_id("A_B_C").is_ok());
        assert!(validate_canvas_project_id("550e8400e29b41d4a716446655440000").is_ok());
    }

    #[test]
    fn canvas_id_rejects_bad() {
        assert!(validate_canvas_project_id("").is_err());
        assert!(validate_canvas_project_id("..").is_err());
        assert!(validate_canvas_project_id("a/b").is_err());
        assert!(validate_canvas_project_id("a\\b").is_err());
        assert!(validate_canvas_project_id("a;b").is_err());
    }
}
