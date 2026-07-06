//! Tauri commands for the Design Space feature.
//!
//! Thin wrappers around `ha_core::design` — all logic lives in ha-core. These
//! run on the **owner plane** (desktop = trusted local machine): the operator
//! sees all their design projects/artifacts, not gated by any agent access
//! check (that is for the agent `design` tool).

use crate::commands::CmdError;
use ha_core::design::extract::Direction;
use ha_core::design::service::{
    self, ArtifactView, CreateArtifactInput, CreateProjectInput, ElementPatch, ExportResult,
    ExtractSystemInput, SaveSystemInput, UpdateProjectInput,
};
use ha_core::design::{
    CritiqueResult, DesignArtifact, DesignArtifactVersion, DesignConfig, DesignProject,
    DesignSystemFull, DesignSystemMeta,
};

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

/// 导出一个设计系统为规范 DESIGN.md 文本 → `{ designMd }`。owner 平面。
#[tauri::command]
pub async fn export_design_md_cmd(system_id: String) -> Result<serde_json::Value, CmdError> {
    let md = service::export_design_md(&system_id)?;
    Ok(serde_json::json!({ "designMd": md }))
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
