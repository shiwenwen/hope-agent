//! 设计空间 owner 平面业务入口（Tauri / HTTP 薄壳统一调用）。
//!
//! owner 平面 = 本机 / API key 信任，负责 UI 的项目/产物 CRUD、可视化编辑回写、
//! 导出——**不经 agent 访问检查**（见 `docs/architecture/design-space.md` §3）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::db::{
    DesignArtifact, DesignArtifactVersion, DesignComment, DesignDb, DesignProject, DesignSystemMeta,
};
use super::patch;
use super::renderer::{self, ArtifactKind, ArtifactParts};
use super::system::{self, DesignSystemFull};
use crate::paths;
use crate::platform::write_atomic;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 打开（懒建目录）设计库连接。
pub fn open_db() -> Result<DesignDb> {
    let db_path = paths::design_db_path()?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    DesignDb::open(&db_path)
}

fn emit(event: &str, payload: serde_json::Value) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(event, payload);
    }
}

/// 解析设计系统的 CSS 变量 token（注入产物 `:root`）。
///
/// Phase 3 返回空（内置设计系统在 Phase 2 落地，届时读 `systems/{id}/tokens.json`）。
fn resolve_tokens(system_id: Option<&str>) -> Vec<(String, String)> {
    let Some(id) = system_id else {
        return Vec::new();
    };
    let Ok(dir) = paths::design_system_dir(id) else {
        return Vec::new();
    };
    let path = dir.join("tokens.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(map) = serde_json::from_str::<std::collections::BTreeMap<String, String>>(&raw) else {
        return Vec::new();
    };
    map.into_iter().collect()
}

/// 把当前 index.html + source + oidmap 快照进 `versions/{n}/`。
fn write_version_snapshot(
    dir: &std::path::Path,
    n: i64,
    html: &str,
    parts: &ArtifactParts,
    oidmap_json: &str,
) -> Result<()> {
    let vdir = dir.join("versions").join(n.to_string());
    std::fs::create_dir_all(vdir.join("source"))?;
    write_atomic(&vdir.join("index.html"), html.as_bytes())?;
    write_atomic(
        &vdir.join("source").join("body.html"),
        parts.body_html.as_bytes(),
    )?;
    write_atomic(&vdir.join("source").join("style.css"), parts.css.as_bytes())?;
    write_atomic(&vdir.join("source").join("script.js"), parts.js.as_bytes())?;
    write_atomic(&vdir.join("oidmap.json"), oidmap_json.as_bytes())?;
    Ok(())
}

/// 读取产物当前源（工作副本）。**读失败即上抛**（区分「文件不存在=合法空」与
/// 「读错误=不可静默降级为空」），否则 `update_artifact` 会拿空正文覆盖 + 永久快照，
/// 一次改标题就把产物抹了。
fn read_source(dir: &std::path::Path) -> Result<ArtifactParts> {
    let read = |name: &str| -> Result<String> {
        match std::fs::read_to_string(dir.join("source").join(name)) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(anyhow::anyhow!("read source/{name}: {e}")),
        }
    };
    Ok(ArtifactParts {
        body_html: read("body.html")?,
        css: read("style.css")?,
        js: read("script.js")?,
    })
}

/// 写产物工作副本源 + 渲染 index.html + oidmap。
fn write_working(
    dir: &std::path::Path,
    html: &str,
    parts: &ArtifactParts,
    oidmap_json: &str,
) -> Result<()> {
    std::fs::create_dir_all(dir.join("source"))?;
    write_atomic(&dir.join("index.html"), html.as_bytes())?;
    write_atomic(
        &dir.join("source").join("body.html"),
        parts.body_html.as_bytes(),
    )?;
    write_atomic(&dir.join("source").join("style.css"), parts.css.as_bytes())?;
    write_atomic(&dir.join("source").join("script.js"), parts.js.as_bytes())?;
    write_atomic(&dir.join("oidmap.json"), oidmap_json.as_bytes())?;
    Ok(())
}

/// 渲染 + 序列化 oidmap（create/update 共用）。
fn render(
    kind: ArtifactKind,
    title: &str,
    parts: &ArtifactParts,
    tokens: &[(String, String)],
) -> Result<(String, String)> {
    // Component：body_html 存 JSX 源，后端 oxc 编译成 JS 后内联 React runtime 组装。编译失败
    // 不 bail、渲染静态错误页（产物仍可开、可重生），故不阻断创建/定稿。无 oid（编译产物≠源码）。
    if kind == ArtifactKind::Component {
        let html = match super::compile::compile_component(&parts.body_html) {
            Ok(js) => renderer::build_component_html(title, &js, &parts.css, tokens),
            Err(e) => {
                crate::app_warn!("design", "compile", "component compile failed: {e}");
                renderer::build_component_error_html(title, &e.to_string())
            }
        };
        return Ok((html, "[]".to_string()));
    }
    // Image / Audio 是媒体产物（data-uri 内嵌），无源码 oid 可微调 → 不注 inspector/oid。
    let editable = !matches!(kind, ArtifactKind::Image | ArtifactKind::Audio);
    let (html, oidmap) = renderer::build_artifact_html(kind, title, parts, tokens, editable);
    let oidmap_json = serde_json::to_string(&oidmap)?;
    Ok((html, oidmap_json))
}

/// 渲染**干净可交付** HTML（`editable=false`，无 inspector/oid）。**Component 走 oxc 编译**（与
/// `render` 同分支），失败降级静态错误页——所有导出路径（artifact / zip / handoff）统一经此，
/// 保证导出的 `index.html` 与预览一致、可直接打开，绝不把未编译 JSX 塞进交付物。
fn render_clean(
    kind: ArtifactKind,
    title: &str,
    parts: &ArtifactParts,
    tokens: &[(String, String)],
) -> String {
    if kind == ArtifactKind::Component {
        return match super::compile::compile_component(&parts.body_html) {
            Ok(js) => renderer::build_component_html(title, &js, &parts.css, tokens),
            Err(e) => {
                crate::app_warn!("design", "compile", "component export compile failed: {e}");
                renderer::build_component_error_html(title, &e.to_string())
            }
        };
    }
    let (html, _) = renderer::build_artifact_html(kind, title, parts, tokens, false);
    html
}

