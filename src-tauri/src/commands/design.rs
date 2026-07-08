//! Tauri commands for the Design Space feature.
//!
//! Thin wrappers around `ha_core::design` — all logic lives in ha-core. These
//! run on the **owner plane** (desktop = trusted local machine): the operator
//! sees all their design projects/artifacts, not gated by any agent access
//! check (that is for the agent `design` tool).

use crate::commands::CmdError;
use ha_core::design::extract::Direction;
use ha_core::design::service::BindingSyncReport;
use ha_core::design::service::{
    self, ArtifactView, CreateArtifactInput, CreateProjectInput, ElementPatch, ExportResult,
    ExtractSystemInput, SaveSystemInput, UpdateProjectInput,
};
use ha_core::design::token_export::TokenExport;
use ha_core::design::{
    CritiqueResult, DesignArtifact, DesignArtifactVersion, DesignChatThread, DesignCodeBinding,
    DesignComment, DesignConfig, DesignProject, DesignSystemFull, DesignSystemMeta,
};
use ha_core::session::SessionMeta;

// ── Projects ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_projects_cmd() -> Result<Vec<DesignProject>, CmdError> {
    service::list_projects().map_err(Into::into)
}

#[tauri::command]
pub async fn create_design_project_cmd(
    input: CreateProjectInput,
) -> Result<DesignProject, CmdError> {
    service::create_project(input).map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_project_cmd(id: String) -> Result<Option<DesignProject>, CmdError> {
    service::get_project(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn update_design_project_cmd(
    input: UpdateProjectInput,
) -> Result<DesignProject, CmdError> {
    service::update_project(input).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_project_cmd(id: String) -> Result<(), CmdError> {
    service::delete_project(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn duplicate_design_project_cmd(id: String) -> Result<DesignProject, CmdError> {
    service::duplicate_project(&id).map_err(Into::into)
}

// ── Artifacts ───────────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_artifacts_cmd(
    project_id: String,
) -> Result<Vec<DesignArtifact>, CmdError> {
    service::list_artifacts(&project_id).map_err(Into::into)
}

#[tauri::command]
pub async fn create_design_artifact_cmd(
    input: CreateArtifactInput,
) -> Result<DesignArtifact, CmdError> {
    service::create_artifact_generating(input)
        .await
        .map_err(Into::into)
}

/// 「一句话 → 流式生成」：建 generating 壳同步返回，内容经 `design:generate_delta` 流式回填。
/// image / 无 brief / 未知 kind 自动回落阻塞生成。
#[tauri::command]
pub async fn generate_design_artifact_cmd(
    input: CreateArtifactInput,
) -> Result<DesignArtifact, CmdError> {
    service::generate_design_artifact(input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_all_design_artifacts_cmd() -> Result<Vec<DesignArtifact>, CmdError> {
    service::list_all_artifacts().map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_artifact_cmd(id: String) -> Result<Option<ArtifactView>, CmdError> {
    service::get_artifact_view(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_artifact_cmd(id: String) -> Result<(), CmdError> {
    service::delete_artifact(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn list_design_artifact_versions_cmd(
    id: String,
) -> Result<Vec<DesignArtifactVersion>, CmdError> {
    service::list_versions(&id).map_err(Into::into)
}

/// 某历史版本快照的 index.html（历史面板右栏 iframe srcdoc 预览用）。
#[tauri::command]
pub async fn get_design_artifact_version_html_cmd(
    artifact_id: String,
    version_number: i64,
) -> Result<String, CmdError> {
    service::get_artifact_version_html(&artifact_id, version_number).map_err(Into::into)
}

#[tauri::command]
pub async fn patch_design_element_cmd(input: ElementPatch) -> Result<DesignArtifact, CmdError> {
    service::patch_element(input).map_err(Into::into)
}

#[tauri::command]
pub async fn export_design_artifact_cmd(
    id: String,
    format: Option<String>,
) -> Result<ExportResult, CmdError> {
    service::export_artifact(&id, format.as_deref().unwrap_or("html")).map_err(Into::into)
}

#[tauri::command]
pub async fn critique_design_artifact_cmd(id: String) -> Result<CritiqueResult, CmdError> {
    service::critique_artifact(&id).await.map_err(Into::into)
}

/// 就地换设计系统（restyle）：改产物设计系统 + 用新 token 重渲染 + 落新版本。owner 平面。
#[tauri::command]
pub async fn restyle_design_artifact_cmd(
    id: String,
    system_id: Option<String>,
) -> Result<DesignArtifact, CmdError> {
    service::restyle_artifact(&id, system_id.as_deref()).map_err(Into::into)
}

/// 导出代码交付包（handoff ZIP，content 为 base64）。owner 平面。
#[tauri::command]
pub async fn export_design_handoff_cmd(id: String) -> Result<ExportResult, CmdError> {
    service::export_handoff(&id).map_err(Into::into)
}

// ── Code bindings (工程轴 D) ────────────────────────────────────

/// 绑定设计系统到代码工程目录（owner 平面）。
#[tauri::command]
pub async fn bind_design_code_project_cmd(
    system_id: String,
    target_dir: String,
    subfolder: Option<String>,
    formats: Option<Vec<String>>,
) -> Result<DesignCodeBinding, CmdError> {
    service::bind_code_project(
        &system_id,
        &target_dir,
        subfolder.as_deref().unwrap_or(""),
        &formats.unwrap_or_default(),
    )
    .map_err(Into::into)
}

/// 同步：把绑定系统的多平台 token 写入代码工程目录（owner 平面）。
#[tauri::command]
pub async fn sync_design_code_binding_cmd(id: i64) -> Result<BindingSyncReport, CmdError> {
    service::sync_code_binding(id).map_err(Into::into)
}

/// 列出代码绑定（可按 system 过滤）。owner 平面。
#[tauri::command]
pub async fn list_design_code_bindings_cmd(
    system_id: Option<String>,
) -> Result<Vec<DesignCodeBinding>, CmdError> {
    service::list_code_bindings(system_id.as_deref()).map_err(Into::into)
}

/// 解绑（删记录，不删已写文件）。owner 平面。
#[tauri::command]
pub async fn unbind_design_code_project_cmd(id: i64) -> Result<(), CmdError> {
    service::unbind_code_project(id).map_err(Into::into)
}

#[tauri::command]
pub async fn restore_design_version_cmd(
    artifact_id: String,
    version_id: i64,
) -> Result<DesignArtifact, CmdError> {
    service::restore_version(&artifact_id, version_id).map_err(Into::into)
}

/// 导出强路依赖预检：ffmpeg（MP4 编码器）三态状态。导出面板在走 MP4 强路前调它。
#[tauri::command]
pub async fn design_ffmpeg_doctor_cmd() -> Result<ha_core::ffmpeg::FfmpegStatus, CmdError> {
    Ok(ha_core::ffmpeg::doctor().await)
}

/// 导出强路依赖预检：浏览器引擎（PDF/PNG 矢量/全保真捕获）三态状态。
#[tauri::command]
pub async fn design_browser_doctor_cmd(
) -> Result<ha_core::design::render_native::BrowserExportStatus, CmdError> {
    Ok(ha_core::design::render_native::browser_export_status())
}

/// 按需下载 Chromium runtime（PDF/PNG 强路引擎）。进度经 `browser:chromium_download_progress`。
#[tauri::command]
pub async fn design_install_browser_cmd() -> Result<FfmpegRuntimeResult, CmdError> {
    let binary = ha_core::browser::runtime::install_with_event_bus_progress().await?;
    Ok(FfmpegRuntimeResult {
        binary_path: binary.display().to_string(),
    })
}

/// 按需下载 + 解包静态 ffmpeg（MP4 强路编码器）。幂等；进度经
/// `design:ffmpeg_download_progress` 事件推给导出面板渲染进度条。
#[tauri::command]
pub async fn design_install_ffmpeg_cmd() -> Result<FfmpegRuntimeResult, CmdError> {
    let binary = ha_core::ffmpeg::install_with_event_bus_progress().await?;
    Ok(FfmpegRuntimeResult {
        binary_path: binary.display().to_string(),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegRuntimeResult {
    pub binary_path: String,
}

/// PPTX：前端栅格化的整页 PNG（base64）→ 后端组装 → 返回 `{ pptx: base64 }`。
/// 形状与 HTTP `POST /api/design/pptx` 一致，前端两模式统一读 `res.pptx`。
#[tauri::command]
pub async fn export_design_pptx_cmd(
    slides: Vec<String>,
    title: Option<String>,
) -> Result<serde_json::Value, CmdError> {
    let pptx = service::export_pptx(&slides, title.as_deref().unwrap_or("design"))?;
    Ok(serde_json::json!({ "pptx": pptx }))
}

/// ZIP：`artifactId` = 单产物源码包；`projectId` = 项目级全产物包 → `{ zip: base64 }`。
#[tauri::command]
pub async fn export_design_zip_cmd(
    artifact_id: Option<String>,
    project_id: Option<String>,
) -> Result<serde_json::Value, CmdError> {
    let zip = service::export_zip(artifact_id.as_deref(), project_id.as_deref())?;
    Ok(serde_json::json!({ "zip": zip }))
}

// ── Design systems ──────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_systems_cmd() -> Result<Vec<DesignSystemMeta>, CmdError> {
    service::list_systems().map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_system_cmd(id: String) -> Result<DesignSystemFull, CmdError> {
    service::get_system_full(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn save_design_system_cmd(input: SaveSystemInput) -> Result<DesignSystemMeta, CmdError> {
    service::save_system(input).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_system_cmd(id: String) -> Result<(), CmdError> {
    service::delete_system(&id).map_err(Into::into)
}

/// 反向提取设计系统（brief / codebase / url / image）。owner 平面。
#[tauri::command]
pub async fn extract_design_system_cmd(
    input: ExtractSystemInput,
) -> Result<DesignSystemMeta, CmdError> {
    service::extract_system(input).await.map_err(Into::into)
}

/// 导入一份 DESIGN.md 文本为设计系统（互通格式）。owner 平面。
#[tauri::command]
pub async fn import_design_md_cmd(name: String, md: String) -> Result<DesignSystemMeta, CmdError> {
    service::import_design_md(&name, &md)
        .await
        .map_err(Into::into)
}

/// 从 Figma 文件导入设计系统（owner 平面专属；token 按次传、不落盘）。
#[tauri::command]
pub async fn import_figma_system_cmd(
    url: String,
    token: String,
    name: Option<String>,
) -> Result<DesignSystemMeta, CmdError> {
    service::import_figma(&url, &token, name.as_deref())
        .await
        .map_err(Into::into)
}

/// 导出一个设计系统为规范 DESIGN.md 文本 → `{ designMd }`。owner 平面。
#[tauri::command]
pub async fn export_design_md_cmd(system_id: String) -> Result<serde_json::Value, CmdError> {
    let md = service::export_design_md(&system_id)?;
    Ok(serde_json::json!({ "designMd": md }))
}

/// 导出设计系统 Token 为多平台开发者格式（CSS/SCSS/TS/Swift/Android/DTCG）。owner 平面。
#[tauri::command]
pub async fn export_design_tokens_cmd(system_id: String) -> Result<Vec<TokenExport>, CmdError> {
    Ok(service::export_tokens(&system_id)?)
}

/// 设计方向候选（无品牌 brief 时的选择器）。
#[tauri::command]
pub async fn propose_design_directions_cmd(
    brief: String,
    count: Option<usize>,
) -> Result<Vec<Direction>, CmdError> {
    service::propose_directions(&brief, count.unwrap_or(4))
        .await
        .map_err(Into::into)
}

// ── Config ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_design_config_cmd() -> Result<DesignConfig, CmdError> {
    Ok(ha_core::config::cached_config().design.clone())
}

#[tauri::command]
pub async fn save_design_config_cmd(config: DesignConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("design", "tauri"), |store| {
        store.design = config.clone();
        Ok(())
    })?;
    Ok(())
}

// ── Recipes（设计模板目录，供 GUI 首屏模板快选）─────────────────────

#[tauri::command]
pub async fn list_design_recipes_cmd() -> Result<Vec<ha_core::design::recipe::Recipe>, CmdError> {
    Ok(ha_core::design::recipe::builtin_recipes())
}

/// 强路导出：真实浏览器原生捕获（PDF 矢量可选文字 / PNG 全保真）→ `{ data: base64, mime }`。
/// 复用现有 CDP 后端（Chromium 按需下载、不打包）；后端不可用时返回 Err，前端回退客户端
/// 栅格化（html2canvas / jsPDF）。owner 平面。
#[tauri::command]
pub async fn export_design_native_cmd(
    id: String,
    format: String,
) -> Result<serde_json::Value, CmdError> {
    let (data, mime) = ha_core::design::render_native::capture_artifact_b64(&id, &format).await?;
    Ok(serde_json::json!({ "data": data, "mime": mime }))
}

// ── Comments (批注钉) ────────────────────────────────────────────

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn design_comment_add_cmd(
    artifact_id: String,
    oid: Option<i64>,
    rel_x: f64,
    rel_y: f64,
    tag: Option<String>,
    snippet: Option<String>,
    body: String,
) -> Result<DesignComment, CmdError> {
    service::add_comment(
        &artifact_id,
        oid,
        rel_x,
        rel_y,
        tag.as_deref(),
        snippet.as_deref(),
        &body,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_list_cmd(artifact_id: String) -> Result<Vec<DesignComment>, CmdError> {
    service::list_comments(&artifact_id).map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_relocate_cmd(
    artifact_id: String,
    comment_id: i64,
    oid: Option<i64>,
    rel_x: f64,
    rel_y: f64,
) -> Result<bool, CmdError> {
    service::relocate_comment(&artifact_id, comment_id, oid, rel_x, rel_y).map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_update_cmd(
    artifact_id: String,
    comment_id: i64,
    body: String,
) -> Result<bool, CmdError> {
    service::update_comment_body(&artifact_id, comment_id, &body).map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_resolve_cmd(
    artifact_id: String,
    comment_id: i64,
    resolved: bool,
) -> Result<bool, CmdError> {
    service::set_comment_resolved(&artifact_id, comment_id, resolved).map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_delete_cmd(
    artifact_id: String,
    comment_id: i64,
) -> Result<bool, CmdError> {
    service::delete_comment(&artifact_id, comment_id).map_err(Into::into)
}

/// 回灌对话：让 AI 按批注精修产物（产物就地更新新版本）。
#[tauri::command]
pub async fn design_comment_refine_cmd(
    artifact_id: String,
    comment_id: i64,
) -> Result<DesignArtifact, CmdError> {
    service::refine_artifact_with_comment(&artifact_id, comment_id)
        .await
        .map_err(Into::into)
}

/// 设计系统套件视图自包含 HTML（前端进沙箱 iframe 渲染）。
#[tauri::command]
pub async fn get_design_system_kit_cmd(id: String) -> Result<String, CmdError> {
    service::get_system_kit_html(&id).map_err(Into::into)
}

/// 反-slop 自查复查：`action ∈ recheck|dismiss`，返回更新后的产物。
#[tauri::command]
pub async fn design_review_artifact_cmd(
    artifact_id: String,
    action: String,
) -> Result<DesignArtifact, CmdError> {
    service::review_artifact(&artifact_id, &action).map_err(Into::into)
}

// ── Design-space per-project chat threads ───────────────────────

/// Default-load target: the most recent chat thread anchored to `projectId`.
/// `None` when the project has no prior conversation (panel shows empty state).
#[tauri::command]
pub async fn design_chat_thread_get_cmd(
    project_id: String,
) -> Result<Option<SessionMeta>, CmdError> {
    service::design_chat_thread_latest(&project_id).map_err(Into::into)
}

/// History picker: a page of chat threads in a design project, newest-active
/// first. `query` FTS-filters by message content when non-empty; `limit`/`offset`
/// paginate.
#[tauri::command]
pub async fn design_chat_threads_list_cmd(
    project_id: String,
    query: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DesignChatThread>, CmdError> {
    service::design_chat_threads_list(&project_id, query.as_deref(), limit, offset)
        .map_err(Into::into)
}
