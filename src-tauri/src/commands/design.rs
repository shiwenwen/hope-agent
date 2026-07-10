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
    ha_core::blocking::run_blocking(service::list_projects)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_design_project_cmd(
    input: CreateProjectInput,
) -> Result<DesignProject, CmdError> {
    ha_core::blocking::run_blocking(move || service::create_project(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_project_cmd(id: String) -> Result<Option<DesignProject>, CmdError> {
    ha_core::blocking::run_blocking(move || service::get_project(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_design_project_cmd(
    input: UpdateProjectInput,
) -> Result<DesignProject, CmdError> {
    ha_core::blocking::run_blocking(move || service::update_project(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_project_cmd(id: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::delete_project(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn duplicate_design_project_cmd(id: String) -> Result<DesignProject, CmdError> {
    ha_core::blocking::run_blocking(move || service::duplicate_project(&id))
        .await
        .map_err(Into::into)
}

// ── Artifacts ───────────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_artifacts_cmd(
    project_id: String,
) -> Result<Vec<DesignArtifact>, CmdError> {
    ha_core::blocking::run_blocking(move || service::list_artifacts(&project_id))
        .await
        .map_err(Into::into)
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
    ha_core::blocking::run_blocking(service::list_all_artifacts)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_artifact_cmd(id: String) -> Result<Option<ArtifactView>, CmdError> {
    ha_core::blocking::run_blocking(move || service::get_artifact_view(&id))
        .await
        .map_err(Into::into)
}

/// 打开产物时自愈渲染版本（inspector bridge 等工具层升级对老产物生效）。返回是否重渲染。
#[tauri::command]
pub async fn ensure_design_artifact_fresh_cmd(id: String) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || service::ensure_artifact_render_fresh(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_artifact_cmd(id: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::delete_artifact(&id))
        .await
        .map_err(Into::into)
}

/// 轻量改名产物（仅 title）。
#[tauri::command]
pub async fn rename_design_artifact_cmd(
    id: String,
    title: String,
) -> Result<DesignArtifact, CmdError> {
    ha_core::blocking::run_blocking(move || service::rename_artifact(&id, &title))
        .await
        .map_err(Into::into)
}

/// 复制产物（同项目内，深拷贝，标题加「(副本)」）。
#[tauri::command]
pub async fn duplicate_design_artifact_cmd(id: String) -> Result<DesignArtifact, CmdError> {
    ha_core::blocking::run_blocking(move || service::duplicate_artifact(&id))
        .await
        .map_err(Into::into)
}

/// 重排项目内产物页面顺序（拖动）。
#[tauri::command]
pub async fn reorder_design_artifacts_cmd(
    project_id: String,
    ordered_ids: Vec<String>,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::reorder_artifacts(&project_id, &ordered_ids))
        .await
        .map_err(Into::into)
}

// ── 页面分组文件夹 ──
#[tauri::command]
pub async fn list_design_folders_cmd(project_id: String) -> Result<Vec<String>, CmdError> {
    ha_core::blocking::run_blocking(move || service::list_folders(&project_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn create_design_folder_cmd(project_id: String, name: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::create_folder(&project_id, &name))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_folder_cmd(project_id: String, path: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::delete_folder(&project_id, &path))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rename_design_folder_cmd(
    project_id: String,
    from: String,
    to: String,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::rename_folder(&project_id, &from, &to))
        .await
        .map_err(Into::into)
}

/// 把页面移到某文件夹（folder 空 = 根）。
#[tauri::command]
pub async fn move_design_artifact_cmd(
    id: String,
    folder: String,
) -> Result<DesignArtifact, CmdError> {
    ha_core::blocking::run_blocking(move || service::move_artifact_to_folder(&id, &folder))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_design_artifact_versions_cmd(
    id: String,
) -> Result<Vec<DesignArtifactVersion>, CmdError> {
    ha_core::blocking::run_blocking(move || service::list_versions(&id))
        .await
        .map_err(Into::into)
}

/// 某历史版本快照的 index.html（历史面板右栏 iframe srcdoc 预览用）。
#[tauri::command]
pub async fn get_design_artifact_version_html_cmd(
    artifact_id: String,
    version_number: i64,
) -> Result<String, CmdError> {
    ha_core::blocking::run_blocking(move || {
        service::get_artifact_version_html(&artifact_id, version_number)
    })
    .await
    .map_err(Into::into)
}

// ── Shares（B7-1 只读分享，owner 平面）────────────────────────────

/// 建/取产物只读分享 token（幂等）。
#[tauri::command]
pub async fn create_design_share_cmd(artifact_id: String) -> Result<String, CmdError> {
    ha_core::blocking::run_blocking(move || service::create_share(&artifact_id))
        .await
        .map_err(Into::into)
}

/// 取产物当前分享 token（无则 None）。
#[tauri::command]
pub async fn get_design_share_cmd(artifact_id: String) -> Result<Option<String>, CmdError> {
    ha_core::blocking::run_blocking(move || service::share_token_for_artifact(&artifact_id))
        .await
        .map_err(Into::into)
}

/// 撤销产物分享。
#[tauri::command]
pub async fn revoke_design_share_cmd(artifact_id: String) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || service::revoke_share_for_artifact(&artifact_id))
        .await
        .map_err(Into::into)
}

// ── Cloudflare Pages 部署（B7-2，owner 平面 opt-in）─────────────────

/// 保存 CF 部署配置（token 0600 落 credentials；token=mask 保留原值）。
#[tauri::command]
pub async fn save_cf_deploy_config_cmd(
    api_token: String,
    account_id: String,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::design::deploy::save_cf_config(&api_token, &account_id)
    })
    .await
    .map_err(Into::into)
}

/// 读 CF 部署配置（**token 脱敏**：只回 hasToken + mask 哨兵）。
#[tauri::command]
pub async fn get_cf_deploy_config_cmd() -> Result<ha_core::design::deploy::CfConfigPublic, CmdError>
{
    ha_core::blocking::run_blocking(ha_core::design::deploy::public_cf_config)
        .await
        .map_err(Into::into)
}

/// 部署产物到 CF Pages，返回 `{ url }`（与 HTTP `POST /deploy` 同形，前端统一读 `res.url`）。
#[derive(serde::Serialize)]
pub struct DeployUrl {
    pub url: String,
}
#[tauri::command]
pub async fn deploy_design_artifact_cmd(artifact_id: String) -> Result<DeployUrl, CmdError> {
    let url = ha_core::design::deploy::deploy_artifact(&artifact_id).await?;
    Ok(DeployUrl { url })
}
#[tauri::command]
pub async fn bind_design_domain_cmd(
    artifact_id: String,
    domain: String,
) -> Result<ha_core::design::deploy::CustomDomain, CmdError> {
    ha_core::design::deploy::bind_custom_domain(&artifact_id, &domain)
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn list_design_domains_cmd(
    artifact_id: String,
) -> Result<Vec<ha_core::design::deploy::CustomDomain>, CmdError> {
    ha_core::design::deploy::list_custom_domains(&artifact_id)
        .await
        .map_err(Into::into)
}

// ── Vercel 部署（多提供商第二 provider，owner 平面 opt-in）─────────────────

/// 保存 Vercel 部署配置（token 0600 落 credentials；token=mask 保留原值）。
#[tauri::command]
pub async fn save_vercel_deploy_config_cmd(
    api_token: String,
    team_id: String,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        ha_core::design::deploy_vercel::save_vercel_config(&api_token, &team_id)
    })
    .await
    .map_err(Into::into)
}

/// 读 Vercel 部署配置（**token 脱敏**：只回 hasToken + mask 哨兵）。
#[tauri::command]
pub async fn get_vercel_deploy_config_cmd(
) -> Result<ha_core::design::deploy_vercel::VercelConfigPublic, CmdError> {
    ha_core::blocking::run_blocking(ha_core::design::deploy_vercel::public_vercel_config)
        .await
        .map_err(Into::into)
}

/// 部署产物到 Vercel，返回 `{ url }`（与 CF 同形，前端统一读 `res.url`）。
#[tauri::command]
pub async fn deploy_design_artifact_vercel_cmd(
    artifact_id: String,
) -> Result<DeployUrl, CmdError> {
    let url = ha_core::design::deploy_vercel::deploy_artifact(&artifact_id).await?;
    Ok(DeployUrl { url })
}

#[tauri::command]
pub async fn patch_design_element_cmd(input: ElementPatch) -> Result<DesignArtifact, CmdError> {
    ha_core::blocking::run_blocking(move || service::patch_element(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn export_design_artifact_cmd(
    id: String,
    format: Option<String>,
) -> Result<ExportResult, CmdError> {
    ha_core::blocking::run_blocking(move || {
        service::export_artifact(&id, format.as_deref().unwrap_or("html"))
    })
    .await
    .map_err(Into::into)
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
    ha_core::blocking::run_blocking(move || service::restyle_artifact(&id, system_id.as_deref()))
        .await
        .map_err(Into::into)
}

/// 导出代码交付包（handoff ZIP，content 为 base64）。owner 平面。
#[tauri::command]
pub async fn export_design_handoff_cmd(id: String) -> Result<ExportResult, CmdError> {
    ha_core::blocking::run_blocking(move || service::export_handoff(&id))
        .await
        .map_err(Into::into)
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
    ha_core::blocking::run_blocking(move || {
        service::bind_code_project(
            &system_id,
            &target_dir,
            subfolder.as_deref().unwrap_or(""),
            &formats.unwrap_or_default(),
        )
    })
    .await
    .map_err(Into::into)
}

/// 同步：把绑定系统的多平台 token 写入代码工程目录（owner 平面）。
#[tauri::command]
pub async fn sync_design_code_binding_cmd(id: i64) -> Result<BindingSyncReport, CmdError> {
    ha_core::blocking::run_blocking(move || service::sync_code_binding(id))
        .await
        .map_err(Into::into)
}

/// 列出代码绑定（可按 system 过滤）。owner 平面。
#[tauri::command]
pub async fn list_design_code_bindings_cmd(
    system_id: Option<String>,
) -> Result<Vec<DesignCodeBinding>, CmdError> {
    ha_core::blocking::run_blocking(move || service::list_code_bindings(system_id.as_deref()))
        .await
        .map_err(Into::into)
}

/// 解绑（删记录，不删已写文件）。owner 平面。
#[tauri::command]
pub async fn unbind_design_code_project_cmd(id: i64) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::unbind_code_project(id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn restore_design_version_cmd(
    artifact_id: String,
    version_id: i64,
) -> Result<DesignArtifact, CmdError> {
    ha_core::blocking::run_blocking(move || service::restore_version(&artifact_id, version_id))
        .await
        .map_err(Into::into)
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
    let pptx = ha_core::blocking::run_blocking(move || {
        service::export_pptx(&slides, title.as_deref().unwrap_or("design"))
    })
    .await?;
    Ok(serde_json::json!({ "pptx": pptx }))
}

/// ZIP：`artifactId` = 单产物源码包；`projectId` = 项目级全产物包 → `{ zip: base64 }`。
#[tauri::command]
pub async fn export_design_zip_cmd(
    artifact_id: Option<String>,
    project_id: Option<String>,
) -> Result<serde_json::Value, CmdError> {
    let zip = ha_core::blocking::run_blocking(move || {
        service::export_zip(artifact_id.as_deref(), project_id.as_deref())
    })
    .await?;
    Ok(serde_json::json!({ "zip": zip }))
}

/// 批量导出选中产物为一个 ZIP（每产物一目录 + 画廊，Wave 1-③）→ `{ zip: base64 }`。
#[tauri::command]
pub async fn export_design_selected_zip_cmd(
    artifact_ids: Vec<String>,
) -> Result<serde_json::Value, CmdError> {
    let zip = ha_core::blocking::run_blocking(move || service::export_selected_zip(&artifact_ids))
        .await?;
    Ok(serde_json::json!({ "zip": zip }))
}

// ── Design systems ──────────────────────────────────────────────

#[tauri::command]
pub async fn list_design_systems_cmd() -> Result<Vec<DesignSystemMeta>, CmdError> {
    ha_core::blocking::run_blocking(service::list_systems)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_design_system_cmd(id: String) -> Result<DesignSystemFull, CmdError> {
    ha_core::blocking::run_blocking(move || service::get_system_full(&id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_design_system_cmd(input: SaveSystemInput) -> Result<DesignSystemMeta, CmdError> {
    ha_core::blocking::run_blocking(move || service::save_system(input))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_design_system_cmd(id: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || service::delete_system(&id))
        .await
        .map_err(Into::into)
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
    let md = ha_core::blocking::run_blocking(move || service::export_design_md(&system_id)).await?;
    Ok(serde_json::json!({ "designMd": md }))
}

/// 导出设计系统 Token 为多平台开发者格式（CSS/SCSS/TS/Swift/Android/DTCG）。owner 平面。
#[tauri::command]
pub async fn export_design_tokens_cmd(system_id: String) -> Result<Vec<TokenExport>, CmdError> {
    Ok(ha_core::blocking::run_blocking(move || service::export_tokens(&system_id)).await?)
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
    ha_core::blocking::run_blocking(move || {
        service::add_comment(
            &artifact_id,
            oid,
            rel_x,
            rel_y,
            tag.as_deref(),
            snippet.as_deref(),
            &body,
        )
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_list_cmd(artifact_id: String) -> Result<Vec<DesignComment>, CmdError> {
    ha_core::blocking::run_blocking(move || service::list_comments(&artifact_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_relocate_cmd(
    artifact_id: String,
    comment_id: i64,
    oid: Option<i64>,
    rel_x: f64,
    rel_y: f64,
) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || {
        service::relocate_comment(&artifact_id, comment_id, oid, rel_x, rel_y)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_update_cmd(
    artifact_id: String,
    comment_id: i64,
    body: String,
) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || {
        service::update_comment_body(&artifact_id, comment_id, &body)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_resolve_cmd(
    artifact_id: String,
    comment_id: i64,
    resolved: bool,
) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || {
        service::set_comment_resolved(&artifact_id, comment_id, resolved)
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn design_comment_delete_cmd(
    artifact_id: String,
    comment_id: i64,
) -> Result<bool, CmdError> {
    ha_core::blocking::run_blocking(move || service::delete_comment(&artifact_id, comment_id))
        .await
        .map_err(Into::into)
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
    ha_core::blocking::run_blocking(move || service::get_system_kit_html(&id))
        .await
        .map_err(Into::into)
}

/// 反-slop 自查复查：`action ∈ recheck|dismiss`，返回更新后的产物。
#[tauri::command]
pub async fn design_review_artifact_cmd(
    artifact_id: String,
    action: String,
) -> Result<DesignArtifact, CmdError> {
    ha_core::blocking::run_blocking(move || service::review_artifact(&artifact_id, &action))
        .await
        .map_err(Into::into)
}

// ── Design-space per-project chat threads ───────────────────────

/// Default-load target: the most recent chat thread anchored to `projectId`.
/// `None` when the project has no prior conversation (panel shows empty state).
#[tauri::command]
pub async fn design_chat_thread_get_cmd(
    project_id: String,
) -> Result<Option<SessionMeta>, CmdError> {
    ha_core::blocking::run_blocking(move || service::design_chat_thread_latest(&project_id))
        .await
        .map_err(Into::into)
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
    ha_core::blocking::run_blocking(move || {
        service::design_chat_threads_list(&project_id, query.as_deref(), limit, offset)
    })
    .await
    .map_err(Into::into)
}