/// 产物目录绝对路径（前端 iframe / 事件 payload 用）。
pub fn artifact_dir_str(project_id: &str, artifact_id: &str) -> String {
    paths::design_artifact_dir(project_id, artifact_id)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ── Projects ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub default_system_id: Option<String>,
    #[serde(default)]
    pub ha_project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

pub fn create_project(input: CreateProjectInput) -> Result<DesignProject> {
    let db = open_db()?;
    let ts = now();
    let title = if input.title.trim().is_empty() {
        "未命名项目".to_string()
    } else {
        input.title.trim().to_string()
    };
    let project = DesignProject {
        id: new_id(),
        title,
        description: input.description,
        color: input.color,
        default_system_id: input.default_system_id,
        ha_project_id: input.ha_project_id,
        session_id: input.session_id,
        agent_id: input.agent_id,
        created_at: ts.clone(),
        updated_at: ts,
        artifact_count: 0,
        metadata: None,
    };
    // 建项目目录 + project.json（真相源镜像）。
    let dir = paths::design_project_dir(&project.id)?;
    std::fs::create_dir_all(dir.join("artifacts"))?;
    write_atomic(
        &dir.join("project.json"),
        serde_json::to_string_pretty(&project)?.as_bytes(),
    )?;
    db.create_project(&project)?;
    crate::app_info!("design", "service", "create project {}", project.id);
    emit("design:project_changed", json!({ "projectId": project.id }));
    Ok(project)
}

pub fn list_projects() -> Result<Vec<DesignProject>> {
    open_db()?.list_projects()
}

/// agent 侧：解析当前会话的设计项目（取最近一个，无则新建草稿项目）。
pub fn get_or_create_session_project(
    session_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<DesignProject> {
    if let Some(sid) = session_id {
        let existing = open_db()?.list_projects_by_session(sid)?;
        if let Some(p) = existing.into_iter().next() {
            return Ok(p);
        }
    }
    create_project(CreateProjectInput {
        title: "设计草稿".to_string(),
        description: None,
        color: None,
        default_system_id: None,
        ha_project_id: None,
        session_id: session_id.map(str::to_string),
        agent_id: agent_id.map(str::to_string),
    })
}

pub fn get_project(id: &str) -> Result<Option<DesignProject>> {
    open_db()?.get_project(id)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectInput {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub default_system_id: Option<String>,
    #[serde(default)]
    pub ha_project_id: Option<String>,
}

pub fn update_project(input: UpdateProjectInput) -> Result<DesignProject> {
    let db = open_db()?;
    db.update_project(
        &input.id,
        input.title.as_deref(),
        input.description.as_deref(),
        input.color.as_deref(),
        input.default_system_id.as_deref(),
        input.ha_project_id.as_deref(),
        &now(),
    )?;
    let project = db
        .get_project(&input.id)?
        .context("project not found after update")?;
    // 回写 project.json。
    if let Ok(dir) = paths::design_project_dir(&project.id) {
        let _ = write_atomic(
            &dir.join("project.json"),
            serde_json::to_string_pretty(&project)?.as_bytes(),
        );
    }
    emit("design:project_changed", json!({ "projectId": project.id }));
    Ok(project)
}

/// 删除项目：DB 级联删产物/版本 + `rm -rf` 项目目录。
pub fn delete_project(id: &str) -> Result<()> {
    let db = open_db()?;
    db.delete_project(id)?;
    if let Ok(dir) = paths::design_project_dir(id) {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    crate::app_info!("design", "service", "delete project {}", id);
    emit("design:project_changed", json!({ "projectId": id }));
    Ok(())
}

// ── Artifacts ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateArtifactInput {
    pub project_id: String,
    pub title: String,
    /// web|mobile|deck|dashboard|poster|document|email|image|motion
    pub kind: String,
    #[serde(default)]
    pub system_id: Option<String>,
    /// 产物 body 结构 HTML（可选；空则生成占位）。
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub css: Option<String>,
    #[serde(default)]
    pub js: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// image 形态：图片描述 prompt（走 image_generate 生成并内嵌）。
    #[serde(default)]
    pub prompt: Option<String>,
    /// 参考图 base64（「照着这张图生成匹配产物」）：非媒体形态经 vision 描述成重建 brief
    /// 后走生成管线。与 `prompt` 可叠加（图 = 视觉参照，prompt = 额外要求）。
    #[serde(default)]
    pub reference_image_b64: Option<String>,
    #[serde(default)]
    pub reference_image_mime: Option<String>,
}

/// 若 image 形态且无 body，用 prompt/title 调 image_generate 生成后再落库。
/// owner（Tauri/HTTP）与 agent 工具共用此入口。
pub async fn create_artifact_generating(mut input: CreateArtifactInput) -> Result<DesignArtifact> {
    let body_empty = input.body_html.as_deref().unwrap_or("").trim().is_empty();
    if body_empty && input.kind == "image" {
        let prompt = input
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| input.title.clone());
        let parts = super::image::generate_image_parts(&prompt, &input.title).await?;
        input.body_html = Some(parts.body_html);
    } else if body_empty && input.kind == "audio" {
        // audio 形态：prompt → 音频合成（TTS/音乐/音效）→ 内嵌 data-uri <audio> 播放器。
        let prompt = input
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| input.title.clone());
        let parts = super::audio::generate_audio_parts(&prompt, &input.title).await?;
        input.body_html = Some(parts.body_html);
    } else if body_empty && input.kind == "component" {
        // component 形态：brief → 生成 React 组件源（JSX），render() 时后端 oxc 编译。
        // 生成失败降级为合法占位组件源（不阻断创建）。
        if let Some(brief) = input.prompt.clone().filter(|p| !p.trim().is_empty()) {
            let (system_md, tokens) = resolve_system_for_generation(&input);
            match super::generate::generate_component_source(&brief, &system_md, &tokens).await {
                Ok(src) => input.body_html = Some(src),
                Err(e) => {
                    crate::app_warn!(
                        "design",
                        "generate",
                        "component generation failed, blank shell: {e}"
                    );
                    input.body_html = Some(renderer::placeholder_component_source().to_string());
                }
            }
        } else {
            input.body_html = Some(renderer::placeholder_component_source().to_string());
        }
    } else if body_empty {
        // 非 image 形态：有 brief 时用一次模型生成完整自包含设计（GUI prompt→生成，对齐
        // 参照品类）。生成失败**不阻断**——降级为空壳产物（用户可在对话里继续细化）。
        if let (Some(kind), Some(brief)) = (
            ArtifactKind::from_str(&input.kind),
            input.prompt.clone().filter(|p| !p.trim().is_empty()),
        ) {
            let (system_md, tokens) = resolve_system_for_generation(&input);
            match super::generate::generate_design_parts(&brief, kind, &system_md, &tokens).await {
                Ok(parts) => {
                    input.body_html = Some(parts.body_html);
                    input.css = Some(parts.css);
                    input.js = Some(parts.js);
                }
                Err(e) => {
                    crate::app_warn!(
                        "design",
                        "generate",
                        "brief→design generation failed ({}), creating shell: {e}",
                        input.kind
                    );
                }
            }
        }
    }
    create_artifact(input)
}

/// 解析生成用的设计系统正文 + token（explicit > project default > config default）。
fn resolve_system_for_generation(
    input: &CreateArtifactInput,
) -> (String, std::collections::BTreeMap<String, String>) {
    let empty = || (String::new(), std::collections::BTreeMap::new());
    let Ok(db) = open_db() else {
        return empty();
    };
    let project_default = db
        .get_project(&input.project_id)
        .ok()
        .flatten()
        .and_then(|p| p.default_system_id);
    let system_id = input.system_id.clone().or(project_default).or_else(|| {
        crate::config::cached_config()
            .design
            .default_system_id
            .clone()
            .filter(|s| !s.trim().is_empty())
    });
    let Some(sid) = system_id else {
        return empty();
    };
    match system::read_full(&db, &sid) {
        Ok(full) => (full.system_md, full.tokens),
        Err(_) => empty(),
    }
}

/// `artifact.json` 磁盘元数据镜像。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactMeta {
    id: String,
    project_id: String,
    title: String,
    kind: String,
    system_id: Option<String>,
    current_version: i64,
}

/// 反 AI-slop 确定性自查：开启 `design.self_check` 时对产物正文跑无 LLM 检测，返回
/// `(status, metadata)`。命中翻 `needs_review` + 合并 `selfCheck` 键；未命中 / 关闭 →
/// `ready` + 清 `selfCheck` 键（回收自动标记，保留其它 metadata）。见 selfcheck.rs。
fn resolve_self_check(existing_meta: Option<&str>, body_html: &str) -> (String, Option<String>) {
    let enabled = crate::config::cached_config().design.self_check;
    let verdict = if enabled {
        super::selfcheck::evaluate(body_html)
    } else {
        None
    };
    if let Some(v) = &verdict {
        crate::app_info!(
            "design",
            "selfcheck",
            "artifact flagged needs_review: {} ({})",
            v.flag,
            v.detail
        );
    }
    let status = if verdict.is_some() {
        "needs_review"
    } else {
        "ready"
    };
    let metadata = super::selfcheck::merge_into_metadata(existing_meta, verdict.as_ref());
    (status.to_string(), metadata)
}

pub fn create_artifact(input: CreateArtifactInput) -> Result<DesignArtifact> {
    let db = open_db()?;
    let kind = ArtifactKind::from_str(&input.kind)
        .with_context(|| format!("unknown artifact kind: {}", input.kind))?;
    // 项目必须存在；产物设计系统缺省时继承项目默认。
    let project = db
        .get_project(&input.project_id)?
        .with_context(|| format!("project not found: {}", input.project_id))?;
    // System resolution: explicit > project default > global config default.
    let system_id = input
        .system_id
        .clone()
        .or(project.default_system_id.clone())
        .or_else(|| {
            crate::config::cached_config()
                .design
                .default_system_id
                .clone()
                .filter(|s| !s.trim().is_empty())
        });

    let ts = now();
    let artifact_id = new_id();
    let title = if input.title.trim().is_empty() {
        format!("未命名{}", kind.as_str())
    } else {
        input.title.trim().to_string()
    };

    // 空正文 = 起草占位模板（非 slop，不自查，避免误标 needs_review）；
    // 有正文（模型生成 / 用户提供）才跑确定性自查。
    let had_body = !input.body_html.as_deref().unwrap_or("").trim().is_empty();
    let parts = if !had_body {
        renderer::placeholder_parts(kind, &title)
    } else {
        ArtifactParts {
            body_html: input.body_html.unwrap_or_default(),
            css: input.css.unwrap_or_default(),
            js: input.js.unwrap_or_default(),
        }
    };
    let (status, self_check_meta) = if had_body {
        resolve_self_check(None, &parts.body_html)
    } else {
        ("ready".to_string(), None)
    };

    // 磁盘落地：artifact_dir / index.html / source/ / versions/1 / artifact.json
    let dir = paths::design_artifact_dir(&input.project_id, &artifact_id)?;
    let tokens = resolve_tokens(system_id.as_deref());
    let (html, oidmap_json) = render(kind, &title, &parts, &tokens)?;
    write_working(&dir, &html, &parts, &oidmap_json)?;
    write_version_snapshot(&dir, 1, &html, &parts, &oidmap_json)?;

    let (vw, vh) = kind.default_viewport();
    let artifact = DesignArtifact {
        id: artifact_id.clone(),
        project_id: input.project_id.clone(),
        title: title.clone(),
        kind: kind.as_str().to_string(),
        system_id: system_id.clone(),
        status,
        viewport_w: if vw > 0 { Some(vw) } else { None },
        viewport_h: if vh > 0 { Some(vh) } else { None },
        current_version: 1,
        critique_score: None,
        thumbnail_path: None,
        created_at: ts.clone(),
        updated_at: ts.clone(),
        metadata: self_check_meta,
    };
    let meta = ArtifactMeta {
        id: artifact.id.clone(),
        project_id: artifact.project_id.clone(),
        title: artifact.title.clone(),
        kind: artifact.kind.clone(),
        system_id: artifact.system_id.clone(),
        current_version: 1,
    };
    write_atomic(
        &dir.join("artifact.json"),
        serde_json::to_string_pretty(&meta)?.as_bytes(),
    )?;

    // Persist to the registry; if it fails, remove the just-written directory so we
    // don't leak an orphan artifact dir (DB row is the source of truth for listing).
    let persisted = (|| -> Result<()> {
        db.create_artifact(&artifact)?;
        db.create_version(&DesignArtifactVersion {
            id: 0,
            artifact_id: artifact_id.clone(),
            version_number: 1,
            message: Some("Initial version".to_string()),
            critique_score: None,
            created_at: ts.clone(),
        })?;
        db.touch_project(&input.project_id, &ts)?;
        Ok(())
    })();
    if let Err(e) = persisted {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }

    crate::app_info!(
        "design",
        "service",
        "create artifact {} kind={} project={}",
        artifact_id,
        kind.as_str(),
        input.project_id
    );
    emit(
        "design:artifact_ready",
        json!({
            "projectId": input.project_id,
            "artifactId": artifact_id,
            "sessionId": input.session_id,
        }),
    );
    Ok(artifact)
}

