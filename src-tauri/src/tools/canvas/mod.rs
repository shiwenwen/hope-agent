use anyhow::Result;
use serde_json::Value;

use crate::canvas_db::CanvasDB;
use crate::paths;
use crate::tools::browser::IMAGE_BASE64_PREFIX;

pub(crate) mod project;
pub(crate) mod renderer;

// ── Canvas Config ──────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_auto_show")]
    pub auto_show: bool,
    #[serde(default = "default_content_type")]
    pub default_content_type: String,
    #[serde(default = "default_max_projects")]
    pub max_projects: u32,
    #[serde(default = "default_max_versions")]
    pub max_versions_per_project: i64,
    #[serde(default = "default_panel_width")]
    pub panel_width: u32,
}

fn default_enabled() -> bool { true }
fn default_auto_show() -> bool { true }
fn default_content_type() -> String { "html".to_string() }
fn default_max_projects() -> u32 { 100 }
fn default_max_versions() -> i64 { 50 }
fn default_panel_width() -> u32 { 480 }

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            auto_show: default_auto_show(),
            default_content_type: default_content_type(),
            max_projects: default_max_projects(),
            max_versions_per_project: default_max_versions(),
            panel_width: default_panel_width(),
        }
    }
}

/// Check if canvas is enabled in config.
pub fn is_canvas_enabled() -> bool {
    crate::provider::load_store()
        .map(|s| s.canvas.enabled)
        .unwrap_or(true)
}

// ── Helper: get or init canvas DB ──────────────────────────────────

fn get_canvas_db() -> Result<CanvasDB> {
    let db_path = paths::canvas_db_path()?;
    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    CanvasDB::open(&db_path)
}

// ── Helper: emit canvas events ─────────────────────────────────────

fn emit_canvas_event(event_name: &str, payload: &Value) {
    if let Some(app) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app.emit(event_name, payload.to_string());
    }
}

/// Build a canvas_show event payload with the project path resolved.
fn build_show_payload(project_id: &str, title: &str, content_type: &str) -> Value {
    let project_path = paths::canvas_project_dir(project_id)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    serde_json::json!({
        "projectId": project_id,
        "title": title,
        "contentType": content_type,
        "projectPath": project_path,
    })
}

// ── Tool Entry Point ───────────────────────────────────────────────

pub(crate) async fn tool_canvas(args: &Value, ctx: &super::execution::ToolExecContext) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    match action {
        "create" => action_create(args, ctx).await,
        "update" => action_update(args).await,
        "show" => action_show(args).await,
        "hide" => action_hide().await,
        "list" => action_list().await,
        "delete" => action_delete(args).await,
        "versions" => action_versions(args).await,
        "restore" => action_restore(args).await,
        "snapshot" => action_snapshot(args).await,
        "eval_js" => action_eval_js(args).await,
        "export" => action_export(args).await,
        _ => Err(anyhow::anyhow!("Unknown canvas action: '{}'", action)),
    }
}

// ── Actions ────────────────────────────────────────────────────────

async fn action_create(args: &Value, ctx: &super::execution::ToolExecContext) -> Result<String> {
    let db = get_canvas_db()?;

    let title = args.get("title").and_then(|v| v.as_str());
    let content_type = args
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("html");
    let html = args.get("html").and_then(|v| v.as_str());
    let css = args.get("css").and_then(|v| v.as_str());
    let js = args.get("js").and_then(|v| v.as_str());
    let content = args.get("content").and_then(|v| v.as_str());
    let language = args.get("language").and_then(|v| v.as_str());

    let project = project::create_project(
        &db,
        title,
        content_type,
        html,
        css,
        js,
        content,
        language,
        ctx.session_id.as_deref(),
        ctx.agent_id.as_deref(),
    )?;

    app_info!(
        "tool",
        "canvas",
        "Canvas project created: {} ({}), type={}",
        project.id,
        project.title,
        project.content_type
    );

    // Emit show event so frontend opens the panel
    let config = crate::provider::load_store()
        .map(|s| s.canvas)
        .unwrap_or_default();
    if config.auto_show {
        emit_canvas_event(
            "canvas_show",
            &build_show_payload(&project.id, &project.title, &project.content_type),
        );
    }

    Ok(serde_json::json!({
        "status": "created",
        "project_id": project.id,
        "title": project.title,
        "content_type": project.content_type,
        "version": 1,
        "message": format!("Canvas project '{}' created successfully. The preview panel is now visible to the user.", project.title)
    })
    .to_string())
}

