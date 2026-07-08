//! 设计空间 HTTP 路由（owner 平面薄壳，逻辑全在 `ha_core::design::service`）。
//!
//! Body 方法（POST/PUT）接收 wrapper（`{ input }`），与前端 transport-http 把整个
//! remaining args 作 body 的行为对齐（同 knowledge `CreateKbBody`）；GET/DELETE 用
//! path 参数，避免 body 与 path 参数混用。

use axum::extract::{Path, Query, Request};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use ha_core::design::extract::Direction;
use ha_core::design::service::{
    self, BindingSyncReport, CreateArtifactInput, CreateProjectInput, ElementPatch,
    ExtractSystemInput, SaveSystemInput, UpdateProjectInput,
};
use ha_core::design::{
    DesignArtifact, DesignArtifactVersion, DesignChatThread, DesignCodeBinding, DesignComment,
    DesignProject, DesignSystemMeta,
};
use ha_core::paths;
use ha_core::session::SessionMeta;

use crate::error::AppError;
use crate::routes::file_serve::{
    apply_inline_media_headers, contained_canonical, resolve_mime_for_path,
    validate_safe_rest_path, HeaderOpts, MimeOpts,
};

/// 设计空间 id（UUID-ish）：仅 ASCII 字母数字 + `-`/`_`，长度受限，
/// 挡住 `..` / `/` / shell 元字符。
fn validate_id(id: &str) -> Result<(), AppError> {
    if id.is_empty() || id.len() > 128 {
        return Err(AppError::bad_request("invalid design id"));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AppError::bad_request("invalid design id"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectBody {
    pub input: CreateProjectInput,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectBody {
    pub input: UpdateProjectInput,
}

#[derive(Debug, Deserialize)]
pub struct CreateArtifactBody {
    pub input: CreateArtifactInput,
}

#[derive(Debug, Deserialize)]
pub struct SaveSystemBody {
    pub input: SaveSystemInput,
}

#[derive(Debug, Deserialize)]
pub struct PatchBody {
    pub input: ElementPatch,
}

#[derive(Debug, Deserialize)]
pub struct ExtractSystemBody {
    pub input: ExtractSystemInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDesignMdBody {
    #[serde(default)]
    pub name: String,
    pub md: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFigmaBody {
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeDirectionsBody {
    pub brief: String,
    #[serde(default)]
    pub count: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPptxBody {
    pub slides: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportZipBody {
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBody {
    pub version_id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCommentBody {
    #[serde(default)]
    pub oid: Option<i64>,
    #[serde(default)]
    pub rel_x: f64,
    #[serde(default)]
    pub rel_y: f64,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    pub body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateCommentBody {
    #[serde(default)]
    pub oid: Option<i64>,
    #[serde(default)]
    pub rel_x: f64,
    #[serde(default)]
    pub rel_y: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommentBody {
    pub body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveCommentBody {
    pub resolved: bool,
}

// ── Projects ───────────────────────────────────────────────────────

/// `GET /api/design/projects`
pub async fn list_projects() -> Result<Json<Vec<DesignProject>>, AppError> {
    Ok(Json(
        service::list_projects().map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/projects`
pub async fn create_project(
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<DesignProject>, AppError> {
    Ok(Json(
        service::create_project(body.input).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/projects/{id}`
pub async fn get_project(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    match service::get_project(&id).map_err(|e| AppError::internal(e.to_string()))? {
        Some(p) => Ok(Json(serde_json::to_value(p).unwrap_or(Value::Null))),
        None => Err(AppError::not_found("design project not found")),
    }
}

/// `PUT /api/design/projects` — update (id inside body).
pub async fn update_project(
    Json(body): Json<UpdateProjectBody>,
) -> Result<Json<DesignProject>, AppError> {
    validate_id(&body.input.id)?;
    Ok(Json(
        service::update_project(body.input).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `DELETE /api/design/projects/{id}`
pub async fn delete_project(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    service::delete_project(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/design/projects/{id}/duplicate` — deep-copy a project (artifacts + versions).
pub async fn duplicate_project(Path(id): Path<String>) -> Result<Json<DesignProject>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        service::duplicate_project(&id).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

// ── Artifacts ──────────────────────────────────────────────────────

/// `GET /api/design/projects/{project_id}/artifacts`
pub async fn list_artifacts(
    Path(project_id): Path<String>,
) -> Result<Json<Vec<DesignArtifact>>, AppError> {
    validate_id(&project_id)?;
    Ok(Json(
        service::list_artifacts(&project_id).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts` — create (projectId inside body).
pub async fn create_artifact(
    Json(body): Json<CreateArtifactBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&body.input.project_id)?;
    Ok(Json(
        service::create_artifact_generating(body.input)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/generate` — streaming generate (returns generating
/// shell immediately; content streams via `design:generate_delta` over WS).
pub async fn generate_artifact(
    Json(body): Json<CreateArtifactBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&body.input.project_id)?;
    Ok(Json(
        service::generate_design_artifact(body.input)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts` — all artifacts across projects (library wall).
pub async fn list_all_artifacts() -> Result<Json<Vec<DesignArtifact>>, AppError> {
    Ok(Json(
        service::list_all_artifacts().map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}` — artifact + resolved preview path.
pub async fn get_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    match service::get_artifact_view(&id).map_err(|e| AppError::internal(e.to_string()))? {
        Some(v) => Ok(Json(serde_json::to_value(v).unwrap_or(Value::Null))),
        None => Err(AppError::not_found("design artifact not found")),
    }
}

/// `DELETE /api/design/artifacts/{id}`
pub async fn delete_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    service::delete_artifact(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    pub format: Option<String>,
}

/// `GET /api/design/artifacts/{id}/export?format=html`
pub async fn export_artifact(
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let format = q.format.as_deref().unwrap_or("html");
    let res =
        service::export_artifact(&id, format).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(res).unwrap_or(Value::Null)))
}

/// `GET /api/design/artifacts/{id}/handoff` — developer handoff ZIP (base64 in `content`):
/// clean index.html + source/ + multi-platform tokens/ + HANDOFF.md.
pub async fn export_handoff(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let res = service::export_handoff(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(res).unwrap_or(Value::Null)))
}

// ── Code bindings (工程轴 D) ────────────────────────────────────

/// 外部写盘门：HTTP 侧默认禁写外部工程，需 `filesystem.allowRemoteWrites`（桌面 Tauri 不受限）。
fn ensure_design_writes_allowed() -> Result<(), AppError> {
    if ha_core::config::cached_config()
        .filesystem
        .allow_remote_writes
    {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "remote file writes are disabled; enable filesystem.allowRemoteWrites to sync tokens to a code project",
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindCodeBody {
    pub system_id: String,
    pub target_dir: String,
    #[serde(default)]
    pub subfolder: Option<String>,
    #[serde(default)]
    pub formats: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingsQuery {
    #[serde(default)]
    pub system_id: Option<String>,
}

/// `POST /api/design/bindings` — bind a design system to a code project (write-gated).
pub async fn bind_code(
    Json(body): Json<BindCodeBody>,
) -> Result<Json<DesignCodeBinding>, AppError> {
    ensure_design_writes_allowed()?;
    let b = service::bind_code_project(
        &body.system_id,
        &body.target_dir,
        body.subfolder.as_deref().unwrap_or(""),
        &body.formats.unwrap_or_default(),
    )
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(b))
}

/// `POST /api/design/bindings/{id}/sync` — write tokens to the bound dir (write-gated).
pub async fn sync_code(Path(id): Path<i64>) -> Result<Json<BindingSyncReport>, AppError> {
    ensure_design_writes_allowed()?;
    let r = service::sync_code_binding(id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(r))
}

/// `GET /api/design/bindings?systemId=` — list bindings (read-only).
pub async fn list_code_bindings(
    Query(q): Query<BindingsQuery>,
) -> Result<Json<Vec<DesignCodeBinding>>, AppError> {
    let list = service::list_code_bindings(q.system_id.as_deref())
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(list))
}

/// `DELETE /api/design/bindings/{id}` — unbind (no external write).
pub async fn unbind_code(Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    service::unbind_code_project(id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestyleBody {
    #[serde(default)]
    pub system_id: Option<String>,
}

/// `POST /api/design/artifacts/{id}/restyle` — 就地换设计系统（重渲染 + 落新版本）。
pub async fn restyle_artifact(
    Path(id): Path<String>,
    Json(body): Json<RestyleBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    let a = service::restyle_artifact(&id, body.system_id.as_deref())
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(a))
}

/// `POST /api/design/artifacts/{id}/critique` — 5-dimension quality review.
pub async fn critique_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let res = service::critique_artifact(&id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(res).unwrap_or(Value::Null)))
}

/// `POST /api/design/patch` — visual edit (element style/text writeback).
pub async fn patch_element(Json(body): Json<PatchBody>) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&body.input.artifact_id)?;
    Ok(Json(
        service::patch_element(body.input).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}/versions`
pub async fn list_versions(
    Path(id): Path<String>,
) -> Result<Json<Vec<DesignArtifactVersion>>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        service::list_versions(&id).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}/versions/{version}/html` — snapshot HTML for preview.
/// Bare JSON string to mirror the Tauri command's `String` return (transport parity).
pub async fn get_version_html(
    Path((id, version)): Path<(String, i64)>,
) -> Result<Json<String>, AppError> {
    validate_id(&id)?;
    let html = service::get_artifact_version_html(&id, version)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(html))
}

// ── Shares（B7-1 只读分享）─────────────────────────────────────────

/// `POST /api/design/artifacts/{id}/share` — 建/取只读分享 token（owner，幂等）。
pub async fn create_share(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let token = service::create_share(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "token": token })))
}

/// `GET /api/design/artifacts/{id}/share` — 产物当前分享 token（owner；无则 null）。
pub async fn get_share(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let token =
        service::share_token_for_artifact(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "token": token })))
}

/// `DELETE /api/design/artifacts/{id}/share` — 撤销分享（owner）。
pub async fn revoke_share(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let ok =
        service::revoke_share_for_artifact(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `GET /api/design/share/{token}` — **公开（无鉴权）**只读快照。token 是唯一不可猜凭证；
/// 返回干净自包含 HTML（`render_clean`，无 bridge/oid），`sandbox allow-scripts` 隔离到 opaque
/// origin（不能读服务端 cookie / 同源接口）+ no-referrer。token 非法 / 查不到一律 404。
pub async fn serve_share(Path(token): Path<String>) -> Result<Response, AppError> {
    // token 形态白名单：纯 ASCII 字母数字（uuid simple = 32 hex），挡路径穿越 / 注入。
    if token.is_empty() || token.len() > 128 || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::not_found("share not found"));
    }
    match service::render_share_html(&token).map_err(|e| AppError::internal(e.to_string()))? {
        Some(html) => {
            let mut resp = axum::response::Html(html).into_response();
            let h = resp.headers_mut();
            h.insert(
                axum::http::header::CONTENT_SECURITY_POLICY,
                axum::http::HeaderValue::from_static("sandbox allow-scripts"),
            );
            h.insert(
                axum::http::header::REFERRER_POLICY,
                axum::http::HeaderValue::from_static("no-referrer"),
            );
            h.insert(
                axum::http::header::X_CONTENT_TYPE_OPTIONS,
                axum::http::HeaderValue::from_static("nosniff"),
            );
            Ok(resp)
        }
        None => Err(AppError::not_found("share not found")),
    }
}

/// `POST /api/design/artifacts/{id}/restore` — restore a historical version.
pub async fn restore_version(
    Path(id): Path<String>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        service::restore_version(&id, body.version_id)
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/pptx` — assemble PPTX from client-rasterized slide PNGs (base64).
pub async fn export_pptx(Json(body): Json<ExportPptxBody>) -> Result<Json<Value>, AppError> {
    let title = body.title.as_deref().unwrap_or("design");
    let b64 =
        service::export_pptx(&body.slides, title).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "pptx": b64 })))
}

/// `POST /api/design/zip` — single-artifact source bundle (`artifactId`) or
/// project-level bundle (`projectId`). Returns `{ zip: base64 }`.
pub async fn export_zip(Json(body): Json<ExportZipBody>) -> Result<Json<Value>, AppError> {
    let b64 = service::export_zip(body.artifact_id.as_deref(), body.project_id.as_deref())
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "zip": b64 })))
}

// ── Design systems ─────────────────────────────────────────────────

/// `GET /api/design/systems`
pub async fn list_systems() -> Result<Json<Vec<DesignSystemMeta>>, AppError> {
    Ok(Json(
        service::list_systems().map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/systems/{id}` — system meta + prose + tokens.
pub async fn get_system(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let full = service::get_system_full(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(full).unwrap_or(Value::Null)))
}

/// `POST /api/design/systems` — create/update a user design system.
pub async fn save_system(
    Json(body): Json<SaveSystemBody>,
) -> Result<Json<DesignSystemMeta>, AppError> {
    Ok(Json(
        service::save_system(body.input).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `DELETE /api/design/systems/{id}`
pub async fn delete_system(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    service::delete_system(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/design/systems/extract` — reverse-extract a design system.
pub async fn extract_system(
    Json(body): Json<ExtractSystemBody>,
) -> Result<Json<DesignSystemMeta>, AppError> {
    Ok(Json(
        service::extract_system(body.input)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/systems/import` — import a DESIGN.md-spec design system.
pub async fn import_design_md(
    Json(body): Json<ImportDesignMdBody>,
) -> Result<Json<DesignSystemMeta>, AppError> {
    Ok(Json(
        service::import_design_md(&body.name, &body.md)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/systems/figma` — import a design system from a Figma file
/// (owner plane; the access token is passed per-call and never persisted).
pub async fn import_figma_system(
    Json(body): Json<ImportFigmaBody>,
) -> Result<Json<DesignSystemMeta>, AppError> {
    Ok(Json(
        service::import_figma(&body.url, &body.token, body.name.as_deref())
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/systems/{id}/design-md` — export a design system as DESIGN.md.
pub async fn export_design_md(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Value>, AppError> {
    let md = service::export_design_md(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "designMd": md })))
}

/// `GET /api/design/systems/{id}/tokens/export` — export tokens to multi-platform
/// developer formats (CSS/SCSS/TS/Swift/Android/DTCG).
pub async fn export_design_tokens(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Vec<ha_core::design::token_export::TokenExport>>, AppError> {
    let out = service::export_tokens(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(out))
}

/// `POST /api/design/directions` — propose N design direction candidates.
pub async fn propose_directions(
    Json(body): Json<ProposeDirectionsBody>,
) -> Result<Json<Vec<Direction>>, AppError> {
    Ok(Json(
        service::propose_directions(&body.brief, body.count.unwrap_or(4))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/recipes` — built-in design template (recipe) catalog.
pub async fn list_recipes() -> Result<Json<Vec<ha_core::design::recipe::Recipe>>, AppError> {
    Ok(Json(ha_core::design::recipe::builtin_recipes()))
}

/// `GET /api/design/artifacts/{id}/native?format=pdf` — real-browser native capture
/// (vector PDF via printToPDF / full-fidelity PNG via captureScreenshot). Falls back
/// to client rasterization on the frontend when the browser backend is unavailable.
pub async fn export_native(
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let format = q.format.as_deref().unwrap_or("pdf");
    let (data, mime) = ha_core::design::render_native::capture_artifact_b64(&id, format)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "data": data, "mime": mime })))
}

/// `GET /api/design/ffmpeg/doctor` — MP4-export ffmpeg encoder three-state probe.
pub async fn ffmpeg_doctor() -> Result<Json<ha_core::ffmpeg::FfmpegStatus>, AppError> {
    Ok(Json(ha_core::ffmpeg::doctor().await))
}

/// `POST /api/design/ffmpeg/install` — on-demand download the static ffmpeg
/// encoder (progress on `design:ffmpeg_download_progress` WS event).
pub async fn install_ffmpeg() -> Result<Json<Value>, AppError> {
    let binary = ha_core::ffmpeg::install_with_event_bus_progress()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "binaryPath": binary.display().to_string() })))
}

/// `GET /api/design/browser/doctor` — PDF/PNG-export browser-engine three-state probe.
pub async fn browser_doctor(
) -> Result<Json<ha_core::design::render_native::BrowserExportStatus>, AppError> {
    Ok(Json(ha_core::design::render_native::browser_export_status()))
}

/// `POST /api/design/browser/install` — on-demand download the Chromium runtime
/// (progress on `browser:chromium_download_progress` WS event).
pub async fn install_browser() -> Result<Json<Value>, AppError> {
    let binary = ha_core::browser::runtime::install_with_event_bus_progress()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "binaryPath": binary.display().to_string() })))
}

/// `GET /api/design/projects/{project_id}/artifacts/{artifact_id}/{*rest}` —
/// serve a file from an artifact directory (the preview iframe loads
/// `…/index.html` through this route). Three-gate path containment.
pub async fn serve_artifact_file(
    Path((project_id, artifact_id, rest)): Path<(String, String, String)>,
    request: Request,
) -> Result<Response, AppError> {
    validate_id(&project_id)?;
    validate_id(&artifact_id)?;
    validate_safe_rest_path(&rest)?;

    let base_dir = paths::design_artifact_dir(&project_id, &artifact_id)
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
        .map_err(|e| AppError::internal(format!("serve design file: {}", e)))?
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

// ── Comments (批注钉) ────────────────────────────────────────────────

/// `GET /api/design/artifacts/{id}/comments`
pub async fn list_comments(
    Path(artifact_id): Path<String>,
) -> Result<Json<Vec<DesignComment>>, AppError> {
    validate_id(&artifact_id)?;
    Ok(Json(
        service::list_comments(&artifact_id).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/comments`
pub async fn add_comment(
    Path(artifact_id): Path<String>,
    Json(payload): Json<AddCommentBody>,
) -> Result<Json<DesignComment>, AppError> {
    validate_id(&artifact_id)?;
    Ok(Json(
        service::add_comment(
            &artifact_id,
            payload.oid,
            payload.rel_x,
            payload.rel_y,
            payload.tag.as_deref(),
            payload.snippet.as_deref(),
            &payload.body,
        )
        .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/comments/{comment_id}/relocate`
pub async fn relocate_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
    Json(payload): Json<RelocateCommentBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = service::relocate_comment(
        &artifact_id,
        comment_id,
        payload.oid,
        payload.rel_x,
        payload.rel_y,
    )
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `PUT /api/design/artifacts/{id}/comments/{comment_id}`
pub async fn update_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
    Json(payload): Json<UpdateCommentBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = service::update_comment_body(&artifact_id, comment_id, &payload.body)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `POST /api/design/artifacts/{id}/comments/{comment_id}/resolve`
pub async fn resolve_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
    Json(payload): Json<ResolveCommentBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = service::set_comment_resolved(&artifact_id, comment_id, payload.resolved)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `DELETE /api/design/artifacts/{id}/comments/{comment_id}`
pub async fn delete_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = service::delete_comment(&artifact_id, comment_id)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `POST /api/design/artifacts/{id}/comments/{comment_id}/refine` — 让 AI 按批注精修产物。
pub async fn refine_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&artifact_id)?;
    Ok(Json(
        service::refine_artifact_with_comment(&artifact_id, comment_id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/systems/{id}/kit` — 设计系统套件视图自包含 HTML（返回 JSON 字符串，
/// 与 Tauri `get_design_system_kit_cmd` 的 `String` 返回一致，前端 `call<string>` 两态通用）。
pub async fn system_kit(Path(id): Path<String>) -> Result<Json<String>, AppError> {
    validate_id(&id)?;
    let html = service::get_system_kit_html(&id).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(html))
}

#[derive(serde::Deserialize)]
pub struct ReviewBody {
    pub action: String,
}

/// `POST /api/design/artifacts/{id}/review` — 反-slop 自查复查（recheck|dismiss）。
pub async fn review_artifact(
    Path(id): Path<String>,
    Json(body): Json<ReviewBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        service::review_artifact(&id, &body.action)
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

// ── Design-space per-project chat threads ───────────────────────

#[derive(serde::Deserialize)]
pub struct ThreadListQuery {
    pub query: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /api/design/projects/{project_id}/chat/thread` — default-load target:
/// the most recent chat thread anchored to this project (`None` = empty state).
pub async fn chat_thread_latest(
    Path(project_id): Path<String>,
) -> Result<Json<Option<SessionMeta>>, AppError> {
    validate_id(&project_id)?;
    Ok(Json(
        service::design_chat_thread_latest(&project_id)
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/projects/{project_id}/chat/threads` — history picker page.
pub async fn chat_threads_list(
    Path(project_id): Path<String>,
    Query(q): Query<ThreadListQuery>,
) -> Result<Json<Vec<DesignChatThread>>, AppError> {
    validate_id(&project_id)?;
    Ok(Json(
        service::design_chat_threads_list(&project_id, q.query.as_deref(), q.limit, q.offset)
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_accepts_typical() {
        assert!(validate_id("abc-123").is_ok());
        assert!(validate_id("550e8400e29b41d4a716446655440000").is_ok());
    }

    #[test]
    fn id_rejects_bad() {
        assert!(validate_id("").is_err());
        assert!(validate_id("..").is_err());
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("a\\b").is_err());
    }
}