// ── 真流式生成（owner/GUI「一句话 → 流式生成」，见 design-space.md §11）────────────
//
// 数据流：owner 入口 `generate_design_artifact` 同步建 generating 壳（有样式的空 body
// 容器 + postMessage 接收脚本）立即返回 → 前端挂稳定 iframe → spawn `stream_generate_artifact`
// 走 `generate::stream_design_parts`（CSS-first 真流式）→ 逐帧 emit `design:generate_delta`
// → 前端 postMessage 增量灌进 iframe（无 FOUC）→ 定稿单次 render+落盘+status=ready+swap。
// 任何失败降级为 status=failed + 保留壳（对齐 `create_artifact_generating` 的降级空壳）。

/// 在途流式生成的协作取消旗（per-artifact）。regenerate 覆盖前翻旧旗，delete 时翻真。
fn generation_cancels(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, Arc<AtomicBool>>> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CANCELS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_generation_cancel(artifact_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut map = generation_cancels()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // 同产物 regenerate：翻旧旗（止其白流 + finalize 覆盖），装新旗。
    if let Some(old) = map.insert(artifact_id.to_string(), flag.clone()) {
        old.store(true, Ordering::SeqCst);
    }
    flag
}

fn clear_generation_cancel(artifact_id: &str, flag: &Arc<AtomicBool>) {
    let mut map = generation_cancels()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // 仅当仍是自己这面旗时移除（regenerate 已换新旗则不动，防误删后来者）。
    if map
        .get(artifact_id)
        .is_some_and(|cur| Arc::ptr_eq(cur, flag))
    {
        map.remove(artifact_id);
    }
}

/// delete 时取消该产物在途流式生成（止其白流 + finalize 写已删目录）。
fn cancel_generation(artifact_id: &str) {
    if let Some(flag) = generation_cancels()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(artifact_id)
    {
        flag.store(true, Ordering::SeqCst);
    }
}

/// 流式失败/崩溃降级：`artifact_lock` 下渲染**干净占位** index.html（不再是 spinner 壳）+ 置
/// status。让失败产物预览是可读占位而非永久转圈（对齐 `create_artifact_generating` 非流式降级
/// 产出可用占位）。
///
/// 返回 `Ok(true)` = 真降级了；`Ok(false)` = **未降级**（产物已删 → 不复活已删目录，守 #6；
/// 或已非 generating → 不 clobber 已 finalize 的 ready）。调用方据此决定是否 emit
/// `generate_error`——已删产物**不该**收到「生成失败」（与 finalize-None 静默契约对齐）。
fn degrade_to_placeholder(id: &str, status: &str) -> Result<bool> {
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let db = open_db()?;
    let Some(a) = db.get_artifact(id)? else {
        return Ok(false);
    };
    // 只降级仍在 generating 的产物——锁内重查守卫，防在 lock 等待期间产物已被别的路径
    // finalize 成 ready（reconcile / 晚到的失败回调）被误打回 failed 占位。
    if a.status != "generating" {
        return Ok(false);
    }
    let kind = ArtifactKind::from_str(&a.kind).unwrap_or(ArtifactKind::Web);
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = renderer::placeholder_parts(kind, &a.title);
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens)?;
    write_working(&dir, &html, &parts, &oidmap_json)?;
    db.update_artifact(id, None, Some(status), None, None, None, &now())?;
    Ok(true)
}

/// 崩溃/重启孤儿对账：进程本地 cancel 注册表里没有、status 仍 `generating`、且 `updated_at`
/// 陈旧（早于 grace，远超正常流式时长）的产物 = 上个进程流式到一半就挂了的孤儿——翻 `failed`
/// + 落干净占位。owner library-wall 加载时调用（design 无专用启动钩子），只命中陈旧孤儿故开销
/// 可忽略。**不用持久 replay 表**——注册表进程本地 + grace 足以区分在途 vs 孤儿。
const ORPHAN_GENERATING_GRACE_SECS: i64 = 600;

/// 对账**已取到的** rows（不再自己二次全表扫——由 `list_all_artifacts` 单次 fetch 传入）。
/// 返回 `true` = 有孤儿被降级（调用方据此才需重取一次反映新 status）。
fn reconcile_orphaned_generating(rows: &[DesignArtifact]) -> bool {
    let now_ts = chrono::Utc::now();
    let orphans: Vec<String> = {
        let live = generation_cancels()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        rows.iter()
            .filter(|a| {
                a.status == "generating"
                    && !live.contains_key(&a.id)
                    && chrono::DateTime::parse_from_rfc3339(&a.updated_at)
                        .map(|t| {
                            (now_ts - t.with_timezone(&chrono::Utc)).num_seconds()
                                > ORPHAN_GENERATING_GRACE_SECS
                        })
                        .unwrap_or(true)
            })
            .map(|a| a.id.clone())
            .collect()
    };
    let mut degraded_any = false;
    for id in orphans {
        match degrade_to_placeholder(&id, "failed") {
            Ok(true) => {
                degraded_any = true;
                crate::app_warn!(
                    "design",
                    "generate",
                    "recovered orphaned generating artifact {}",
                    id
                );
            }
            Ok(false) => {}
            Err(e) => crate::app_warn!(
                "design",
                "generate",
                "reconcile orphan {} failed: {}",
                id,
                e
            ),
        }
    }
    degraded_any
}

/// 建 generating 壳：status=generating + 流式占位 index.html（CSS-first head 定稿 + 空 body
/// 容器 + 常驻接收脚本），立即返回让前端挂稳定 iframe。内容由 `stream_generate_artifact` 回填。
pub fn create_artifact_shell(input: &CreateArtifactInput) -> Result<DesignArtifact> {
    let db = open_db()?;
    let kind = ArtifactKind::from_str(&input.kind)
        .with_context(|| format!("unknown artifact kind: {}", input.kind))?;
    let project = db
        .get_project(&input.project_id)?
        .with_context(|| format!("project not found: {}", input.project_id))?;
    let system_id = input
        .system_id
        .clone()
        .or(project.default_system_id.clone())
        .or_else(|| {
            crate::config::cached_config()
                .design
                .default_system_id
                .clone()
                .filter(|s| !s.trim().is_empty())
        });
    let ts = now();
    let artifact_id = new_id();
    let title = if input.title.trim().is_empty() {
        format!("未命名{}", kind.as_str())
    } else {
        input.title.trim().to_string()
    };

    let dir = paths::design_artifact_dir(&input.project_id, &artifact_id)?;
    let tokens = resolve_tokens(system_id.as_deref());
    let host_html = renderer::build_stream_host_html(kind, &title, &tokens);
    std::fs::create_dir_all(dir.join("source"))?;
    write_atomic(&dir.join("index.html"), host_html.as_bytes())?;

    let (vw, vh) = kind.default_viewport();
    let artifact = DesignArtifact {
        id: artifact_id.clone(),
        project_id: input.project_id.clone(),
        title: title.clone(),
        kind: kind.as_str().to_string(),
        system_id: system_id.clone(),
        status: "generating".to_string(),
        viewport_w: if vw > 0 { Some(vw) } else { None },
        viewport_h: if vh > 0 { Some(vh) } else { None },
        current_version: 1,
        critique_score: None,
        thumbnail_path: None,
        created_at: ts.clone(),
        updated_at: ts.clone(),
        metadata: None,
    };
    let meta = ArtifactMeta {
        id: artifact.id.clone(),
        project_id: artifact.project_id.clone(),
        title: artifact.title.clone(),
        kind: artifact.kind.clone(),
        system_id: artifact.system_id.clone(),
        current_version: 1,
    };
    write_atomic(
        &dir.join("artifact.json"),
        serde_json::to_string_pretty(&meta)?.as_bytes(),
    )?;

    let persisted = (|| -> Result<()> {
        db.create_artifact(&artifact)?;
        db.touch_project(&input.project_id, &ts)?;
        Ok(())
    })();
    if let Err(e) = persisted {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    emit(
        "design:artifact_generating",
        json!({
            "projectId": input.project_id,
            "artifactId": artifact_id,
            "sessionId": input.session_id,
        }),
    );
    Ok(artifact)
}

/// 轻量 status setter（不 bump 版本 / 不重渲染）——status 单点切换用。
pub fn set_artifact_status(id: &str, status: &str) -> Result<()> {
    open_db()?.update_artifact(id, None, Some(status), None, None, None, &now())
}

/// 定稿 generating 产物：`artifact_lock` 下单次 render(editable) + write_working +
/// write_version_snapshot + status=ready + create_version(1)，随后 emit done。
///
/// 返回 `None` = 产物在定稿前已被删（`delete_artifact` 也持同一 `artifact_lock` 故二者互斥；
/// get 到 None 即 mid-finalize 被删）→ **静默 no-op**：不写盘复活已删目录、不 emit
/// generate_error（守 #6：不对已删产物误报「生成失败」、不产孤儿目录）。
pub fn finalize_generating_artifact(
    id: &str,
    parts: &ArtifactParts,
) -> Result<Option<DesignArtifact>> {
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    let db = open_db()?;
    let Some(a) = db.get_artifact(id)? else {
        return Ok(None);
    };
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &a.title, parts, &tokens)?;
    write_working(&dir, &html, parts, &oidmap_json)?;
    write_version_snapshot(&dir, a.current_version, &html, parts, &oidmap_json)?;

    let ts = now();
    // 生成定稿：对模型产出的正文跑确定性自查，命中翻 needs_review + 写 selfCheck 元数据。
    let (status, self_check_meta) = resolve_self_check(a.metadata.as_deref(), &parts.body_html);
    db.update_artifact_review(id, None, &status, None, self_check_meta.as_deref(), &ts)?;
    // 壳未建版本行——定稿补首版（避免 list_versions 为空）。
    db.create_version(&DesignArtifactVersion {
        id: 0,
        artifact_id: a.id.clone(),
        version_number: a.current_version,
        message: Some("Generated".to_string()),
        critique_score: None,
        created_at: ts.clone(),
    })?;
    db.touch_project(&a.project_id, &ts)?;

    // 只发 generate_done（前端据此做唯一一次受控 swap 到定稿 index.html）；不再叠发
    // design:reload——否则前端 done + reload 两条都 previewKey++ = 双重 remount 双闪。
    emit(
        "design:generate_done",
        json!({ "projectId": a.project_id, "artifactId": a.id }),
    );
    Ok(Some(db.get_artifact(id)?.unwrap_or(a)))
}