async fn action_update(args: &Value) -> Result<String> {
    let db = get_canvas_db()?;

    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for update"))?;

    let title = args.get("title").and_then(|v| v.as_str());
    let html = args.get("html").and_then(|v| v.as_str());
    let css = args.get("css").and_then(|v| v.as_str());
    let js = args.get("js").and_then(|v| v.as_str());
    let content = args.get("content").and_then(|v| v.as_str());
    let language = args.get("language").and_then(|v| v.as_str());
    let version_message = args.get("version_message").and_then(|v| v.as_str());

    let config = crate::provider::load_store()
        .map(|s| s.canvas)
        .unwrap_or_default();

    let project = project::update_project(
        &db,
        project_id,
        title,
        html,
        css,
        js,
        content,
        language,
        version_message,
        config.max_versions_per_project,
    )?;

    app_info!(
        "tool",
        "canvas",
        "Canvas project updated: {} (v{})",
        project.id,
        project.version_count
    );

    // Emit reload event so frontend refreshes the iframe
    emit_canvas_event(
        "canvas_reload",
        &serde_json::json!({ "projectId": project.id }),
    );

    Ok(serde_json::json!({
        "status": "updated",
        "project_id": project.id,
        "title": project.title,
        "version": project.version_count,
        "message": format!("Canvas project '{}' updated to version {}. The preview has been refreshed.", project.title, project.version_count)
    })
    .to_string())
}

async fn action_show(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for show"))?;

    let db = get_canvas_db()?;
    let project = db
        .get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Canvas project '{}' not found", project_id))?;

    emit_canvas_event(
        "canvas_show",
        &build_show_payload(&project.id, &project.title, &project.content_type),
    );

    Ok(serde_json::json!({
        "status": "shown",
        "project_id": project.id,
        "message": format!("Canvas '{}' is now visible.", project.title)
    })
    .to_string())
}

