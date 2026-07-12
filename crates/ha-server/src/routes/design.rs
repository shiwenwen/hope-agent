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
    ExtractSystemInput, RemoveElementResult, SaveSystemInput, UpdateProjectInput,
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
#[serde(rename_all = "camelCase")]
pub struct RemoveElementBody {
    pub oid: u32,
    #[serde(default)]
    pub expected_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertElementBody {
    #[serde(default)]
    pub parent_oid: Option<u32>,
    #[serde(default)]
    pub after_oid: Option<u32>,
    #[serde(default)]
    pub insert_offset: usize,
    pub html: String,
    #[serde(default)]
    pub expected_hash: Option<String>,
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
        ha_core::blocking::run_blocking(service::list_projects)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/projects`
pub async fn create_project(
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<DesignProject>, AppError> {
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::create_project(body.input))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/projects/{id}`
pub async fn get_project(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    match ha_core::blocking::run_blocking(move || service::get_project(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
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
        ha_core::blocking::run_blocking(move || service::update_project(body.input))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `DELETE /api/design/projects/{id}`
pub async fn delete_project(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::delete_project(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/design/projects/{id}/duplicate` — deep-copy a project (artifacts + versions).
pub async fn duplicate_project(Path(id): Path<String>) -> Result<Json<DesignProject>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::duplicate_project(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameArtifactBody {
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderArtifactsBody {
    pub ordered_ids: Vec<String>,
}

/// `PUT /api/design/artifacts/{id}/title` — 轻量改名产物。
pub async fn rename_artifact(
    Path(id): Path<String>,
    Json(body): Json<RenameArtifactBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::rename_artifact(&id, &body.title))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/duplicate` — 复制产物（同项目内）。
pub async fn duplicate_artifact(Path(id): Path<String>) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::duplicate_artifact(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/projects/{id}/artifacts/reorder` — 重排项目内产物页面顺序。
pub async fn reorder_artifacts(
    Path(id): Path<String>,
    Json(body): Json<ReorderArtifactsBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::reorder_artifacts(&id, &body.ordered_ids))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

// ── 页面分组文件夹 ──
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderBody {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameFolderBody {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFolderQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveArtifactBody {
    pub folder: String,
}

/// `GET /api/design/projects/{id}/folders` — 项目内全部文件夹路径。
pub async fn list_folders(Path(id): Path<String>) -> Result<Json<Vec<String>>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::list_folders(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/projects/{id}/folders` — 新建（空）文件夹。
pub async fn create_folder(
    Path(id): Path<String>,
    Json(body): Json<CreateFolderBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::create_folder(&id, &body.name))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `PUT /api/design/projects/{id}/folders` — 文件夹改名/移动（body `from`/`to`）。
pub async fn rename_folder(
    Path(id): Path<String>,
    Json(body): Json<RenameFolderBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::rename_folder(&id, &body.from, &body.to))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/design/projects/{id}/folders?path=…` — 删文件夹（页面移到根，query `path`）。
pub async fn delete_folder(
    Path(id): Path<String>,
    Query(q): Query<DeleteFolderQuery>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::delete_folder(&id, &q.path))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `PUT /api/design/artifacts/{id}/folder` — 把页面移到某文件夹（body `folder`，空=根）。
pub async fn move_artifact(
    Path(id): Path<String>,
    Json(body): Json<MoveArtifactBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || {
            service::move_artifact_to_folder(&id, &body.folder)
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

// ── Artifacts ──────────────────────────────────────────────────────

/// `GET /api/design/projects/{project_id}/artifacts`
pub async fn list_artifacts(
    Path(project_id): Path<String>,
) -> Result<Json<Vec<DesignArtifact>>, AppError> {
    validate_id(&project_id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::list_artifacts(&project_id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenterNotesBody {
    pub notes: Vec<String>,
}

/// `POST /api/design/threads/{sessionId}/fork` — fork 设计对话线程，返回新 sessionId。
pub async fn fork_thread(Path(session_id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&session_id)?;
    let new_id =
        ha_core::blocking::run_blocking(move || ha_core::design::threads::fork_thread(&session_id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "sessionId": new_id })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InpaintBody {
    pub prompt: String,
    pub mask_b64: String,
}

/// `POST /api/design/artifacts/{id}/inpaint` — image 产物按蒙版局部重绘。
pub async fn inpaint_image(
    Path(id): Path<String>,
    Json(body): Json<InpaintBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    let a = service::inpaint_image_artifact(&id, &body.prompt, &body.mask_b64)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(a))
}

#[derive(Deserialize)]
pub struct PageStyleBody {
    pub props: std::collections::BTreeMap<String, String>,
}

/// `PUT /api/design/artifacts/{id}/page-style` — 页面级样式编辑（body 层）。
pub async fn patch_page_style(
    Path(id): Path<String>,
    Json(body): Json<PageStyleBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    let a = ha_core::blocking::run_blocking(move || {
        service::patch_page_style(&id, body.props.into_iter().collect())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(a))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDirBody {
    pub rtl: bool,
}

/// `PUT /api/design/artifacts/{id}/dir` — 设置产物文本方向（RTL/LTR）。
pub async fn set_artifact_dir(
    Path(id): Path<String>,
    Json(body): Json<SetDirBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    let a = ha_core::blocking::run_blocking(move || service::set_artifact_dir(&id, body.rtl))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(a))
}

/// `PUT /api/design/artifacts/{id}/presenter-notes` — 保存 deck 演讲者备注。
pub async fn set_presenter_notes(
    Path(id): Path<String>,
    Json(body): Json<PresenterNotesBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::set_presenter_notes(&id, body.notes))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportImageBody {
    pub project_id: String,
    pub title: String,
    pub mime: String,
    pub data_b64: String,
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandPackBody {
    pub project_id: String,
    pub brief: String,
    pub kinds: Vec<String>,
    #[serde(default)]
    pub system_id: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub reference_image_b64: Option<String>,
    #[serde(default)]
    pub reference_image_mime: Option<String>,
    #[serde(default)]
    pub model_override: Option<ha_core::provider::ActiveModel>,
}

/// `POST /api/design/artifacts/brand-pack` — 一个 brief 批量生成一组共享设计系统的协调产物。
/// 可带参考图（每件产物都真看原图）与显式模型（单模型不降级）。
pub async fn generate_brand_pack(
    Json(body): Json<BrandPackBody>,
) -> Result<Json<Vec<DesignArtifact>>, AppError> {
    validate_id(&body.project_id)?;
    let arts = service::generate_brand_pack(
        &body.project_id,
        &body.brief,
        body.kinds,
        body.system_id,
        body.folder,
        body.reference_image_b64,
        body.reference_image_mime,
        body.model_override,
    )
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(arts))
}

/// `POST /api/design/artifacts/import-image` — 拖入导入：base64 图片 → image 产物。
pub async fn import_image(
    Json(body): Json<ImportImageBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&body.project_id)?;
    let art = ha_core::blocking::run_blocking(move || {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body.data_b64.trim())
            .map_err(|e| anyhow::anyhow!("base64 decode failed: {e}"))?;
        service::import_image_artifact(
            &body.project_id,
            &body.title,
            &body.mime,
            &bytes,
            body.folder,
        )
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(art))
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
        ha_core::blocking::run_blocking(service::list_all_artifacts)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}` — artifact + resolved preview path.
pub async fn get_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    match ha_core::blocking::run_blocking(move || service::get_artifact_view(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        Some(v) => Ok(Json(serde_json::to_value(v).unwrap_or(Value::Null))),
        None => Err(AppError::not_found("design artifact not found")),
    }
}

/// `DELETE /api/design/artifacts/{id}`
pub async fn delete_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::delete_artifact(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
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
    let res = ha_core::blocking::run_blocking(move || {
        service::export_artifact(&id, q.format.as_deref().unwrap_or("html"))
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(res).unwrap_or(Value::Null)))
}

/// `GET /api/design/artifacts/{id}/handoff` — developer handoff ZIP (base64 in `content`):
/// clean index.html + source/ + multi-platform tokens/ + HANDOFF.md.
pub async fn export_handoff(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let res = ha_core::blocking::run_blocking(move || service::export_handoff(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
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
    let b = ha_core::blocking::run_blocking(move || {
        service::bind_code_project(
            &body.system_id,
            &body.target_dir,
            body.subfolder.as_deref().unwrap_or(""),
            &body.formats.unwrap_or_default(),
        )
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(b))
}

/// `POST /api/design/bindings/{id}/sync` — write tokens to the bound dir (write-gated).
pub async fn sync_code(Path(id): Path<i64>) -> Result<Json<BindingSyncReport>, AppError> {
    ensure_design_writes_allowed()?;
    let r = ha_core::blocking::run_blocking(move || service::sync_code_binding(id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(r))
}

/// `GET /api/design/bindings?systemId=` — list bindings (read-only).
pub async fn list_code_bindings(
    Query(q): Query<BindingsQuery>,
) -> Result<Json<Vec<DesignCodeBinding>>, AppError> {
    let list = ha_core::blocking::run_blocking(move || {
        service::list_code_bindings(q.system_id.as_deref())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(list))
}

/// `DELETE /api/design/bindings/{id}` — unbind (no external write).
pub async fn unbind_code(Path(id): Path<i64>) -> Result<Json<Value>, AppError> {
    ha_core::blocking::run_blocking(move || service::unbind_code_project(id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
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
    let a = ha_core::blocking::run_blocking(move || {
        service::restyle_artifact(&id, body.system_id.as_deref())
    })
    .await
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
        ha_core::blocking::run_blocking(move || service::patch_element(body.input))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/remove-element` — 删元素 + 回传重建上下文（结构 undo）。
pub async fn remove_element(
    Path(id): Path<String>,
    Json(body): Json<RemoveElementBody>,
) -> Result<Json<RemoveElementResult>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || {
            service::remove_element_owner(&id, body.oid, body.expected_hash)
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/insert-element` — 重插被删元素（结构 undo 撤销侧）。
pub async fn insert_element(
    Path(id): Path<String>,
    Json(body): Json<InsertElementBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || {
            service::insert_element(
                &id,
                body.parent_oid,
                body.after_oid,
                body.insert_offset,
                &body.html,
                body.expected_hash,
            )
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/cancel` — 停止在途流式生成（降级占位，不删）。
pub async fn cancel_generation(Path(id): Path<String>) -> Result<Json<bool>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::cancel_artifact_generation(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}/versions`
pub async fn list_versions(
    Path(id): Path<String>,
) -> Result<Json<Vec<DesignArtifactVersion>>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::list_versions(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}/versions/{version}/html` — snapshot HTML for preview.
/// Bare JSON string to mirror the Tauri command's `String` return (transport parity).
pub async fn get_version_html(
    Path((id, version)): Path<(String, i64)>,
) -> Result<Json<String>, AppError> {
    validate_id(&id)?;
    let html =
        ha_core::blocking::run_blocking(move || service::get_artifact_version_html(&id, version))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(html))
}

// ── Shares（B7-1 只读分享）─────────────────────────────────────────

/// `POST /api/design/artifacts/{id}/share` — 建/取只读分享 token（owner，幂等）。
pub async fn create_share(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let token = ha_core::blocking::run_blocking(move || service::create_share(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "token": token })))
}

/// `GET /api/design/artifacts/{id}/share` — 产物当前分享 token（owner；无则 null）。
pub async fn get_share(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let token = ha_core::blocking::run_blocking(move || service::share_token_for_artifact(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "token": token })))
}

/// `DELETE /api/design/artifacts/{id}/share` — 撤销分享（owner）。
pub async fn revoke_share(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let ok = ha_core::blocking::run_blocking(move || service::revoke_share_for_artifact(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
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
    match ha_core::blocking::run_blocking(move || service::render_share_html(&token))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
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

// ── Cloudflare Pages 部署（B7-2，owner）─────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfConfigBody {
    pub api_token: String,
    pub account_id: String,
}

/// `PUT /api/design/deploy/config` — 保存 CF token（0600）+ account。
pub async fn save_deploy_config(Json(body): Json<CfConfigBody>) -> Result<Json<Value>, AppError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::design::deploy::save_cf_config(&body.api_token, &body.account_id)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/design/deploy/config` — 读配置（**token 脱敏**）。
pub async fn get_deploy_config() -> Result<Json<Value>, AppError> {
    let cfg = ha_core::blocking::run_blocking(ha_core::design::deploy::public_cf_config)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(cfg).unwrap_or(Value::Null)))
}

/// `POST /api/design/artifacts/{id}/deploy` — 部署到 CF Pages，返回 `{ url }`。
pub async fn deploy_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let url = ha_core::design::deploy::deploy_artifact(&id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/design/deploy/probe` — 探测部署 URL 是否已生效（body `{ url }`），返回 `{ ready, status }`。
pub async fn probe_deploy(Json(body): Json<Value>) -> Result<Json<Value>, AppError> {
    let url = body
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing url"))?;
    let r = ha_core::design::deploy::probe_deploy_ready(url)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ready": r.ready, "status": r.status })))
}

/// `POST /api/design/artifacts/{id}/domains` — 绑定自定义域名（body `{ domain }`），返回 `{ name, status }`。
pub async fn bind_domain(
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let domain = body
        .get("domain")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing domain"))?;
    let d = ha_core::design::deploy::bind_custom_domain(&id, domain)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "name": d.name, "status": d.status })))
}

/// `GET /api/design/artifacts/{id}/domains` — 列出已绑定的自定义域名及验证状态。
pub async fn list_domains(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let list = ha_core::design::deploy::list_custom_domains(&id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!(list
        .into_iter()
        .map(|d| json!({ "name": d.name, "status": d.status }))
        .collect::<Vec<_>>())))
}

/// `GET /api/design/artifacts/{id}/quality-review` — 确定性多镜头质量审查（a11y/内容/语义）。
pub async fn quality_review_artifact(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let findings = ha_core::blocking::run_blocking(move || service::quality_review_artifact(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(findings).unwrap_or(Value::Null)))
}

/// `GET /api/design/artifacts/{id}/deployments` — 部署历史（最新在前）。
pub async fn list_deployments(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let list =
        ha_core::blocking::run_blocking(move || ha_core::design::service::list_deployments(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(list).unwrap_or(Value::Null)))
}

/// `GET /api/design/artifacts/{id}/deploy/preflight` — 部署预检（CF/Vercel 共用）。
pub async fn preflight_deploy(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let report =
        ha_core::blocking::run_blocking(move || ha_core::design::deploy::preflight_artifact(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(report).unwrap_or(Value::Null)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VercelConfigBody {
    pub api_token: String,
    #[serde(default)]
    pub team_id: String,
}

/// `PUT /api/design/deploy/vercel/config` — 保存 Vercel token（0600）+ team。
pub async fn save_vercel_config(
    Json(body): Json<VercelConfigBody>,
) -> Result<Json<Value>, AppError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::design::deploy_vercel::save_vercel_config(&body.api_token, &body.team_id)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/design/deploy/vercel/config` — 读配置（**token 脱敏**）。
pub async fn get_vercel_config() -> Result<Json<Value>, AppError> {
    let cfg = ha_core::blocking::run_blocking(ha_core::design::deploy_vercel::public_vercel_config)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(cfg).unwrap_or(Value::Null)))
}

/// `POST /api/design/artifacts/{id}/deploy/vercel` — 部署到 Vercel，返回 `{ url }`。
pub async fn deploy_artifact_vercel(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let url = ha_core::design::deploy_vercel::deploy_artifact(&id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "url": url })))
}

/// `POST /api/design/artifacts/{id}/ensure-fresh` — 自愈渲染版本（工具层升级对老产物生效）。
/// 返回 `bool`（是否重渲染），与 Tauri `ensure_design_artifact_fresh_cmd` 同形。
pub async fn ensure_artifact_fresh(Path(id): Path<String>) -> Result<Json<bool>, AppError> {
    validate_id(&id)?;
    let rerendered =
        ha_core::blocking::run_blocking(move || service::ensure_artifact_render_fresh(&id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(rerendered))
}

/// `POST /api/design/artifacts/{id}/restore` — restore a historical version.
pub async fn restore_version(
    Path(id): Path<String>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<DesignArtifact>, AppError> {
    validate_id(&id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::restore_version(&id, body.version_id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/artifacts/{id}/pptx-outline` — 结构化可编辑文本 PPTX（服务端抽 deck 大纲）。
pub async fn export_pptx_outline(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let pptx = ha_core::blocking::run_blocking(move || service::export_pptx_outline(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "pptx": pptx })))
}

/// `POST /api/design/pptx` — assemble PPTX from client-rasterized slide PNGs (base64).
pub async fn export_pptx(Json(body): Json<ExportPptxBody>) -> Result<Json<Value>, AppError> {
    let b64 = ha_core::blocking::run_blocking(move || {
        service::export_pptx(&body.slides, body.title.as_deref().unwrap_or("design"))
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "pptx": b64 })))
}

/// `POST /api/design/zip` — single-artifact source bundle (`artifactId`) or
/// project-level bundle (`projectId`). Returns `{ zip: base64 }`.
pub async fn export_zip(Json(body): Json<ExportZipBody>) -> Result<Json<Value>, AppError> {
    let b64 = ha_core::blocking::run_blocking(move || {
        service::export_zip(body.artifact_id.as_deref(), body.project_id.as_deref())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "zip": b64 })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSelectedZipBody {
    #[serde(default)]
    pub artifact_ids: Vec<String>,
}

/// `POST /api/design/zip/selected` — bundle the given artifacts into one ZIP
/// (one folder each + gallery). Returns `{ zip: base64 }`.
pub async fn export_selected_zip(
    Json(body): Json<ExportSelectedZipBody>,
) -> Result<Json<Value>, AppError> {
    let b64 =
        ha_core::blocking::run_blocking(move || service::export_selected_zip(&body.artifact_ids))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "zip": b64 })))
}

// ── Design systems ─────────────────────────────────────────────────

/// `GET /api/design/systems`
pub async fn list_systems() -> Result<Json<Vec<DesignSystemMeta>>, AppError> {
    Ok(Json(
        ha_core::blocking::run_blocking(service::list_systems)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `GET /api/design/systems/{id}` — system meta + prose + tokens.
pub async fn get_system(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    let full = ha_core::blocking::run_blocking(move || service::get_system_full(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(full).unwrap_or(Value::Null)))
}

/// `POST /api/design/systems` — create/update a user design system.
pub async fn save_system(
    Json(body): Json<SaveSystemBody>,
) -> Result<Json<DesignSystemMeta>, AppError> {
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::save_system(body.input))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `DELETE /api/design/systems/{id}`
pub async fn delete_system(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    validate_id(&id)?;
    ha_core::blocking::run_blocking(move || service::delete_system(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `PATCH /api/design/systems/{id}` — rename a user design system (body `{ name }`).
pub async fn rename_system(
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<DesignSystemMeta>, AppError> {
    validate_id(&id)?;
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing name"))?
        .to_string();
    Ok(Json(
        ha_core::blocking::run_blocking(move || service::rename_system(&id, &name))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
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
    let md = ha_core::blocking::run_blocking(move || service::export_design_md(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "designMd": md })))
}

/// `GET /api/design/systems/{id}/tokens/export` — export tokens to multi-platform
/// developer formats (CSS/SCSS/TS/Swift/Android/DTCG).
pub async fn export_design_tokens(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Vec<ha_core::design::token_export::TokenExport>>, AppError> {
    let out = ha_core::blocking::run_blocking(move || service::export_tokens(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
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

#[derive(Deserialize)]
pub struct RecipeDemoQuery {
    /// 注入配色的设计系统 id（camelCase 对齐 Tauri 参数名）。
    #[serde(rename = "systemId")]
    pub system_id: Option<String>,
}

/// `GET /api/design/recipes/{id}/demo?systemId=` — recipe skeleton demo HTML
/// (toolbox hover preview; bare JSON string, mirrors the Tauri command).
pub async fn recipe_demo(
    Path(id): Path<String>,
    Query(q): Query<RecipeDemoQuery>,
) -> Result<Json<String>, AppError> {
    Ok(Json(
        service::get_recipe_demo_html(&id, q.system_id.as_deref())
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
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
        ha_core::blocking::run_blocking(move || service::list_comments(&artifact_id))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/comments`
pub async fn add_comment(
    Path(artifact_id): Path<String>,
    Json(payload): Json<AddCommentBody>,
) -> Result<Json<DesignComment>, AppError> {
    validate_id(&artifact_id)?;
    Ok(Json(
        ha_core::blocking::run_blocking(move || {
            service::add_comment(
                &artifact_id,
                payload.oid,
                payload.rel_x,
                payload.rel_y,
                payload.tag.as_deref(),
                payload.snippet.as_deref(),
                &payload.body,
            )
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

/// `POST /api/design/artifacts/{id}/comments/{comment_id}/relocate`
pub async fn relocate_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
    Json(payload): Json<RelocateCommentBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = ha_core::blocking::run_blocking(move || {
        service::relocate_comment(
            &artifact_id,
            comment_id,
            payload.oid,
            payload.rel_x,
            payload.rel_y,
        )
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `PUT /api/design/artifacts/{id}/comments/{comment_id}`
pub async fn update_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
    Json(payload): Json<UpdateCommentBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = ha_core::blocking::run_blocking(move || {
        service::update_comment_body(&artifact_id, comment_id, &payload.body)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `POST /api/design/artifacts/{id}/comments/{comment_id}/resolve`
pub async fn resolve_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
    Json(payload): Json<ResolveCommentBody>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok = ha_core::blocking::run_blocking(move || {
        service::set_comment_resolved(&artifact_id, comment_id, payload.resolved)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": ok })))
}

/// `DELETE /api/design/artifacts/{id}/comments/{comment_id}`
pub async fn delete_comment(
    Path((artifact_id, comment_id)): Path<(String, i64)>,
) -> Result<Json<Value>, AppError> {
    validate_id(&artifact_id)?;
    let ok =
        ha_core::blocking::run_blocking(move || service::delete_comment(&artifact_id, comment_id))
            .await
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
    let html = ha_core::blocking::run_blocking(move || service::get_system_kit_html(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
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
        ha_core::blocking::run_blocking(move || service::review_artifact(&id, &body.action))
            .await
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
        ha_core::blocking::run_blocking(move || service::design_chat_thread_latest(&project_id))
            .await
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
        ha_core::blocking::run_blocking(move || {
            service::design_chat_threads_list(&project_id, q.query.as_deref(), q.limit, q.offset)
        })
        .await
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