/// 后端流式编排（建壳后 spawn）：逐帧回填预览 → 定稿 / 降级 failed。
pub async fn stream_generate_artifact(
    artifact_id: String,
    project_id: String,
    brief: String,
    kind: ArtifactKind,
    system_md: String,
    tokens: BTreeMap<String, String>,
    cancel: Arc<AtomicBool>,
) {
    // 本流唯一 id + 单调 seq：前端按 streamId 变化重置累积、按 seq 丢乱序帧（EventBus 无 seq）。
    // move 闭包持事件字段的独立克隆 + 内部 .clone()，保证是 Fn（可反复调）而非 FnOnce。
    let stream_id = new_id();
    let seq = std::sync::atomic::AtomicU64::new(0);
    let ev_project = project_id.clone();
    let ev_artifact = artifact_id.clone();
    let on_snapshot = move |parts: &ArtifactParts| {
        let n = seq.fetch_add(1, Ordering::SeqCst);
        emit(
            "design:generate_delta",
            json!({
                "projectId": ev_project.clone(),
                "artifactId": ev_artifact.clone(),
                "streamId": stream_id.clone(),
                "seq": n,
                "css": parts.css.clone(),
                "bodyHtml": parts.body_html.clone(),
                "done": false,
            }),
        );
    };

    let result = super::generate::stream_design_parts(
        &brief,
        kind,
        &system_md,
        &tokens,
        &cancel,
        &on_snapshot,
    )
    .await;

    // 已取消（产物被删 / regenerate）：不 finalize（可能写已删目录）、不 emit。
    if cancel.load(Ordering::SeqCst) {
        return;
    }

    match result {
        Ok(parts) => match finalize_generating_artifact(&artifact_id, &parts) {
            // 成功定稿。
            Ok(Some(_)) => {}
            // 定稿前已被删 → 静默（不误报 generate_error、不复活目录）。
            Ok(None) => {}
            Err(e) => {
                // 已删（degrade→Ok(false)）不 emit generate_error（对齐 #6 静默契约）；
                // 真降级 / degrade 自身出错才报失败。
                if !matches!(degrade_to_placeholder(&artifact_id, "failed"), Ok(false)) {
                    emit(
                        "design:generate_error",
                        json!({ "projectId": project_id, "artifactId": artifact_id, "reason": e.to_string() }),
                    );
                    crate::app_warn!(
                        "design",
                        "generate",
                        "finalize streaming artifact {} failed: {}",
                        artifact_id,
                        e
                    );
                }
            }
        },
        Err(e) => {
            // 失败降级为干净占位（非 spinner 壳），status=failed。已删则静默（守 #6）。
            if !matches!(degrade_to_placeholder(&artifact_id, "failed"), Ok(false)) {
                emit(
                    "design:generate_error",
                    json!({ "projectId": project_id, "artifactId": artifact_id, "reason": e.to_string() }),
                );
                crate::app_warn!(
                    "design",
                    "generate",
                    "streaming generation for {} failed, degraded to placeholder: {}",
                    artifact_id,
                    e
                );
            }
        }
    }
}

/// owner/GUI「一句话 → 流式生成」入口：建壳同步返回 → spawn 流式回填。
///
/// image 形态 / 无 brief / 未知 kind → 回落阻塞 `create_artifact_generating`（无流式意义 +
/// 兜底）。非流式路径完整保留作 agent 工具面 + 无 tokio runtime 时的退路。
pub async fn generate_design_artifact(input: CreateArtifactInput) -> Result<DesignArtifact> {
    let text_brief = input.prompt.clone().unwrap_or_default();
    let has_ref = input
        .reference_image_b64
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let kind_opt = ArtifactKind::from_str(&input.kind);
    // 媒体 / 组件 / 未知 kind → 阻塞 / 空壳路径（图→产物只对 HTML 形态）。
    // 无任何生成信号（无 brief 且无参考图）→ 空壳。
    if input.kind == "image"
        || input.kind == "audio"
        || input.kind == "component"
        || kind_opt.is_none()
        || (text_brief.trim().is_empty() && !has_ref)
    {
        return create_artifact_generating(input).await;
    }
    let kind = kind_opt.expect("checked above");
    let (system_md, tokens) = resolve_system_for_generation(&input);
    let ref_b64 = if has_ref {
        input.reference_image_b64.clone()
    } else {
        None
    };

    // 建壳优先 + 立即返回（含参考图路径）：库里即出 generating 壳、cancel 覆盖 describe 阶段；
    // 参考图的 vision 描述（可能 ~90s）移进后台任务，命令不阻塞、模态可即时关闭（review #2/#4）。
    let shell = create_artifact_shell(&input)?;
    let cancel = register_generation_cancel(&shell.id);
    let artifact_id = shell.id.clone();
    let project_id = shell.project_id.clone();
    tokio::spawn(async move {
        use futures_util::future::FutureExt;
        // 参考图 → vision 描述成重建 brief（+ 叠加文本要求）；描述失败回退文本 brief。
        let brief = match ref_b64 {
            Some(b64) => match super::extract::describe_reference_image(&b64, kind).await {
                Ok(desc) if text_brief.trim().is_empty() => desc,
                Ok(desc) => format!("{desc}\n\n额外要求：{text_brief}"),
                Err(e) => {
                    crate::app_warn!(
                        "design",
                        "generate",
                        "reference image describe failed ({e}), falling back to text brief"
                    );
                    text_brief.clone()
                }
            },
            None => text_brief.clone(),
        };
        // describe 失败且无文本 brief → 降级壳为空白占位（ready，可编辑），不永久转圈。
        if brief.trim().is_empty() {
            let _ = degrade_to_placeholder(&artifact_id, "ready");
            clear_generation_cancel(&artifact_id, &cancel);
            return;
        }
        // catch_unwind：spawned future 内部 panic（generate / finalize 里的意外）不留持久
        // generating 半态——降级为 failed 占位 + 清 cancel flag，而非永久转圈。
        let ran = std::panic::AssertUnwindSafe(stream_generate_artifact(
            artifact_id.clone(),
            project_id.clone(),
            brief,
            kind,
            system_md,
            tokens,
            cancel.clone(),
        ))
        .catch_unwind()
        .await;
        if ran.is_err() && !matches!(degrade_to_placeholder(&artifact_id, "failed"), Ok(false)) {
            // 产物已删（degrade→Ok(false)）则整段静默（守 #6：不对已删产物报「生成失败」）。
            emit(
                "design:generate_error",
                json!({ "projectId": project_id, "artifactId": artifact_id, "reason": "internal panic" }),
            );
            crate::app_warn!(
                "design",
                "generate",
                "streaming generation for {} panicked, degraded to placeholder",
                artifact_id
            );
        }
        clear_generation_cancel(&artifact_id, &cancel);
    });
    Ok(shell)
}

pub fn list_artifacts(project_id: &str) -> Result<Vec<DesignArtifact>> {
    open_db()?.list_artifacts(project_id)
}

pub fn list_all_artifacts() -> Result<Vec<DesignArtifact>> {
    let db = open_db()?;
    let rows = db.list_all_artifacts()?;
    // library-wall 加载时顺带对账上个进程崩溃留下的 generating 孤儿（design 无专用启动钩子）。
    // 复用已取的 rows；仅真有孤儿被降级时才重取一次反映新 status（无孤儿常态零额外扫表）。
    if reconcile_orphaned_generating(&rows) {
        return db.list_all_artifacts();
    }
    Ok(rows)
}

pub fn get_artifact(id: &str) -> Result<Option<DesignArtifact>> {
    open_db()?.get_artifact(id)
}

pub fn delete_artifact(id: &str) -> Result<()> {
    // 先取消在途流式生成，否则它会白流完 + finalize 往已删目录写。
    cancel_generation(id);
    // 持 artifact_lock：与 finalize_generating_artifact 互斥——要么 finalize 完整跑完（随后本
    // delete 清干净），要么 delete 先删（finalize 内 get 到 None 静默跳过），二者不再交错产孤儿
    // 目录 / 误 emit generate_error。
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let db = open_db()?;
    if let Some(a) = db.get_artifact(id)? {
        db.delete_artifact(id)?;
        if let Ok(dir) = paths::design_artifact_dir(&a.project_id, id) {
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        db.touch_project(&a.project_id, &now())?;
        emit(
            "design:artifact_deleted",
            json!({ "projectId": a.project_id, "artifactId": id }),
        );
    }
    Ok(())
}

pub fn list_versions(artifact_id: &str) -> Result<Vec<DesignArtifactVersion>> {
    open_db()?.list_versions(artifact_id)
}

/// 产物预览信息（前端 iframe 加载用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    #[serde(flatten)]
    pub artifact: DesignArtifact,
    /// 产物目录绝对路径（前端拼 `/index.html`）。
    pub artifact_path: String,
    /// 当前 body.html 的 BLAKE3（可视化编辑 stale-write 守卫用）。
    pub body_hash: String,
}