async fn action_hide() -> Result<String> {
    emit_canvas_event("canvas_hide", &serde_json::json!({}));
    Ok(r#"{"status":"hidden","message":"Canvas panel hidden."}"#.to_string())
}

async fn action_list() -> Result<String> {
    let db = get_canvas_db()?;
    let projects = db.list_projects()?;
    Ok(serde_json::to_string(&serde_json::json!({
        "status": "ok",
        "count": projects.len(),
        "projects": projects,
    }))?)
}

async fn action_delete(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for delete"))?;

    let db = get_canvas_db()?;
    project::delete_project(&db, project_id)?;

    emit_canvas_event(
        "canvas_deleted",
        &serde_json::json!({ "projectId": project_id }),
    );

    app_info!("tool", "canvas", "Canvas project deleted: {}", project_id);

    Ok(serde_json::json!({
        "status": "deleted",
        "project_id": project_id,
        "message": "Canvas project deleted."
    })
    .to_string())
}

async fn action_versions(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for versions"))?;

    let db = get_canvas_db()?;
    let versions = db.list_versions(project_id)?;

    // Return metadata only, not full content
    let summaries: Vec<serde_json::Value> = versions
        .iter()
        .map(|v| {
            serde_json::json!({
                "version_number": v.version_number,
                "message": v.message,
                "created_at": v.created_at,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "status": "ok",
        "project_id": project_id,
        "count": summaries.len(),
        "versions": summaries,
    })
    .to_string())
}

async fn action_restore(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for restore"))?;
    let version_id = args
        .get("version_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Missing 'version_id' parameter for restore"))?;

    let db = get_canvas_db()?;
    let project = project::restore_version(&db, project_id, version_id)?;

    // Reload the preview
    emit_canvas_event(
        "canvas_reload",
        &serde_json::json!({ "projectId": project.id }),
    );

    Ok(serde_json::json!({
        "status": "restored",
        "project_id": project.id,
        "restored_from_version": version_id,
        "current_version": project.version_count,
        "message": format!("Restored to version {} and created new version {}.", version_id, project.version_count)
    })
    .to_string())
}

async fn action_snapshot(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for snapshot"))?;

    // Verify project exists
    let db = get_canvas_db()?;
    let _project = db
        .get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Canvas project '{}' not found", project_id))?;

    // Request snapshot from frontend via event + oneshot channel
    let request_id = uuid::Uuid::new_v4().to_string();

    // Register a pending snapshot request
    let rx = {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut pending = PENDING_SNAPSHOTS.lock().unwrap();
        pending.insert(request_id.clone(), tx);
        rx
    }; // MutexGuard dropped here before any .await

    // Emit snapshot request event to frontend
    emit_canvas_event(
        "canvas_snapshot_request",
        &serde_json::json!({
            "projectId": project_id,
            "requestId": request_id,
        }),
    );

    // Wait for frontend response (timeout 15s)
    match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
        Ok(Ok(snapshot_data)) => {
            if let Some(error) = snapshot_data.error {
                return Ok(serde_json::json!({
                    "status": "error",
                    "message": format!("Snapshot failed: {}", error)
                }).to_string());
            }
            if let Some(data_url) = snapshot_data.data_url {
                // Parse data URL: data:image/png;base64,xxxxx
                if let Some(comma_pos) = data_url.find(',') {
                    let mime_part = &data_url[5..comma_pos]; // skip "data:"
                    let mime = mime_part.split(';').next().unwrap_or("image/png");
                    let b64 = &data_url[comma_pos + 1..];

                    // Save to file
                    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                    let save_dir = paths::canvas_project_dir(project_id)?;
                    let snapshot_path = save_dir.join(format!("snapshot_{}.png", timestamp));
                    if let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                        let _ = std::fs::write(&snapshot_path, &bytes);
                    }

                    return Ok(format!(
                        "{}{}__{}__\nCanvas snapshot captured for project '{}'.",
                        IMAGE_BASE64_PREFIX, mime, b64, project_id
                    ));
                }
            }
            Ok(serde_json::json!({
                "status": "error",
                "message": "Snapshot returned invalid data"
            }).to_string())
        }
        Ok(Err(_)) => {
            Ok(serde_json::json!({
                "status": "error",
                "message": "Snapshot request was cancelled"
            }).to_string())
        }
        Err(_) => {
            // Cleanup
            let mut pending = PENDING_SNAPSHOTS.lock().unwrap();
            pending.remove(&request_id);
            Ok(serde_json::json!({
                "status": "error",
                "message": "Snapshot timed out (15s). Make sure the canvas panel is open and visible."
            }).to_string())
        }
    }
}

async fn action_eval_js(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for eval_js"))?;
    let js_code = args
        .get("js")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'js' parameter for eval_js"))?;

    // Register pending eval request
    let request_id = uuid::Uuid::new_v4().to_string();
    let rx = {
        let (tx, rx) = tokio::sync::oneshot::channel::<EvalResult>();
        let mut pending = PENDING_EVALS.lock().unwrap();
        pending.insert(request_id.clone(), tx);
        rx
    }; // MutexGuard dropped here before any .await

    // Emit eval request to frontend
    emit_canvas_event(
        "canvas_eval_request",
        &serde_json::json!({
            "projectId": project_id,
            "requestId": request_id,
            "code": js_code,
        }),
    );

    // Wait for response (timeout 10s)
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(eval_result)) => {
            if let Some(error) = eval_result.error {
                Ok(serde_json::json!({
                    "status": "error",
                    "error": error,
                    "message": format!("JavaScript execution error: {}", error)
                }).to_string())
            } else {
                Ok(serde_json::json!({
                    "status": "ok",
                    "result": eval_result.result.unwrap_or_default(),
                }).to_string())
            }
        }
        Ok(Err(_)) => Ok(r#"{"status":"error","message":"Eval request cancelled"}"#.to_string()),
        Err(_) => {
            let mut pending = PENDING_EVALS.lock().unwrap();
            pending.remove(&request_id);
            Ok(r#"{"status":"error","message":"Eval timed out (10s). Make sure the canvas panel is open."}"#.to_string())
        }
    }
}

async fn action_export(args: &Value) -> Result<String> {
    let project_id = args
        .get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'project_id' parameter for export"))?;
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("html");

    let db = get_canvas_db()?;
    let project = db
        .get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Canvas project '{}' not found", project_id))?;

    let project_dir = paths::canvas_project_dir(project_id)?;

    match format {
        "html" => {
            let html_path = project_dir.join("index.html");
            let content = std::fs::read_to_string(&html_path)?;
            Ok(serde_json::json!({
                "status": "ok",
                "format": "html",
                "path": html_path.to_string_lossy(),
                "content_length": content.len(),
                "message": format!("HTML export ready at: {}", html_path.display())
            }).to_string())
        }
        "markdown" => {
            // Find content.md if exists
            let md_path = project_dir.join("content.md");
            if md_path.exists() {
                let content = std::fs::read_to_string(&md_path)?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "format": "markdown",
                    "path": md_path.to_string_lossy(),
                    "content": content,
                }).to_string())
            } else {
                Ok(serde_json::json!({
                    "status": "error",
                    "message": format!("No markdown source for project '{}' (content_type: {})", project.title, project.content_type)
                }).to_string())
            }
        }
        _ => {
            Err(anyhow::anyhow!("Unsupported export format '{}'. Supported: html, markdown", format))
        }
    }
}

// ── Snapshot/Eval Response Infrastructure ───────────────────────────

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

pub struct SnapshotData {
    pub data_url: Option<String>,
    pub error: Option<String>,
}

pub struct EvalResult {
    pub result: Option<String>,
    pub error: Option<String>,
}

use std::sync::LazyLock;

static PENDING_SNAPSHOTS: LazyLock<StdMutex<HashMap<String, tokio::sync::oneshot::Sender<SnapshotData>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

static PENDING_EVALS: LazyLock<StdMutex<HashMap<String, tokio::sync::oneshot::Sender<EvalResult>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// Called from Tauri command when frontend submits a snapshot result.
pub fn submit_snapshot(request_id: &str, data_url: Option<String>, error: Option<String>) {
    let mut pending = PENDING_SNAPSHOTS.lock().unwrap();
    if let Some(tx) = pending.remove(request_id) {
        let _ = tx.send(SnapshotData { data_url, error });
    }
}

/// Called from Tauri command when frontend submits an eval result.
pub fn submit_eval_result(request_id: &str, result: Option<String>, error: Option<String>) {
    let mut pending = PENDING_EVALS.lock().unwrap();
    if let Some(tx) = pending.remove(request_id) {
        let _ = tx.send(EvalResult { result, error });
    }
}

// ── Tauri Commands ─────────────────────────────────────────────────

#[tauri::command]
pub async fn canvas_submit_snapshot(
    request_id: String,
    data_url: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    submit_snapshot(&request_id, data_url, error);
    Ok(())
}

#[tauri::command]
pub async fn canvas_submit_eval_result(
    request_id: String,
    result: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    submit_eval_result(&request_id, result, error);
    Ok(())
}

#[tauri::command]
pub async fn get_canvas_config() -> Result<CanvasConfig, String> {
    let store = crate::provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.canvas)
}

#[tauri::command]
pub async fn save_canvas_config(config: CanvasConfig) -> Result<(), String> {
    let mut store = crate::provider::load_store().map_err(|e| e.to_string())?;
    store.canvas = config;
    crate::provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_canvas_projects() -> Result<String, String> {
    let db = get_canvas_db().map_err(|e| e.to_string())?;
    let projects = db.list_projects().map_err(|e| e.to_string())?;
    serde_json::to_string(&projects).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_canvas_project(project_id: String) -> Result<String, String> {
    let db = get_canvas_db().map_err(|e| e.to_string())?;
    let project = db.get_project(&project_id).map_err(|e| e.to_string())?;
    serde_json::to_string(&project).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_canvas_project(project_id: String) -> Result<(), String> {
    let db = get_canvas_db().map_err(|e| e.to_string())?;
    project::delete_project(&db, &project_id).map_err(|e| e.to_string())
}