pub fn get_artifact_view(id: &str) -> Result<Option<ArtifactView>> {
    let Some(artifact) = open_db()?.get_artifact(id)? else {
        return Ok(None);
    };
    let artifact_path = artifact_dir_str(&artifact.project_id, &artifact.id);
    let dir = paths::design_artifact_dir(&artifact.project_id, &artifact.id)?;
    let body = read_source(&dir)?.body_html;
    let body_hash = patch::body_hash(&body);
    Ok(Some(ArtifactView {
        artifact,
        artifact_path,
        body_hash,
    }))
}

/// 可视化微调：单元素样式 / 文本回写（D1）。text 先于 style 应用（两段字节范围
/// 不重叠且 text 在 open tag 之后，故 style 用同一 oidmap 仍有效）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementPatch {
    pub artifact_id: String,
    pub oid: u32,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub styles: Option<Vec<(String, String)>>,
    /// 可选 stale-write 守卫（load 时拿到的 bodyHash）。
    #[serde(default)]
    pub expected_hash: Option<String>,
}

pub fn patch_element(p: ElementPatch) -> Result<DesignArtifact> {
    let db = open_db()?;
    let a = db
        .get_artifact(&p.artifact_id)?
        .with_context(|| format!("artifact not found: {}", p.artifact_id))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let body = std::fs::read_to_string(dir.join("source").join("body.html")).unwrap_or_default();
    let oidmap: Vec<patch::OidEntry> = std::fs::read_to_string(dir.join("oidmap.json"))
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default();

    // Hash of the body we patch against. Checked here (client's load-time guard) and
    // re-checked under the write lock in `update_artifact` (closes the TOCTOU).
    let base_hash = patch::body_hash(&body);
    if let Some(h) = &p.expected_hash {
        if base_hash != *h {
            anyhow::bail!("stale write: source changed, please re-select");
        }
    }

    let mut new_body = body;
    // 先文本（改内部内容，位于 open tag 之后），后样式（改 open tag，range 未被文本移动）。
    if let Some(text) = &p.text {
        let r = patch::apply_text_patch(&new_body, &oidmap, p.oid, text, None)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        new_body = r.new_source;
    }
    if let Some(styles) = &p.styles {
        if !styles.is_empty() {
            let r = patch::apply_style_patch(&new_body, &oidmap, p.oid, styles, None)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            new_body = r.new_source;
        }
    }

    update_artifact(UpdateArtifactInput {
        id: a.id.clone(),
        title: None,
        body_html: Some(new_body),
        css: None,
        js: None,
        message: Some("Visual edit".to_string()),
        expected_body_hash: Some(base_hash),
    })
}

/// 更新产物：未提供的字段沿用当前源，重新渲染 + 累加版本 + 剪旧版本。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateArtifactInput {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub css: Option<String>,
    #[serde(default)]
    pub js: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    /// Optional stale-write guard re-verified **under the per-artifact lock** right
    /// before writing (closes the `patch_element` read→write TOCTOU). Not exposed to
    /// the agent `update_artifact` path — only `patch_element` sets it.
    #[serde(default)]
    pub expected_body_hash: Option<String>,
}

/// Delete on-disk version snapshot dirs that the DB no longer retains, so disk
/// tracks the DB's kept `version_number` set **exactly** (robust to non-contiguous
/// version numbers from crashes — the old arithmetic `current-keep` cutoff diverged
/// from `cleanup_old_versions` on any gap and could orphan a still-listed version →
/// `restore_version` "version not found").
fn prune_version_dirs_to_db(dir: &std::path::Path, keep: &std::collections::HashSet<i64>) {
    let vroot = dir.join("versions");
    let Ok(entries) = std::fs::read_dir(&vroot) else {
        return;
    };
    for entry in entries.flatten() {
        let keep_this = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i64>().ok())
            .map(|n| keep.contains(&n))
            .unwrap_or(true); // non-numeric entry: leave it alone
        if !keep_this {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Per-artifact in-process mutex. Serializes the read-current → write → bump →
/// create_version → prune sequence so two concurrent updates on the same artifact
/// cannot lost-update, collide on `UNIQUE(artifact_id,version_number)`, or leave the
/// version dir's content mismatched against its DB row. `open_db()` opens a fresh
/// connection per call, so SQLite file locks alone do NOT serialize this logical RMW.
fn artifact_lock(artifact_id: &str) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(artifact_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub fn update_artifact(input: UpdateArtifactInput) -> Result<DesignArtifact> {
    // Serialize the whole RMW for this artifact (see `artifact_lock`). Held across
    // sync file + DB IO only (no `.await` inside), so a std mutex is correct here.
    let lock = artifact_lock(&input.id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    let db = open_db()?;
    let a = db
        .get_artifact(&input.id)?
        .with_context(|| format!("artifact not found: {}", input.id))?;
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let existing = read_source(&dir)?;
    // Stale-write guard re-checked under the lock: if the on-disk body changed since
    // the caller computed its patch (e.g. a racing edit), abort instead of lost-update.
    if let Some(expected) = &input.expected_body_hash {
        if patch::body_hash(&existing.body_html) != *expected {
            anyhow::bail!("stale write: source changed, please re-select");
        }
    }
    let parts = ArtifactParts {
        body_html: input.body_html.unwrap_or(existing.body_html),
        css: input.css.unwrap_or(existing.css),
        js: input.js.unwrap_or(existing.js),
    };
    let title = input.title.clone().unwrap_or_else(|| a.title.clone());
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &title, &parts, &tokens)?;
    write_working(&dir, &html, &parts, &oidmap_json)?;

    let next = a.current_version + 1;
    write_version_snapshot(&dir, next, &html, &parts, &oidmap_json)?;

    let ts = now();
    // 编辑落新版本：重跑确定性自查——改好的正文清 selfCheck 标记回 ready，仍 slop 保持标记。
    let (status, self_check_meta) = resolve_self_check(a.metadata.as_deref(), &parts.body_html);
    db.update_artifact_review(
        &a.id,
        input.title.as_deref(),
        &status,
        Some(next),
        self_check_meta.as_deref(),
        &ts,
    )?;
    db.create_version(&DesignArtifactVersion {
        id: 0,
        artifact_id: a.id.clone(),
        version_number: next,
        message: input.message.or_else(|| Some("Update".to_string())),
        critique_score: None,
        created_at: ts.clone(),
    })?;
    let keep = crate::config::cached_config()
        .design
        .max_versions_per_artifact
        .max(1);
    let _ = db.cleanup_old_versions(&a.id, keep);
    // Prune disk snapshots to exactly the versions the DB retained.
    if let Ok(remaining) = db.list_versions(&a.id) {
        let keep_set: std::collections::HashSet<i64> =
            remaining.iter().map(|v| v.version_number).collect();
        prune_version_dirs_to_db(&dir, &keep_set);
    }
    db.touch_project(&a.project_id, &ts)?;

    emit("design:reload", json!({ "artifactId": a.id }));
    db.get_artifact(&a.id)?
        .context("artifact gone after update")
}

// ── Knowledge integration (D4) ─────────────────────────────────────

/// 把产物沉淀为知识空间笔记（进第二大脑可检索）。`kb_id` 缺省用默认 KB。
pub fn save_to_knowledge(artifact_id: &str, kb_id: Option<&str>) -> Result<String> {
    let db = open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;

    let kb = match kb_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(k) => k.to_string(),
        None => {
            crate::knowledge::service::ensure_default_knowledge_base();
            crate::knowledge::service::list_kb_meta(false)?
                .into_iter()
                .next()
                .map(|m| m.kb.id)
                .context("no knowledge base available")?
        }
    };

    // Disambiguate by artifact id so two artifacts with colliding safe-filenames
    // (or empty titles → "design") don't silently overwrite each other's KB note.
    let rel = format!(
        "设计/{}-{}.md",
        safe_filename(&a.title),
        a.id.get(..8).unwrap_or(&a.id)
    );
    let content = format!(
        "---\ntitle: {title}\nkind: {kind}\nsource: design-space\nartifactId: {aid}\n---\n\n\
# {title}\n\n> 来自设计空间的产物（{kind}）。\n\n\
```html\n{body}\n```\n",
        title = a.title,
        kind = a.kind,
        aid = a.id,
        body = parts.body_html,
    );
    let hash = crate::knowledge::service::note_save(&kb, &rel, &content, None, false)?;
    crate::app_info!(
        "design",
        "service",
        "saved artifact {} to knowledge base {}",
        a.id,
        kb
    );
    Ok(hash)
}

// ── Quality gate ───────────────────────────────────────────────────

/// 对产物跑 5 维质量评审门，落总分到产物行。
pub async fn critique_artifact(id: &str) -> Result<super::critique::CritiqueResult> {
    let db = open_db()?;
    let a = db
        .get_artifact(id)?
        .with_context(|| format!("artifact not found: {id}"))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let html = std::fs::read_to_string(dir.join("index.html")).unwrap_or_default();
    let system_md = a
        .system_id
        .as_deref()
        .and_then(|sid| system::read_full(&db, sid).ok())
        .map(|f| f.system_md);
    let result = super::critique::critique_html(&html, system_md.as_deref()).await?;
    let _ = db.update_artifact(&a.id, None, None, None, Some(result.overall), None, &now());
    emit(
        "design:critiqued",
        json!({ "artifactId": a.id, "overall": result.overall }),
    );
    Ok(result)
}

// ── Export ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub filename: String,
    pub mime: String,
    pub content: String,
}

fn safe_filename(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if trimmed.is_empty() {
        "design".to_string()
    } else {
        trimmed
    }
}

/// 导出产物。Phase 5：`html`（干净自包含，无 bridge/oid）。
pub fn export_artifact(id: &str, format: &str) -> Result<ExportResult> {
    let db = open_db()?;
    let a = db
        .get_artifact(id)?
        .with_context(|| format!("artifact not found: {id}"))?;
    let kind =
        ArtifactKind::from_str(&a.kind).with_context(|| format!("unknown kind: {}", a.kind))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    match format {
        "html" => {
            let parts = read_source(&dir)?;
            let tokens = resolve_tokens(a.system_id.as_deref());
            // editable=false → 无 inspector bridge / 无 oid，干净可交付；Component 走编译。
            let html = render_clean(kind, &a.title, &parts, &tokens);
            Ok(ExportResult {
                filename: format!("{}.html", safe_filename(&a.title)),
                mime: "text/html".to_string(),
                content: html,
            })
        }
        "markdown" | "md" => {
            let parts = read_source(&dir)?;
            let md = htmd::convert(&parts.body_html).unwrap_or_default();
            let content = if md.trim().is_empty() {
                format!("# {}\n", a.title)
            } else {
                md.trim().to_string()
            };
            Ok(ExportResult {
                filename: format!("{}.md", safe_filename(&a.title)),
                mime: "text/markdown".to_string(),
                content,
            })
        }
        other => anyhow::bail!("unsupported export format: {other}"),
    }
}

/// 项目级 ZIP 的根画廊页（自包含，链接到各产物目录）。
fn project_gallery_html(project_title: &str, items_li: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"zh\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title>\n<style>\
body{{font-family:system-ui,-apple-system,\"PingFang SC\",\"Microsoft YaHei\",sans-serif;\
max-width:880px;margin:48px auto;padding:0 24px;color:#111827;background:#fff}}\
h1{{font-size:24px;margin:0 0 4px}}p{{color:#6b7280;margin:0 0 24px}}\
ul{{list-style:none;padding:0;display:grid;gap:10px}}\
li{{display:flex;align-items:center;gap:10px;padding:14px 16px;border:1px solid #e5e7eb;border-radius:12px}}\
li a{{font-weight:600;color:#2563eb;text-decoration:none}}\
li span{{margin-left:auto;font-size:12px;color:#9ca3af;text-transform:uppercase;letter-spacing:.04em}}\
</style></head><body>\n<h1>{title}</h1>\n<p>设计空间导出 · {n} 个产物 · 各目录内 index.html 可直接打开</p>\n\
<ul>\n{items}\n</ul>\n</body></html>\n",
        title = renderer::html_escape(project_title),
        n = items_li.matches("<li>").count(),
        items = items_li,
    )
}

/// 导出 ZIP：`artifact_id` = 单产物源码包（index.html + source/ + README）；
/// `project_id` = 项目级全产物包（每产物一目录 + 根 index.html 画廊）。返回 base64。
pub fn export_zip(artifact_id: Option<&str>, project_id: Option<&str>) -> Result<String> {
    use base64::Engine;
    let db = open_db()?;
    let (items, index_html): (Vec<super::export::ZipArtifact>, Option<String>) = if let Some(aid) =
        artifact_id.filter(|s| !s.is_empty())
    {
        let a = db
            .get_artifact(aid)?
            .with_context(|| format!("artifact not found: {aid}"))?;
        let kind =
            ArtifactKind::from_str(&a.kind).with_context(|| format!("unknown kind: {}", a.kind))?;
        let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
        let parts = read_source(&dir)?;
        let tokens = resolve_tokens(a.system_id.as_deref());
        let html = render_clean(kind, &a.title, &parts, &tokens);
        (
            vec![super::export::ZipArtifact {
                folder: String::new(),
                html,
                source: Some((parts.body_html, parts.css, parts.js)),
                title: a.title,
                kind: a.kind,
            }],
            None,
        )
    } else if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        let project = db
            .get_project(pid)?
            .with_context(|| format!("project not found: {pid}"))?;
        let artifacts = db.list_artifacts(pid)?;
        let mut zitems = Vec::new();
        let mut gallery = String::new();
        for a in &artifacts {
            let Some(kind) = ArtifactKind::from_str(&a.kind) else {
                continue;
            };
            let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
            let parts = read_source(&dir)?;
            let tokens = resolve_tokens(a.system_id.as_deref());
            let html = render_clean(kind, &a.title, &parts, &tokens);
            let folder = format!(
                "{}-{}",
                safe_filename(&a.title),
                a.id.get(..8).unwrap_or(&a.id)
            );
            gallery.push_str(&format!(
                "<li><a href=\"{f}/index.html\">{t}</a><span>{k}</span></li>\n",
                f = folder,
                t = renderer::html_escape(&a.title),
                k = renderer::html_escape(&a.kind),
            ));
            zitems.push(super::export::ZipArtifact {
                folder,
                html,
                // 项目整包也带各产物的可编辑源码分离目录（source/），与单产物包一致。
                source: Some((parts.body_html, parts.css, parts.js)),
                title: a.title.clone(),
                kind: a.kind.clone(),
            });
        }
        if zitems.is_empty() {
            anyhow::bail!("project has no artifacts to export");
        }
        (zitems, Some(project_gallery_html(&project.title, &gallery)))
    } else {
        anyhow::bail!("export_zip needs an artifactId or projectId");
    };
    let bytes = super::export::build_zip(&items, index_html.as_deref())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// 由前端栅格化的整页 PNG（base64，可带 data-uri 前缀）组装 PPTX，返回 base64。
/// PNG/PDF 走前端客户端栅格化；PPTX 因需 zip 打包由此后端构建（见 design/export.rs）。
pub fn export_pptx(slides_b64: &[String], title: &str) -> Result<String> {
    use base64::Engine;
    let mut slides = Vec::with_capacity(slides_b64.len());
    for raw in slides_b64 {
        let b64 = raw
            .split_once(",")
            .map(|(_, rest)| rest)
            .unwrap_or(raw.as_str());
        let png = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .context("invalid base64 slide image")?;
        slides.push(super::export::SlideImage { png });
    }
    let bytes = super::export::build_pptx(&slides, title)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// 判断源码是否引用了 `var(name)`：扫每个 `var(`、跳过 `(` 后空白（`var( --x )` 合法 CSS）、
/// 匹配 name、再要求 name 后紧跟 `)` / `,` / 空白 / 结尾（避免 `--ds-color` 误命中
/// `--ds-color-primary`）。
fn css_var_referenced(hay: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = hay[from..].find("var(") {
        let after_paren = from + rel + 4;
        let rest = &hay[after_paren..];
        let ws = rest.len() - rest.trim_start().len();
        let name_start = after_paren + ws;
        if hay[name_start..].starts_with(name) {
            let after_name = name_start + name.len();
            match hay.as_bytes().get(after_name) {
                None | Some(b')') | Some(b',') | Some(b' ') | Some(b'\t') | Some(b'\n')
                | Some(b'\r') => return true,
                _ => {}
            }
        }
        from = after_paren;
    }
    false
}

/// 剥掉 `/* … */` 块注释（CSS/JS 通用、无歧义；`//` 行注释在 CSS/URL 里有歧义故不剥）——
/// 避免注释里出现的 token 名被误判为引用。UTF-8 安全。
fn strip_block_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 扫描产物源码，返回其**实际引用**的 `--ds-*` token（name, value），按名排序。
fn referenced_tokens(parts: &ArtifactParts, all: &[(String, String)]) -> Vec<(String, String)> {
    let raw = format!("{}\n{}\n{}", parts.body_html, parts.css, parts.js);
    let hay = strip_block_comments(&raw);
    all.iter()
        .filter(|(name, _)| css_var_referenced(&hay, name))
        .cloned()
        .collect()
}

/// GFM 表格单元格转义：`|`→`\|`、换行→空格、反引号→单引号（防破表 / 破代码跨度）。
fn md_table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ").replace('`', "'")
}

/// 组装开发交付包的 `HANDOFF.md`（目录说明 + 本产物引用的设计变量 + token 格式清单）。
fn build_handoff_md(
    a: &DesignArtifact,
    system_name: Option<&str>,
    referenced: &[(String, String)],
    dev_formats: &[super::token_export::TokenExport],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {} — 开发交付包\n\n", a.title));
    s.push_str(&format!("- 形态（kind）：`{}`\n", a.kind));
    if let Some(name) = system_name {
        s.push_str(&format!("- 设计系统：{name}\n"));
    }
    s.push_str(
        "\n## 目录结构\n\n\
- `index.html` — 自包含产物（零外部依赖，浏览器直接打开）\n\
- `source/` — 源码（`body.html` / `style.css` / `script.js`）\n\
- `tokens/` — 设计变量的多平台开发者代码\n\n",
    );
    if referenced.is_empty() {
        s.push_str("## 本产物引用的设计变量\n\n（未检测到 `var(--ds-*)` 引用）\n\n");
    } else {
        s.push_str("## 本产物引用的设计变量\n\n| Token | 值 (value) |\n| --- | --- |\n");
        for (name, value) in referenced {
            // GFM 表格单元格：转义 `|`、换行→空格、反引号→单引号（否则破表 / 破代码跨度）。
            s.push_str(&format!("| `{}` | `{}` |\n", md_table_cell(name), md_table_cell(value)));
        }
        s.push('\n');
    }
    s.push_str("## Token 导出格式\n\n");
    for e in dev_formats {
        s.push_str(&format!("- `tokens/{}` — {}\n", e.filename, e.label));
    }
    s.push_str(
        "\n> 接入时用 `tokens/` 里对应平台的文件注入设计变量；产物 CSS 以 `var(--ds-*)` 引用，\
换设计系统即换皮、一致性由 token 锁定。\n",
    );
    s
}

/// 导出**代码交付包**（开发者 handoff）：把产物的干净 `index.html` + `source/` + 多平台
/// token（复用 `token_export`）+ `HANDOFF.md` 规范打成一个 ZIP。返回 base64 的 `ExportResult`。
pub fn export_handoff(artifact_id: &str) -> Result<ExportResult> {
    use base64::Engine;
    let db = open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let kind =
        ArtifactKind::from_str(&a.kind).with_context(|| format!("unknown kind: {}", a.kind))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;
    let tokens_vec = resolve_tokens(a.system_id.as_deref());
    // 干净可交付（editable=false，无 inspector/oid）；Component 走 oxc 编译，绝不塞未编译 JSX。
    let html = render_clean(kind, &a.title, &parts, &tokens_vec);

    let tokens_map: std::collections::BTreeMap<String, String> = tokens_vec.iter().cloned().collect();
    let dev = super::token_export::export_all(&tokens_map);
    let referenced = referenced_tokens(&parts, &tokens_vec);
    let system_name = a
        .system_id
        .as_deref()
        .and_then(|id| system::read_full(&db, id).ok().map(|f| f.meta.name));
    let spec = build_handoff_md(&a, system_name.as_deref(), &referenced, &dev);

    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("index.html".to_string(), html.into_bytes()),
        ("HANDOFF.md".to_string(), spec.into_bytes()),
        ("source/body.html".to_string(), parts.body_html.into_bytes()),
        ("source/style.css".to_string(), parts.css.into_bytes()),
        ("source/script.js".to_string(), parts.js.into_bytes()),
    ];
    for e in &dev {
        files.push((format!("tokens/{}", e.filename), e.content.clone().into_bytes()));
    }
    let bytes = super::export::build_files_zip(&files)?;
    Ok(ExportResult {
        filename: format!("{}-handoff.zip", safe_filename(&a.title)),
        mime: "application/zip".to_string(),
        content: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

// ── Design systems ─────────────────────────────────────────────────

/// 列出设计系统（首次调用懒 seed 内置系统）。
pub fn list_systems() -> Result<Vec<DesignSystemMeta>> {
    let db = open_db()?;
    system::ensure_builtins(&db)?;
    db.list_systems()
}

/// 读取设计系统正文 + token。
pub fn get_system_full(id: &str) -> Result<DesignSystemFull> {
    let db = open_db()?;
    system::ensure_builtins(&db)?;
    system::read_full(&db, id)
}

/// 新建 / 更新用户设计系统入参。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSystemInput {
    /// 缺省则新建（生成 slug id）；提供则更新。
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    pub system_md: String,
    pub tokens: BTreeMap<String, String>,
    /// user | extracted（默认 user）。
    #[serde(default)]
    pub source: Option<String>,
}

fn slugify(name: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed: String = base
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if trimmed.is_empty() {
        format!("sys-{}", &new_id()[..8])
    } else {
        format!("{trimmed}-{}", &new_id()[..6])
    }
}

pub fn save_system(input: SaveSystemInput) -> Result<DesignSystemMeta> {
    let db = open_db()?;
    let id = input.id.clone().unwrap_or_else(|| slugify(&input.name));
    let source = input.source.as_deref().unwrap_or("user");
    let meta = system::save_system(
        &db,
        &id,
        &input.name,
        input.summary.as_deref(),
        &input.system_md,
        &input.tokens,
        source,
    )?;
    emit("design:system_changed", json!({ "systemId": id }));
    Ok(meta)
}

pub fn delete_system(id: &str) -> Result<()> {
    let db = open_db()?;
    system::delete_system(&db, id)?;
    emit("design:system_changed", json!({ "systemId": id }));
    Ok(())
}

// ── DESIGN.md 规范：导入 / 导出 ─────────────────────────────────────

/// 导入一份 **DESIGN.md** 文本为设计系统（互通格式）。抽取显式 token；不足则 LLM 合成。
/// `name` 空则取 DESIGN.md 首个标题 / 引言。source = `imported`。
pub async fn import_design_md(name: &str, md: &str) -> Result<DesignSystemMeta> {
    let extracted = super::extract::from_design_md(md).await?;
    let name = if name.trim().is_empty() {
        super::design_md::extract_summary(md).unwrap_or_else(|| "导入的设计系统".to_string())
    } else {
        name.trim().to_string()
    };
    let db = open_db()?;
    let id = slugify(&name);
    let meta = system::save_system(
        &db,
        &id,
        &name,
        Some(&extracted.summary),
        &extracted.system_md,
        &extracted.tokens,
        "imported",
    )?;
    emit("design:system_changed", json!({ "systemId": id }));
    Ok(meta)
}

/// 导出一个设计系统为规范 **DESIGN.md**（正文 prose + 末尾 Token 表，可无损回灌）。
pub fn export_design_md(system_id: &str) -> Result<String> {
    let db = open_db()?;
    system::ensure_builtins(&db)?;
    let full = system::read_full(&db, system_id)?;
    Ok(super::design_md::to_design_md(
        &full.system_md,
        &full.tokens,
    ))
}

/// 导出一个设计系统的 Token 为多平台开发者格式（CSS / SCSS / TS / Swift / Android / DTCG）。
pub fn export_tokens(system_id: &str) -> Result<Vec<super::token_export::TokenExport>> {
    let db = open_db()?;
    system::ensure_builtins(&db)?;
    let full = system::read_full(&db, system_id)?;
    Ok(super::token_export::export_all(&full.tokens))
}

/// 反向提取设计系统（D2）。`from = brief | codebase | url | image`。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractSystemInput {
    pub name: String,
    /// brief | codebase | url | image
    pub from: String,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// 设计方向选择器：为无品牌 brief 提 N 个候选方向（不落盘）。
pub async fn propose_directions(brief: &str, n: usize) -> Result<Vec<super::extract::Direction>> {
    super::extract::propose_directions(brief, n).await
}

pub async fn extract_system(input: ExtractSystemInput) -> Result<DesignSystemMeta> {
    let extracted = match input.from.as_str() {
        "brief" => super::extract::from_brief(input.brief.as_deref().unwrap_or_default()).await?,
        "codebase" => {
            let p = input
                .path
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .context("'path' required for from=codebase")?;
            super::extract::from_codebase(std::path::Path::new(p)).await?
        }
        "url" => {
            let u = input
                .url
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .context("'url' required for from=url")?;
            super::extract::from_url(u).await?
        }
        "image" => {
            let p = input
                .path
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .context("'path' (image file) required for from=image")?;
            super::extract::from_image(std::path::Path::new(p)).await?
        }
        other => anyhow::bail!("unsupported extract source: {other}"),
    };
    let name = if input.name.trim().is_empty() {
        "提取的设计系统".to_string()
    } else {
        input.name.trim().to_string()
    };
    let db = open_db()?;
    let id = slugify(&name);
    let meta = system::save_system(
        &db,
        &id,
        &name,
        Some(&extracted.summary),
        &extracted.system_md,
        &extracted.tokens,
        "extracted",
    )?;
    emit("design:system_changed", json!({ "systemId": id }));
    Ok(meta)
}

/// 从 **Figma 文件**导入品牌设计系统（**owner 平面专属**：需 Figma 访问令牌，凭据不进模型面）。
/// `url` 为 Figma 文件 URL 或 file key，`token` 为用户的 Figma 个人访问令牌（按次传、不落盘）。
pub async fn import_figma(url: &str, token: &str, name: Option<&str>) -> Result<DesignSystemMeta> {
    let extracted = super::extract::from_figma(url, token).await?;
    let name = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Figma 设计系统")
        .to_string();
    let db = open_db()?;
    let id = slugify(&name);
    let meta = system::save_system(
        &db,
        &id,
        &name,
        Some(&extracted.summary),
        &extracted.system_md,
        &extracted.tokens,
        "extracted",
    )?;
    emit("design:system_changed", json!({ "systemId": id }));
    Ok(meta)
}

/// 从历史版本恢复：读版本快照源码，生成一个**新**版本（原版本不动）。
pub fn restore_version(artifact_id: &str, version_number: i64) -> Result<DesignArtifact> {
    let db = open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let vsrc = dir
        .join("versions")
        .join(version_number.to_string())
        .join("source");
    if !vsrc.exists() {
        anyhow::bail!("version {version_number} not found");
    }
    let read = |name: &str| std::fs::read_to_string(vsrc.join(name)).unwrap_or_default();
    update_artifact(UpdateArtifactInput {
        id: a.id.clone(),
        title: None,
        body_html: Some(read("body.html")),
        css: Some(read("style.css")),
        js: Some(read("script.js")),
        message: Some(format!("Restored from v{version_number}")),
        expected_body_hash: None,
    })
}

// ── Comments (批注钉) ──────────────────────────────────────────────
//
// owner 平面：本机 / API key 信任。坐标是沙箱回传的**不可信**数值——所有 rel 位经
// `clamp_rel`（NaN/极值 → 0，钳 `[0,1]`）、oid 经 `sanitize_oid`（负值 → None）双校验后
// 才落盘（红线，对齐 atelier 的 finite/clamp 双校验）。snippet/body 截断防超长。

const SNIPPET_MAX_BYTES: usize = 400;
const BODY_MAX_BYTES: usize = 4000;

/// 沙箱回传坐标净化：非有限（NaN/Inf）→ 0，其余钳到 `[0,1]`。
fn clamp_rel(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// oid 净化：负值 / 缺省 → None（脱锚）。
fn sanitize_oid(oid: Option<i64>) -> Option<i64> {
    oid.filter(|v| *v >= 0)
}

/// 新建批注钉。校验产物存在；坐标钳制、摘要 / 正文截断。
pub fn add_comment(
    artifact_id: &str,
    oid: Option<i64>,
    rel_x: f64,
    rel_y: f64,
    tag: Option<&str>,
    snippet: Option<&str>,
    body: &str,
) -> Result<DesignComment> {
    let body = body.trim();
    if body.is_empty() {
        anyhow::bail!("comment body is empty");
    }
    let db = open_db()?;
    db.get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    // `truncate_utf8` 返回借用切片，故 snippet_owned 已是 Option<&str>（无需 as_deref）。
    let snippet_owned = snippet.map(|s| crate::truncate_utf8(s, SNIPPET_MAX_BYTES));
    let comment = db.add_comment(
        artifact_id,
        sanitize_oid(oid),
        clamp_rel(rel_x),
        clamp_rel(rel_y),
        tag.filter(|s| !s.is_empty()),
        snippet_owned,
        crate::truncate_utf8(body, BODY_MAX_BYTES),
        &now(),
    )?;
    crate::app_info!(
        "design",
        "comment",
        "add comment {} on artifact {} oid={:?}",
        comment.id,
        artifact_id,
        comment.oid
    );
    Ok(comment)
}

/// 列一个产物的全部批注钉（按 id）。
pub fn list_comments(artifact_id: &str) -> Result<Vec<DesignComment>> {
    open_db()?.list_comments(artifact_id)
}

/// 重锚：拖拽 / 设计变更后回写 oid + rel 位。
pub fn relocate_comment(
    artifact_id: &str,
    comment_id: i64,
    oid: Option<i64>,
    rel_x: f64,
    rel_y: f64,
) -> Result<bool> {
    open_db()?.update_comment_anchor(
        artifact_id,
        comment_id,
        sanitize_oid(oid),
        clamp_rel(rel_x),
        clamp_rel(rel_y),
    )
}

/// 编辑批注正文。
pub fn update_comment_body(artifact_id: &str, comment_id: i64, body: &str) -> Result<bool> {
    let body = body.trim();
    if body.is_empty() {
        anyhow::bail!("comment body is empty");
    }
    open_db()?.update_comment_body(
        artifact_id,
        comment_id,
        crate::truncate_utf8(body, BODY_MAX_BYTES),
    )
}

/// 标记已解决 / 取消解决。
pub fn set_comment_resolved(artifact_id: &str, comment_id: i64, resolved: bool) -> Result<bool> {
    open_db()?.set_comment_resolved(artifact_id, comment_id, resolved)
}

/// 删除批注钉。
pub fn delete_comment(artifact_id: &str, comment_id: i64) -> Result<bool> {
    open_db()?.delete_comment(artifact_id, comment_id)
}

/// 组装「按批注精修」的**短指令**（反馈 + 元素定位；**不含**当前设计——设计经
/// `refine_design_parts` 完整注入、不走截断，见 review #1）。
fn compose_refine_instruction(comment: &DesignComment) -> String {
    let mut b = String::new();
    b.push_str(&comment.body);
    if let Some(tag) = comment.tag.as_deref().filter(|s| !s.is_empty()) {
        b.push_str(&format!("\n（反馈针对元素 <{tag}>）"));
    }
    if let Some(snippet) = comment.snippet.as_deref().filter(|s| !s.is_empty()) {
        b.push_str(&format!("\n元素片段：{snippet}"));
    }
    b
}

/// 回灌对话 = 让 AI 按批注**精修产物**（design-space 原生：产物就地更新、无需切走）。
/// 复用生成管线：读当前设计 + 反馈 → `generate_design_parts` → 落新版本（`design:reload`
/// 刷新视图）。image/audio/component 形态不支持。
pub async fn refine_artifact_with_comment(
    artifact_id: &str,
    comment_id: i64,
) -> Result<DesignArtifact> {
    let (a, comment, current) = {
        let db = open_db()?;
        let a = db
            .get_artifact(artifact_id)?
            .with_context(|| format!("artifact not found: {artifact_id}"))?;
        let comment = db
            .get_comment(artifact_id, comment_id)?
            .with_context(|| format!("comment not found: {comment_id}"))?;
        let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
        let current = read_source(&dir)?;
        (a, comment, current)
    };
    if matches!(a.kind.as_str(), "image" | "audio" | "component") {
        anyhow::bail!("批注精修暂不支持 {} 形态", a.kind);
    }
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    let sys_input = CreateArtifactInput {
        project_id: a.project_id.clone(),
        title: a.title.clone(),
        kind: a.kind.clone(),
        system_id: a.system_id.clone(),
        body_html: None,
        css: None,
        js: None,
        session_id: None,
        prompt: None,
        reference_image_b64: None,
        reference_image_mime: None,
    };
    let (system_md, tokens) = resolve_system_for_generation(&sys_input);
    let instruction = compose_refine_instruction(&comment);
    crate::app_info!(
        "design",
        "comment",
        "refine artifact {} per comment {}",
        artifact_id,
        comment_id
    );
    // 完整注入当前设计（不截断）→ 只精改反馈所指、保留其余（review #1）。
    let parts =
        super::generate::refine_design_parts(&instruction, &current, kind, &system_md, &tokens)
            .await?;
    // 传 expected_body_hash：LLM 调用期间若有并发编辑改了源，则中止精修（stale-write 守卫，
    // 不静默丢用户改动，review #2）。
    update_artifact(UpdateArtifactInput {
        id: a.id.clone(),
        title: None,
        body_html: Some(parts.body_html),
        css: Some(parts.css),
        js: Some(parts.js),
        message: Some(format!("按批注 #{comment_id} 精修")),
        expected_body_hash: Some(patch::body_hash(&current.body_html)),
    })
}

#[cfg(test)]
mod handoff_tests {
    use super::{css_var_referenced, referenced_tokens};
    use crate::design::ArtifactParts;

    #[test]
    fn css_var_ref_avoids_prefix_false_match() {
        // 精确边界：紧跟 ) / , / 空白 / 结尾算命中；作为更长名的前缀不算。
        assert!(css_var_referenced("color: var(--ds-color-primary)", "--ds-color-primary"));
        assert!(css_var_referenced("var(--ds-color-primary, #fff)", "--ds-color-primary"));
        assert!(css_var_referenced("var(--ds-radius )", "--ds-radius"));
        // 容 `(` 后空白（合法 CSS，review #3/#6/#7）。
        assert!(css_var_referenced("var( --ds-color-primary )", "--ds-color-primary"));
        assert!(css_var_referenced("var(\n  --ds-space-4\n)", "--ds-space-4"));
        // --ds-color 不应被 var(--ds-color-primary) 误命中。
        assert!(!css_var_referenced("var(--ds-color-primary)", "--ds-color"));
        assert!(!css_var_referenced("no vars here", "--ds-color"));
    }

    #[test]
    fn referenced_tokens_ignores_comments_and_escapes_cells() {
        // 注释里的 token 名不算引用（review #4）。
        let parts = ArtifactParts {
            body_html: String::new(),
            css: "/* uses var(--ds-unused) here */ .x{color:var(--ds-color-primary)}".into(),
            js: String::new(),
        };
        let all = vec![
            ("--ds-color-primary".to_string(), "#2563eb".to_string()),
            ("--ds-unused".to_string(), "x".to_string()),
        ];
        let got = referenced_tokens(&parts, &all);
        assert_eq!(got, vec![("--ds-color-primary".to_string(), "#2563eb".to_string())]);
        // 表格单元格转义（review #5）。
        assert_eq!(super::md_table_cell("a|b\nc`d"), "a\\|b c'd");
    }

    #[test]
    fn referenced_tokens_filters_and_sorts() {
        let parts = ArtifactParts {
            body_html: "<div style=\"color:var(--ds-color-primary)\"></div>".into(),
            css: ".x{gap:var(--ds-space-4)}".into(),
            js: String::new(),
        };
        let all = vec![
            ("--ds-color-primary".to_string(), "#2563eb".to_string()),
            ("--ds-space-4".to_string(), "16px".to_string()),
            ("--ds-unused".to_string(), "nope".to_string()),
        ];
        let got = referenced_tokens(&parts, &all);
        assert_eq!(
            got,
            vec![
                ("--ds-color-primary".to_string(), "#2563eb".to_string()),
                ("--ds-space-4".to_string(), "16px".to_string()),
            ]
        );
    }
}

#[cfg(test)]
mod comment_tests {
    use super::{clamp_rel, sanitize_oid};

    #[test]
    fn clamp_rel_sanitizes_untrusted_coords() {
        assert_eq!(clamp_rel(0.5), 0.5);
        assert_eq!(clamp_rel(-1.0), 0.0, "负值钳到 0");
        assert_eq!(clamp_rel(2.0), 1.0, "超 1 钳到 1");
        assert_eq!(clamp_rel(f64::NAN), 0.0, "NaN → 0");
        assert_eq!(clamp_rel(f64::INFINITY), 0.0, "Inf → 0");
        assert_eq!(clamp_rel(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn sanitize_oid_rejects_negative() {
        assert_eq!(sanitize_oid(Some(5)), Some(5));
        assert_eq!(sanitize_oid(Some(0)), Some(0));
        assert_eq!(sanitize_oid(Some(-1)), None, "负 oid → 脱锚");
        assert_eq!(sanitize_oid(None), None);
    }
}
