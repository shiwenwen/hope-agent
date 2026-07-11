//! 设计空间 owner 平面业务入口（Tauri / HTTP 薄壳统一调用）。
//!
//! owner 平面 = 本机 / API key 信任，负责 UI 的项目/产物 CRUD、可视化编辑回写、
//! 导出——**不经 agent 访问检查**（见 `docs/architecture/design-space.md` §3）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::db::{
    DesignArtifact, DesignArtifactVersion, DesignCodeBinding, DesignComment, DesignDb,
    DesignProject, DesignSystemMeta,
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
/// 设计系统 id 是否安全（`[A-Za-z0-9-_]`，非空，无 `..`/路径分隔符）。**红线**：`system_id`
/// 由模型经 `create_artifact` 透传并持久化，`design_system_dir` 直接 join 之——不消毒即路径穿越
/// （读越界 `tokens.json`）。builtins 与 `slugify` 产出（`[a-z0-9-]`）均满足。
pub(crate) fn is_valid_system_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// Phase 3 返回空（内置设计系统在 Phase 2 落地，届时读 `systems/{id}/tokens.json`）。
fn resolve_tokens(system_id: Option<&str>) -> Vec<(String, String)> {
    let Some(id) = system_id.filter(|id| is_valid_system_id(id)) else {
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

/// 产物 metadata 是否标记 RTL（`dir == "rtl"`）。缺省 / 解析失败 = LTR。
fn is_rtl(metadata: Option<&str>) -> bool {
    metadata
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("dir").and_then(|d| d.as_str()).map(|d| d == "rtl"))
        .unwrap_or(false)
}

/// 渲染 + 序列化 oidmap（create/update 共用）。`rtl` 注入 `<html dir="rtl">`（post-process）。
fn render(
    kind: ArtifactKind,
    title: &str,
    parts: &ArtifactParts,
    tokens: &[(String, String)],
    rtl: bool,
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
        return Ok((renderer::apply_document_dir(html, rtl), "[]".to_string()));
    }
    // Image / Audio 是媒体产物（data-uri 内嵌），无源码 oid 可微调 → 不注 inspector/oid。
    let editable = !matches!(kind, ArtifactKind::Image | ArtifactKind::Audio);
    let (html, oidmap) = renderer::build_artifact_html(kind, title, parts, tokens, editable);
    let oidmap_json = serde_json::to_string(&oidmap)?;
    Ok((renderer::apply_document_dir(html, rtl), oidmap_json))
}

/// 渲染**干净可交付** HTML（`editable=false`，无 inspector/oid）。**Component 走 oxc 编译**（与
/// `render` 同分支），失败降级静态错误页——所有导出路径（artifact / zip / handoff）统一经此，
/// 保证导出的 `index.html` 与预览一致、可直接打开，绝不把未编译 JSX 塞进交付物。
fn render_clean(
    kind: ArtifactKind,
    title: &str,
    parts: &ArtifactParts,
    tokens: &[(String, String)],
    rtl: bool,
) -> String {
    if kind == ArtifactKind::Component {
        let html = match super::compile::compile_component(&parts.body_html) {
            Ok(js) => renderer::build_component_html(title, &js, &parts.css, tokens),
            Err(e) => {
                crate::app_warn!("design", "compile", "component export compile failed: {e}");
                renderer::build_component_error_html(title, &e.to_string())
            }
        };
        return renderer::apply_document_dir(html, rtl);
    }
    let (html, _) = renderer::build_artifact_html(kind, title, parts, tokens, false);
    renderer::apply_document_dir(html, rtl)
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
        needs_review_count: 0,
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

/// 递归复制目录树（duplicate_project 复制产物磁盘目录用）。源不存在=no-op（产物可能纯 DB 行）。
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 复制整个项目（含全部产物 + 磁盘正文 / 版本快照 / 版本行）为一个新项目。
///
/// 深拷贝：新项目行 + 逐产物新 id + `copy_dir_recursive` 拷 `artifacts/{id}/`（index.html /
/// source / versions / oidmap）+ 改写 `artifact.json` 的 id/project_id + 复制版本行（保留
/// version_number / message / origin / prompt_summary，溯源不丢）。任一产物拷贝失败即整体
/// 回滚（删新项目目录 + DB 级联删行），不留半个副本。
pub fn duplicate_project(id: &str) -> Result<DesignProject> {
    let db = open_db()?;
    let src = db
        .get_project(id)?
        .with_context(|| format!("project not found: {id}"))?;
    let ts = now();
    let new_pid = new_id();
    let project = DesignProject {
        id: new_pid.clone(),
        title: format!("{} (副本)", src.title),
        description: src.description.clone(),
        color: src.color.clone(),
        default_system_id: src.default_system_id.clone(),
        // 副本是独立项目：不继承源的 HA 项目 / 会话绑定（否则两项目抢 1:1 attach）。
        ha_project_id: None,
        session_id: None,
        agent_id: src.agent_id.clone(),
        created_at: ts.clone(),
        updated_at: ts.clone(),
        artifact_count: 0,
        needs_review_count: 0,
        metadata: src.metadata.clone(),
    };
    let new_dir = paths::design_project_dir(&new_pid)?;
    std::fs::create_dir_all(new_dir.join("artifacts"))?;
    write_atomic(
        &new_dir.join("project.json"),
        serde_json::to_string_pretty(&project)?.as_bytes(),
    )?;
    db.create_project(&project)?;

    // 逐产物深拷贝；失败整体回滚。
    let cloned = (|| -> Result<()> {
        for a in db.list_artifacts(id)? {
            let new_aid = new_id();
            let src_dir = paths::design_artifact_dir(id, &a.id)?;
            let dst_dir = paths::design_artifact_dir(&new_pid, &new_aid)?;
            copy_dir_recursive(&src_dir, &dst_dir)?;
            // 改写 artifact.json 的 id/project_id（磁盘元数据镜像须指向新副本）。
            let meta = ArtifactMeta {
                id: new_aid.clone(),
                project_id: new_pid.clone(),
                title: a.title.clone(),
                kind: a.kind.clone(),
                system_id: a.system_id.clone(),
                current_version: a.current_version,
            };
            write_atomic(
                &dst_dir.join("artifact.json"),
                serde_json::to_string_pretty(&meta)?.as_bytes(),
            )?;
            let new_artifact = DesignArtifact {
                id: new_aid.clone(),
                project_id: new_pid.clone(),
                created_at: ts.clone(),
                updated_at: ts.clone(),
                ..a.clone()
            };
            db.create_artifact(&new_artifact)?;
            // 复制版本行（保留 version_number 与溯源；create_version 忽略 id 自增）。
            for v in db.list_versions(&a.id)? {
                db.create_version(&DesignArtifactVersion {
                    id: 0,
                    artifact_id: new_aid.clone(),
                    version_number: v.version_number,
                    message: v.message,
                    critique_score: v.critique_score,
                    origin: v.origin,
                    prompt_summary: v.prompt_summary,
                    created_at: v.created_at,
                })?;
            }
        }
        Ok(())
    })();
    if let Err(e) = cloned {
        // 回滚：DB 删项目（产物 / 版本行 ON DELETE CASCADE）+ 删磁盘目录。
        let _ = db.delete_project(&new_pid);
        let _ = std::fs::remove_dir_all(&new_dir);
        return Err(e);
    }

    crate::app_info!("design", "service", "duplicate project {id} -> {new_pid}");
    emit("design:project_changed", json!({ "projectId": new_pid }));
    // 重读以带回聚合列（artifact_count / needs_review_count）。
    db.get_project(&new_pid)?
        .with_context(|| "duplicated project vanished".to_string())
}

/// 轻量改名产物（仅 `title`，不重渲染 / 不新增版本 / 不碰 source）。空标题拒绝。
pub fn rename_artifact(id: &str, title: &str) -> Result<DesignArtifact> {
    let title = title.trim();
    if title.is_empty() {
        anyhow::bail!("artifact title cannot be empty");
    }
    let db = open_db()?;
    let a = db
        .get_artifact(id)?
        .with_context(|| format!("artifact not found: {id}"))?;
    db.rename_artifact(id, title, &now())?;
    db.touch_project(&a.project_id, &now())?;
    emit(
        "design:artifact_renamed",
        json!({ "projectId": a.project_id, "artifactId": id, "title": title }),
    );
    db.get_artifact(id)?
        .with_context(|| "renamed artifact vanished".to_string())
}

/// 复制单个产物（同项目内）：新 id + 深拷贝 `artifacts/{id}/`（index.html / source / versions /
/// oidmap）+ 版本行（保留溯源）；标题加「(副本)」；位序自增追加末尾。失败整体回滚。
pub fn duplicate_artifact(id: &str) -> Result<DesignArtifact> {
    let db = open_db()?;
    // 持 artifact_lock 串行化整段拷贝（与 finalize/update/restyle/patch 互斥，防拷到源被并发
    // 改写中的半新半旧目录 / 版本行 TOCTOU，review 修复）。锁内读 src 保证状态一致。
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let src = db
        .get_artifact(id)?
        .with_context(|| format!("artifact not found: {id}"))?;
    // 生成中的产物没有稳定正文/版本行，拷出来是永远转圈的幽灵 → 拒绝（review 修复）。
    if src.status == "generating" {
        anyhow::bail!("cannot duplicate an artifact that is still generating");
    }
    let ts = now();
    let new_aid = new_id();
    let dup_title = format!("{} (副本)", src.title);
    let src_dir = paths::design_artifact_dir(&src.project_id, &src.id)?;
    let dst_dir = paths::design_artifact_dir(&src.project_id, &new_aid)?;
    let done = (|| -> Result<()> {
        copy_dir_recursive(&src_dir, &dst_dir)?;
        let meta = ArtifactMeta {
            id: new_aid.clone(),
            project_id: src.project_id.clone(),
            title: dup_title.clone(),
            kind: src.kind.clone(),
            system_id: src.system_id.clone(),
            current_version: src.current_version,
        };
        write_atomic(
            &dst_dir.join("artifact.json"),
            serde_json::to_string_pretty(&meta)?.as_bytes(),
        )?;
        let new_artifact = DesignArtifact {
            id: new_aid.clone(),
            title: dup_title.clone(),
            // 血缘：记派生来源（复用 metadata，免改表），前端展示「派生自 X」。
            metadata: merge_derived_from(src.metadata.as_deref(), &src.id, &src.title),
            created_at: ts.clone(),
            updated_at: ts.clone(),
            ..src.clone()
        };
        db.create_artifact(&new_artifact)?;
        for v in db.list_versions(&src.id)? {
            db.create_version(&DesignArtifactVersion {
                id: 0,
                artifact_id: new_aid.clone(),
                version_number: v.version_number,
                message: v.message,
                critique_score: v.critique_score,
                origin: v.origin,
                prompt_summary: v.prompt_summary,
                created_at: v.created_at,
            })?;
        }
        Ok(())
    })();
    if let Err(e) = done {
        let _ = db.delete_artifact(&new_aid); // 版本行 ON DELETE CASCADE
        let _ = std::fs::remove_dir_all(&dst_dir);
        return Err(e);
    }
    db.touch_project(&src.project_id, &ts)?;
    crate::app_info!("design", "service", "duplicate artifact {id} -> {new_aid}");
    db.get_artifact(&new_aid)?
        .with_context(|| "duplicated artifact vanished".to_string())
}

/// 重排 project 内产物页面顺序（用户拖动）：按 `ordered_ids` 下标写 `position`。
pub fn reorder_artifacts(project_id: &str, ordered_ids: &[String]) -> Result<()> {
    let db = open_db()?;
    db.reorder_artifacts(project_id, ordered_ids)?;
    db.touch_project(project_id, &now())?;
    Ok(())
}

// ── 页面分组文件夹（OD path-based 模型）──────────────────────────────

/// 文件夹路径合法化：去空段、拒 `.`/`..`、每段裁剪 → 斜杠路径（空 = 根）。
fn sanitize_folder(path: &str) -> String {
    path.split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect::<Vec<_>>()
        .join("/")
}

/// 项目内全部文件夹路径（产物 folder ∪ 持久化空文件夹，含祖先，已排序）。
pub fn list_folders(project_id: &str) -> Result<Vec<String>> {
    open_db()?.list_folder_paths(project_id)
}

/// 新建（空）文件夹。`name` 可含斜杠建嵌套。
pub fn create_folder(project_id: &str, name: &str) -> Result<()> {
    let path = sanitize_folder(name);
    if path.is_empty() {
        anyhow::bail!("folder name cannot be empty");
    }
    let db = open_db()?;
    db.create_folder(project_id, &path, &now())?;
    emit("design:folders_changed", json!({ "projectId": project_id }));
    Ok(())
}

/// 删文件夹：**把其中页面移到根（不删页面，防误删——本项 wired-by-us 的安全语义）** + 删文件夹记录。
pub fn delete_folder(project_id: &str, path: &str) -> Result<()> {
    let path = sanitize_folder(path);
    if path.is_empty() {
        anyhow::bail!("invalid folder path");
    }
    let db = open_db()?;
    db.detach_artifacts_from_folder(project_id, &path, &now())?;
    db.delete_folder_records(project_id, &path)?;
    db.touch_project(project_id, &now())?;
    emit("design:folders_changed", json!({ "projectId": project_id }));
    Ok(())
}

/// 文件夹改名/移动（前缀替换到 `to`，同时改其中页面 folder 与子文件夹记录）。
pub fn rename_folder(project_id: &str, from: &str, to: &str) -> Result<()> {
    let from = sanitize_folder(from);
    let to = sanitize_folder(to);
    if from.is_empty() || to.is_empty() {
        anyhow::bail!("invalid folder path");
    }
    if from == to {
        return Ok(());
    }
    // 拒绝把文件夹移进自己的子孙（`to` 在 `from/` 之下）——否则 exact-match 分支先把
    // `from` 改成 `to`，紧接 subpath 分支又用 `from/%` 去匹配已改过的行，前缀替换二次
    // 处理产生错乱路径（review MED）。同名不同层的合法移动不受影响。
    if to.starts_with(&format!("{from}/")) {
        anyhow::bail!("cannot move a folder into its own descendant");
    }
    let db = open_db()?;
    db.rename_folder_prefix(project_id, &from, &to, &now())?;
    db.touch_project(project_id, &now())?;
    emit("design:folders_changed", json!({ "projectId": project_id }));
    Ok(())
}

/// 把页面移到某文件夹（`folder` 空 = 移到根）。
pub fn move_artifact_to_folder(id: &str, folder: &str) -> Result<DesignArtifact> {
    let folder = sanitize_folder(folder);
    let db = open_db()?;
    let a = db
        .get_artifact(id)?
        .with_context(|| format!("artifact not found: {id}"))?;
    db.set_artifact_folder(id, &folder, &now())?;
    db.touch_project(&a.project_id, &now())?;
    emit(
        "design:artifact_moved",
        json!({ "projectId": a.project_id, "artifactId": id, "folder": folder }),
    );
    db.get_artifact(id)?
        .with_context(|| "moved artifact vanished".to_string())
}

pub fn list_projects() -> Result<Vec<DesignProject>> {
    open_db()?.list_projects()
}

/// agent 侧：解析当前会话的设计项目（取最近一个，无则新建草稿项目）。
///
/// 设计空间对话（`kind='design'` 会话）经 `design_chat_threads` 锚到用户当前
/// 打开的项目——优先命中锚定项目，让「跟 AI 说改这个」落到正确项目而不是新建
/// 草稿。锚表在 sessions.db、无 SessionDB（ACP）时静默回落原有按 session 查逻辑。
pub fn get_or_create_session_project(
    session_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<DesignProject> {
    if let Some(sid) = session_id {
        if let Ok(Some(pid)) = crate::design::threads::project_for_session(sid) {
            if let Some(p) = open_db()?.get_project(&pid)? {
                return Ok(p);
            }
        }
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

/// Promote a freshly-created session into a design chat thread anchored to a
/// project (`kind='design'` + `design_chat_threads` row). Called from the `chat`
/// command's auto-create branch when `tool_scope == "design"`. Best-effort:
/// mirrors `knowledge::service::mark_session_as_kb_thread` — the thread row is
/// created FIRST so a failure leaves a usable (if unlisted) regular session
/// rather than a row-less hidden zombie.
pub fn mark_session_as_design_thread(session_id: &str, project_id: &str) {
    let Some(db) = crate::globals::get_session_db() else {
        return;
    };
    if let Err(e) = crate::design::threads::create_thread(session_id, project_id) {
        crate::app_warn!(
            "design",
            "thread_mint",
            "create_thread failed for {}: {}",
            session_id,
            e
        );
        return;
    }
    if let Err(e) = db.set_session_kind(session_id, crate::session::SessionKind::Design) {
        crate::app_warn!(
            "design",
            "thread_mint",
            "set_session_kind failed for {} (thread row kept): {}",
            session_id,
            e
        );
    }
}

/// Default-load target: `SessionMeta` of the most-recently-active chat thread for
/// a design project. `None` when the project has no prior conversation (panel
/// shows the empty starter state).
pub fn design_chat_thread_latest(project_id: &str) -> Result<Option<crate::session::SessionMeta>> {
    let Some(sid) = crate::design::threads::latest_thread_for_project(project_id)? else {
        return Ok(None);
    };
    let Some(db) = crate::globals::get_session_db() else {
        return Ok(None);
    };
    db.get_session(&sid)
}

/// History picker: a page of design chat threads for a project, newest-active
/// first. `query` (when non-empty) FTS-filters by message content; `limit`
/// (default 50, clamped 1..=200) + `offset` paginate.
pub fn design_chat_threads_list(
    project_id: &str,
    query: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<crate::design::DesignChatThread>> {
    crate::design::threads::list_threads(project_id, query, limit, offset)
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
    // Tear down the (otherwise hidden) `kind='design'` chat sessions anchored to
    // this project BEFORE the project row goes away — collect first, then delete
    // each session (which cascades its `design_chat_threads` row + messages).
    // The anchor table lives in sessions.db (no cross-db FK), so this cleanup is
    // explicit rather than an ON DELETE CASCADE.
    if let Some(sdb) = crate::globals::get_session_db() {
        if let Ok(session_ids) = crate::design::threads::thread_session_ids(id) {
            for sid in session_ids {
                let _ = sdb.delete_session(&sid);
            }
        }
    }
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

#[derive(Debug, Clone, Default, Deserialize)]
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
    /// image 形态：参考图路径 / URL（agent 面图生图入口）。每项经 `image_generate::load_input_images`
    /// 加载（本地路径 / data: / http(s) 走 SSRF），≤5 张，坏项跳过不阻断。与 `reference_image_b64`
    /// 叠加（owner 面用 b64、agent 面用 paths）。
    #[serde(default)]
    pub reference_image_paths: Option<Vec<String>>,
    /// 选定的 recipe（模板）id：非媒体形态生成时，用该 recipe 的 guidance/scenario 作 KIND
    /// GUIDANCE（选不同模板产出结构可辨差异）。缺省 / 不匹配 kind → 回退该 kind 首个内置 recipe。
    #[serde(default)]
    pub recipe_id: Option<String>,
    /// image 形态：比例提示（"1:1" / "16:9" / "9:16"…）透传给生图 provider。B0-4。
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    /// audio 形态：music / sfx 目标时长（秒）透传给音频 provider。B8-2。
    #[serde(default)]
    pub audio_duration_secs: Option<f64>,
    /// 新页面落入的文件夹（页面分组）：斜杠路径，缺省 = 根。OD「新文件落 currentDir」的对应。
    #[serde(default)]
    pub folder: Option<String>,
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
        // B0-4：透传比例 + 参考图（有 reference_image_b64 → 图生图/编辑，此前被静默丢弃）。
        use base64::Engine;
        let mut input_images = Vec::new();
        if let Some(b64) = input
            .reference_image_b64
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                Ok(data) => input_images.push(crate::tools::image_generate::InputImage {
                    data,
                    mime: input
                        .reference_image_mime
                        .clone()
                        .unwrap_or_else(|| "image/png".to_string()),
                }),
                Err(e) => crate::app_warn!(
                    "design",
                    "image",
                    "reference image base64 decode failed, ignoring: {e}"
                ),
            }
        }
        // agent 面：参考图路径 / URL → 加载（SSRF-gated、坏项跳过），叠加到 input_images（总量由
        // load_input_images 钳 ≤ MAX_INPUT_IMAGES；此处再钳一次防 b64 + paths 叠加超限）。
        if let Some(paths) = input
            .reference_image_paths
            .as_deref()
            .filter(|p| !p.is_empty())
        {
            match crate::tools::image_generate::load_input_images(paths).await {
                Ok(mut loaded) => {
                    let room = 5usize.saturating_sub(input_images.len());
                    loaded.truncate(room);
                    input_images.append(&mut loaded);
                }
                Err(e) => crate::app_warn!(
                    "design",
                    "image",
                    "reference image paths load failed, ignoring: {e}"
                ),
            }
        }
        let opts = super::image::ImageGenOptions {
            aspect_ratio: input.aspect_ratio.clone().filter(|s| !s.trim().is_empty()),
            input_images,
            mask: None,
        };
        let parts = super::image::generate_image_parts(&prompt, &input.title, &opts).await?;
        input.body_html = Some(parts.body_html);
    } else if body_empty && input.kind == "audio" {
        // audio 形态：prompt → 音频合成（TTS/音乐/音效）→ 内嵌 data-uri <audio> 播放器。
        let prompt = input
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| input.title.clone());
        let parts =
            super::audio::generate_audio_parts(&prompt, &input.title, input.audio_duration_secs)
                .await?;
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
            let recipe_id = input.recipe_id.clone();
            match super::generate::generate_design_parts(
                &brief,
                kind,
                &system_md,
                &tokens,
                recipe_id.as_deref(),
            )
            .await
            {
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

/// 品牌包批量生成上限（防一次拉起过多模型调用）。
const MAX_BRAND_PACK_KINDS: usize = 6;

/// 合法的品牌包形态（媒体形态 image/audio/component 不进批量文案生成）。
fn is_brand_pack_kind(kind: &str) -> bool {
    matches!(
        kind,
        "web" | "mobile" | "deck" | "dashboard" | "poster" | "document" | "email"
    )
}

/// 归一化品牌包形态：滤非法 + 去重（保序）+ 钳数量。纯函数，便于单测。
fn normalize_brand_pack_kinds(kinds: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    kinds
        .into_iter()
        .filter(|k| is_brand_pack_kind(k) && seen.insert(k.clone()))
        .take(MAX_BRAND_PACK_KINDS)
        .collect()
}

/// 从**一个 brief**批量生成一组**共享同一设计系统**的协调产物（多产物品牌包）。owner 平面。
/// 每个 kind 复用现成 `create_artifact_generating`（含降级），任一失败不阻断其余、返回成功者。
/// **顺序生成**（避免并发打满模型），全失败则 `bail!`。
pub async fn generate_brand_pack(
    project_id: &str,
    brief: &str,
    kinds: Vec<String>,
    system_id: Option<String>,
    folder: Option<String>,
) -> Result<Vec<DesignArtifact>> {
    let brief = brief.trim();
    if brief.is_empty() {
        anyhow::bail!("品牌包需要一句设计 brief");
    }
    // 去重 + 过滤合法 kind + 保序 + 钳数量。
    let kinds = normalize_brand_pack_kinds(kinds);
    if kinds.is_empty() {
        anyhow::bail!("品牌包需要至少一个合法形态（web/mobile/deck/dashboard/poster/document/email）");
    }
    let mut out = Vec::new();
    let mut last_err: Option<anyhow::Error> = None;
    let total = kinds.len();
    for (i, kind) in kinds.iter().enumerate() {
        // 逐件进度：前端据此把「一直转圈」换成「正在生成 演示（2/3）」。
        emit(
            "design:brand_pack_progress",
            json!({
                "projectId": project_id,
                "index": i + 1,
                "total": total,
                "kind": kind,
                "done": false,
            }),
        );
        let input = CreateArtifactInput {
            project_id: project_id.to_string(),
            title: format!("{brief}").chars().take(40).collect(),
            kind: kind.clone(),
            system_id: system_id.clone(),
            prompt: Some(brief.to_string()),
            folder: folder.clone(),
            ..Default::default()
        };
        match create_artifact_generating(input).await {
            Ok(a) => out.push(a),
            Err(e) => {
                crate::app_warn!(
                    "design",
                    "generate",
                    "brand pack kind {kind} failed: {e}"
                );
                last_err = Some(e);
            }
        }
    }
    emit(
        "design:brand_pack_progress",
        json!({ "projectId": project_id, "index": total, "total": total, "done": true }),
    );
    if out.is_empty() {
        return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("品牌包生成全部失败")));
    }
    crate::app_info!(
        "design",
        "generate",
        "brand pack generated {}/{} artifacts for project {project_id}",
        out.len(),
        kinds.len()
    );
    Ok(out)
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
fn resolve_self_check(
    existing_meta: Option<&str>,
    body_html: &str,
    css: &str,
) -> (String, Option<String>) {
    let enabled = crate::config::cached_config().design.self_check;
    let verdict = if enabled {
        super::selfcheck::evaluate(body_html, css)
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
        resolve_self_check(None, &parts.body_html, &parts.css)
    } else {
        ("ready".to_string(), None)
    };

    // 磁盘落地：artifact_dir / index.html / source/ / versions/1 / artifact.json
    let dir = paths::design_artifact_dir(&input.project_id, &artifact_id)?;
    let tokens = resolve_tokens(system_id.as_deref());
    let (html, oidmap_json) = render(kind, &title, &parts, &tokens, false)?;
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
        folder: input.folder.clone().unwrap_or_default(),
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
            // 带正文创建=模型/工具产出（ai）；空白起草=用户手动建（manual）。
            origin: Some(if had_body { "ai" } else { "manual" }.to_string()),
            prompt_summary: None,
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
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()))?;
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

/// 往产物 metadata 合并 `derivedFrom`（血缘来源），保留其它键。空/非对象 metadata 从 `{}` 起。
fn merge_derived_from(existing: Option<&str>, from_id: &str, from_title: &str) -> Option<String> {
    let mut meta: serde_json::Value = existing
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    meta["derivedFrom"] = serde_json::json!({ "id": from_id, "title": from_title });
    Some(meta.to_string())
}

/// 保存 deck 演讲者备注（按 slide 顺序，存产物 `metadata.presenterNotes`）。owner 平面。
/// 尾部空串保留位序（第 3 页有备注、1/2 空 → `["","","note"]`）。
pub fn set_presenter_notes(artifact_id: &str, notes: Vec<String>) -> Result<()> {
    let db = open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    let mut meta: serde_json::Value = a
        .metadata
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    meta["presenterNotes"] = serde_json::json!(notes);
    db.update_artifact_metadata(artifact_id, Some(&meta.to_string()), &now())?;
    Ok(())
}

/// 从 image 产物 body 提取内嵌图片（`<img src="data:image/…;base64,…">`）→ (bytes, mime)。
fn extract_image_from_body(body: &str) -> Option<(Vec<u8>, String)> {
    use base64::Engine;
    let start = body.find("data:image/")?;
    let rest = &body[start + "data:".len()..];
    let semi = rest.find(";base64,")?;
    let mime = rest[..semi].to_string();
    let after = &rest[semi + ";base64,".len()..];
    let end = after.find(['"', '\'', ')']).unwrap_or(after.len());
    let b64 = after[..end].trim();
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some((bytes, mime))
}

/// inpaint：对 image 产物按蒙版局部重绘（`mask_b64` = PNG，透明/涂画区=重绘区）。owner 平面。
/// 提取产物当前图 + 蒙版 → OpenAI `/images/edits`（走 `image_generate` 栈）→ 落新版本。
/// 需产物为 image 形态且能提取内嵌图；生图失败/无 provider 由底层报错。
pub async fn inpaint_image_artifact(
    id: &str,
    prompt: &str,
    mask_b64: &str,
) -> Result<DesignArtifact> {
    use base64::Engine;
    let (a, kind, dir, existing_body) = {
        let db = open_db()?;
        let a = db.get_artifact(id)?.context("artifact not found")?;
        if a.kind != "image" {
            anyhow::bail!("仅 image 形态产物支持蒙版重绘");
        }
        let kind = ArtifactKind::from_str(&a.kind).context("bad kind")?;
        let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
        let body = read_source(&dir)?.body_html;
        (a, kind, dir, body)
    };
    let (img_bytes, img_mime) =
        extract_image_from_body(&existing_body).context("产物内未找到可重绘的内嵌图片")?;
    let mask_bytes = base64::engine::general_purpose::STANDARD
        .decode(mask_b64.trim())
        .context("mask base64 decode failed")?;
    let prompt = if prompt.trim().is_empty() {
        a.title.clone()
    } else {
        prompt.trim().to_string()
    };
    let opts = super::image::ImageGenOptions {
        aspect_ratio: None,
        input_images: vec![crate::tools::image_generate::InputImage {
            data: img_bytes,
            mime: img_mime,
        }],
        mask: Some(mask_bytes),
    };
    let parts = super::image::generate_image_parts(&prompt, &a.title, &opts).await?;

    // 落新版本（image 形态：render editable=false、无 oid）。持锁串行化。
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let db = open_db()?;
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()))?;
    write_working(&dir, &html, &parts, &oidmap_json)?;
    let next = a.current_version + 1;
    write_version_snapshot(&dir, next, &html, &parts, &oidmap_json)?;
    let ts = now();
    db.update_artifact_review(&a.id, None, &a.status, Some(next), a.metadata.as_deref(), &ts)?;
    db.create_version(&DesignArtifactVersion {
        id: 0,
        artifact_id: a.id.clone(),
        version_number: next,
        message: Some("Inpaint".to_string()),
        critique_score: None,
        origin: Some("ai".to_string()),
        prompt_summary: Some(crate::truncate_utf8(&prompt, 120).to_string()),
        created_at: ts.clone(),
    })?;
    let keep = crate::config::cached_config()
        .design
        .max_versions_per_artifact
        .max(1);
    let _ = db.cleanup_old_versions(&a.id, keep);
    if let Ok(remaining) = db.list_versions(&a.id) {
        let keep_set: std::collections::HashSet<i64> =
            remaining.iter().map(|v| v.version_number).collect();
        prune_version_dirs_to_db(&dir, &keep_set);
    }
    db.touch_project(&a.project_id, &ts)?;
    emit("design:reload", json!({ "artifactId": a.id }));
    db.get_artifact(&a.id)?.context("artifact gone after inpaint")
}

/// 设置产物文本方向（RTL/LTR，存 `metadata.dir`）并**立即重渲染 working index.html**（RTL 在
/// 渲染期注入 `<html dir="rtl">`）。owner 平面。媒体形态（image/audio）无 `<html>` 外壳 → 拒。
pub fn set_artifact_dir(id: &str, rtl: bool) -> Result<DesignArtifact> {
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let db = open_db()?;
    let a = db.get_artifact(id)?.context("artifact not found")?;
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    if matches!(kind, ArtifactKind::Image | ArtifactKind::Audio) {
        anyhow::bail!("媒体产物无文本方向可设");
    }
    // 合并 metadata.dir（rtl 写入 / ltr 移除键，保留其它键）。
    let mut meta: serde_json::Value = a
        .metadata
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    if rtl {
        meta["dir"] = serde_json::json!("rtl");
    } else if let Some(o) = meta.as_object_mut() {
        o.remove("dir");
    }
    let meta_str = meta.to_string();
    // 重渲染 working（导出/分享读 live metadata 各自反映，无需改历史版本快照）。
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens, rtl)?;
    write_working(&dir, &html, &parts, &oidmap_json)?;
    let ts = now();
    db.update_artifact_review(&a.id, None, &a.status, None, Some(&meta_str), &ts)?;
    db.touch_project(&a.project_id, &ts)?;
    emit("design:reload", json!({ "artifactId": a.id }));
    db.get_artifact(&a.id)?.context("artifact gone after set dir")
}

/// 页面级样式标记块前缀：`/*ds-page*/body{...}` 追加到用户 CSS 末尾（后出现的规则胜出）。
/// 与 oid 元素级微调正交——页面级不走 patch/oid，直接改 CSS 里这一确定性标记块。
const PAGE_STYLE_MARKER: &str = "/*ds-page*/";

fn is_safe_css_prop(p: &str) -> bool {
    !p.is_empty() && p.len() <= 40 && p.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

fn sanitize_css_value(v: &str) -> String {
    v.chars()
        .filter(|c| !matches!(c, '{' | '}' | '<' | ';' | '\\'))
        .take(200)
        .collect::<String>()
        .trim()
        .to_string()
}

/// 去掉已有 page-style 标记块（幂等，供重写）。
fn strip_page_style(css: &str) -> String {
    if let Some(idx) = css.find(PAGE_STYLE_MARKER) {
        let after = &css[idx..];
        if let Some(open) = after.find('{') {
            if let Some(close) = after[open..].find('}') {
                let end = idx + open + close + 1;
                let mut out = String::new();
                out.push_str(css[..idx].trim_end());
                if !css[end..].trim().is_empty() {
                    out.push('\n');
                    out.push_str(css[end..].trim_start());
                }
                return out.trim().to_string();
            }
        }
    }
    css.to_string()
}

/// 应用页面级样式：strip 旧标记块 + 追加新 `body{...}`（空值属性=移除；全空=只 strip）。纯函数。
fn apply_page_style_css(css: &str, props: &[(String, String)]) -> String {
    let base = strip_page_style(css);
    let decls: String = props
        .iter()
        .filter(|(k, _)| is_safe_css_prop(k))
        .filter_map(|(k, v)| {
            let sv = sanitize_css_value(v);
            (!sv.is_empty()).then(|| format!("{k}:{sv};"))
        })
        .collect();
    if decls.is_empty() {
        return base;
    }
    if base.is_empty() {
        format!("{PAGE_STYLE_MARKER}body{{{decls}}}")
    } else {
        format!("{base}\n{PAGE_STYLE_MARKER}body{{{decls}}}")
    }
}

/// 页面级样式编辑（背景 / 文字色 / 最大宽度 / 基础字体等，作用于 `body`）。owner 平面。
/// 与 oid 元素微调正交：只改 CSS 里的确定性标记块，落新版本 + 重渲染 + `design:reload`。
/// 媒体 / component 无用户 CSS 编辑面 → 拒。
pub fn patch_page_style(id: &str, props: Vec<(String, String)>) -> Result<DesignArtifact> {
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let db = open_db()?;
    let a = db.get_artifact(id)?.context("artifact not found")?;
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    if matches!(
        kind,
        ArtifactKind::Image | ArtifactKind::Audio | ArtifactKind::Component
    ) {
        anyhow::bail!("该形态无页面级样式编辑面");
    }
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let existing = read_source(&dir)?;
    let parts = ArtifactParts {
        body_html: existing.body_html,
        css: apply_page_style_css(&existing.css, &props),
        js: existing.js,
    };
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()))?;
    write_working(&dir, &html, &parts, &oidmap_json)?;
    let next = a.current_version + 1;
    write_version_snapshot(&dir, next, &html, &parts, &oidmap_json)?;
    let ts = now();
    db.update_artifact_review(&a.id, None, &a.status, Some(next), a.metadata.as_deref(), &ts)?;
    db.create_version(&DesignArtifactVersion {
        id: 0,
        artifact_id: a.id.clone(),
        version_number: next,
        message: Some("Page style".to_string()),
        critique_score: None,
        origin: Some("manual".to_string()),
        prompt_summary: None,
        created_at: ts.clone(),
    })?;
    let keep = crate::config::cached_config()
        .design
        .max_versions_per_artifact
        .max(1);
    let _ = db.cleanup_old_versions(&a.id, keep);
    if let Ok(remaining) = db.list_versions(&a.id) {
        let keep_set: std::collections::HashSet<i64> =
            remaining.iter().map(|v| v.version_number).collect();
        prune_version_dirs_to_db(&dir, &keep_set);
    }
    db.touch_project(&a.project_id, &ts)?;
    emit("design:reload", json!({ "artifactId": a.id }));
    db.get_artifact(&a.id)?
        .context("artifact gone after page style")
}

/// 拖入导入上限（单张图，与部署单文件量级一致的保守值）。
const MAX_IMPORT_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// 拖入导入：把一张图片字节内嵌成 `image` 形态产物（自包含 data-uri）。owner 平面。
/// `mime` 必须是 `image/*`；超限拒。落入 `folder`（缺省根）。
pub fn import_image_artifact(
    project_id: &str,
    title: &str,
    mime: &str,
    bytes: &[u8],
    folder: Option<String>,
) -> Result<DesignArtifact> {
    if !mime.trim().to_ascii_lowercase().starts_with("image/") {
        anyhow::bail!("仅支持导入图片文件（image/*），收到 mime={mime}");
    }
    if bytes.is_empty() {
        anyhow::bail!("空文件");
    }
    if bytes.len() > MAX_IMPORT_IMAGE_BYTES {
        anyhow::bail!(
            "图片过大（{} MB，上限 {} MB）",
            bytes.len() / 1024 / 1024,
            MAX_IMPORT_IMAGE_BYTES / 1024 / 1024
        );
    }
    let title = if title.trim().is_empty() {
        "Imported image".to_string()
    } else {
        title.trim().to_string()
    };
    let body_html = super::image::image_body_from_bytes(bytes, mime, &title);
    create_artifact(CreateArtifactInput {
        project_id: project_id.to_string(),
        title,
        kind: "image".to_string(),
        body_html: Some(body_html),
        folder,
        ..Default::default()
    })
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
        folder: input.folder.clone().unwrap_or_default(),
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

/// 反-slop 自查复查动作（owner 平面，B0-2）：
/// - `"recheck"`：对**当前磁盘正文**重跑确定性自查——用户改过后命中即清、仍命中则更新 detail；
/// - `"dismiss"`：用户判定无碍，强制 `ready` + 剥 `selfCheck` 键（保留其它 metadata）。
///
/// 只动 `status` + `selfCheck` 键，不碰正文/版本；返回更新后的产物。emit `design:artifact_ready`
/// 让库列表刷新徽章。
pub fn review_artifact(id: &str, action: &str) -> Result<DesignArtifact> {
    let db = open_db()?;
    let a = db
        .get_artifact(id)?
        .with_context(|| format!("artifact not found: {id}"))?;
    let (status, metadata) = if action == "dismiss" {
        (
            "ready".to_string(),
            super::selfcheck::merge_into_metadata(a.metadata.as_deref(), None),
        )
    } else {
        // recheck：读当前磁盘正文重跑自查（未开自查 → 归 ready 清键）。
        let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
        let parts = read_source(&dir)?;
        resolve_self_check(a.metadata.as_deref(), &parts.body_html, &parts.css)
    };
    db.update_artifact_review(id, None, &status, None, metadata.as_deref(), &now())?;
    crate::app_info!("design", "service", "review artifact {} -> {}", id, status);
    emit(
        "design:artifact_ready",
        json!({ "projectId": a.project_id, "artifactId": id }),
    );
    db.get_artifact(id)?
        .with_context(|| format!("artifact not found after review: {id}"))
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
    prompt_summary: Option<&str>,
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
    let (html, oidmap_json) = render(kind, &a.title, parts, &tokens, is_rtl(a.metadata.as_deref()))?;
    write_working(&dir, &html, parts, &oidmap_json)?;
    write_version_snapshot(&dir, a.current_version, &html, parts, &oidmap_json)?;

    let ts = now();
    // 生成定稿：对模型产出的正文跑确定性自查，命中翻 needs_review + 写 selfCheck 元数据。
    let (status, self_check_meta) = resolve_self_check(a.metadata.as_deref(), &parts.body_html, &parts.css);
    db.update_artifact_review(id, None, &status, None, self_check_meta.as_deref(), &ts)?;
    // 壳未建版本行——定稿补首版（避免 list_versions 为空）。
    db.create_version(&DesignArtifactVersion {
        id: 0,
        artifact_id: a.id.clone(),
        version_number: a.current_version,
        message: Some("Generated".to_string()),
        critique_score: None,
        origin: Some("ai".to_string()),
        prompt_summary: prompt_summary.map(|s| crate::truncate_utf8(s, 2000).to_string()),
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
#[allow(clippy::too_many_arguments)]
pub async fn stream_generate_artifact(
    artifact_id: String,
    project_id: String,
    brief: String,
    kind: ArtifactKind,
    system_md: String,
    tokens: BTreeMap<String, String>,
    recipe_id: Option<String>,
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
        recipe_id.as_deref(),
        &cancel,
        &on_snapshot,
    )
    .await;

    // 已取消（产物被删 / regenerate）：不 finalize（可能写已删目录）、不 emit。
    if cancel.load(Ordering::SeqCst) {
        return;
    }

    match result {
        Ok(parts) => match finalize_generating_artifact(&artifact_id, &parts, Some(&brief)) {
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
    let recipe_id = input.recipe_id.clone();
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
            recipe_id,
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

/// 读取某历史版本快照的 `index.html`（历史面板右栏 iframe srcdoc 预览用）。快照本就是
/// 自包含产物 HTML（bridge dormant，未激活不启用），直接喂 sandbox iframe 即渲染，无需重渲。
pub fn get_artifact_version_html(artifact_id: &str, version_number: i64) -> Result<String> {
    let db = open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let f = dir
        .join("versions")
        .join(version_number.to_string())
        .join("index.html");
    match std::fs::read_to_string(&f) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("version {version_number} snapshot not found")
        }
        Err(e) => Err(anyhow::anyhow!("read version index.html: {e}")),
    }
}

// ── Shares（B7-1 只读分享）────────────────────────────────────────

/// 分享 token（32 hex，不可猜，url-safe）。
fn new_share_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 建/取产物只读分享 token（幂等：已分享复用同一链接）。owner 平面。
pub fn create_share(artifact_id: &str) -> Result<String> {
    let db = open_db()?;
    db.get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let token = db.upsert_share(artifact_id, &new_share_token(), &now())?;
    crate::app_info!(
        "design",
        "share",
        "created share for artifact {artifact_id}"
    );
    Ok(token)
}

/// 产物当前分享 token（GUI 显示已有链接；无则 None）。
pub fn share_token_for_artifact(artifact_id: &str) -> Result<Option<String>> {
    open_db()?.share_token_for_artifact(artifact_id)
}

/// 产物部署历史（最新在前，最多 20 条）。
pub fn list_deployments(artifact_id: &str) -> Result<Vec<super::db::DeploymentRecord>> {
    open_db()?.list_deployments(artifact_id, 20)
}

/// 撤销某产物的分享（按产物 id，与 owner 路由 `/artifacts/{id}/share` 对齐，避开公开
/// `/share/{token}` 路径）。无分享返回 false。
pub fn revoke_share_for_artifact(artifact_id: &str) -> Result<bool> {
    let db = open_db()?;
    match db.share_token_for_artifact(artifact_id)? {
        Some(tok) => {
            let ok = db.delete_share(&tok)?;
            if ok {
                crate::app_info!(
                    "design",
                    "share",
                    "revoked share for artifact {artifact_id}"
                );
            }
            Ok(ok)
        }
        None => Ok(false),
    }
}

/// 产物当前**干净**自包含快照 HTML（`render_clean`，无 inspector bridge / oid，导出态一致）。
/// 分享公开路由 + CF 部署（B7-1/7-2）共用单一来源。
pub fn render_clean_html_for_artifact(a: &DesignArtifact) -> Result<String> {
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;
    let tokens = resolve_tokens(a.system_id.as_deref());
    Ok(render_clean(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref())))
}

/// owner 平面：对产物跑确定性多镜头质量审查（a11y / 内容 / 语义），返回结构化发现。
/// media 形态（image/audio）无源码 HTML 可查 → 空。
pub fn quality_review_artifact(id: &str) -> Result<Vec<super::selfcheck::ReviewFinding>> {
    let a = open_db()?.get_artifact(id)?.context("artifact not found")?;
    if matches!(a.kind.as_str(), "image" | "audio") {
        return Ok(Vec::new());
    }
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;
    Ok(super::selfcheck::review(&parts.body_html, &parts.css))
}

/// 公开路由：token → 产物干净快照 HTML。token 查不到 / 产物已删 → None（路由回 404）。
pub fn render_share_html(token: &str) -> Result<Option<String>> {
    let db = open_db()?;
    let Some(artifact_id) = db.resolve_share(token)? else {
        return Ok(None);
    };
    let Some(a) = db.get_artifact(&artifact_id)? else {
        return Ok(None);
    };
    Ok(Some(render_clean_html_for_artifact(&a)?))
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

/// 设计 agent 侧「看当前产物源码」的载荷：body **注 `data-ds-oid`**（agent 据此定位元素 +
/// 用 `edit_element` 就地精改）+ css / js 原样 + body_hash（可作 `edit_element` 的
/// stale-write 守卫）。让 agent 像 open-design 那样「读源码→精确改一处」而非凭记忆整段重造。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentArtifactSource {
    /// body.html：oid-可编辑 kind 注入 `data-ds-oid`（agent 定位用），否则原样。
    pub body: String,
    pub css: String,
    pub js: String,
    /// 当前 body BLAKE3（传给 `edit_element` 的 `expected_body_hash` 守 stale-write）。
    pub body_hash: String,
    /// 该 kind 是否支持 `edit_element` 就地微调（image/audio/component 为 false）。
    pub oid_editable: bool,
}

/// 读取产物源码给 agent（body 注 oid）。见 [`AgentArtifactSource`]。
pub fn get_artifact_source_for_agent(id: &str) -> Result<Option<AgentArtifactSource>> {
    let Some(a) = open_db()?.get_artifact(id)? else {
        return Ok(None);
    };
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;
    let body_hash = patch::body_hash(&parts.body_html);
    let oid_editable = ArtifactKind::from_str(&a.kind).is_some_and(ArtifactKind::supports_oid_edit);
    // 仅可编辑 kind 注 oid——component 源是 JSX、image/audio 是媒体，注 data-ds-oid 无意义/有害。
    let body = if oid_editable {
        patch::annotate(&parts.body_html).0
    } else {
        parts.body_html
    };
    Ok(Some(AgentArtifactSource {
        body,
        css: parts.css,
        js: parts.js,
        body_hash,
        oid_editable,
    }))
}

/// 渲染版本标记是否已在 head（`<body>` 之前）出现——只扫 head 区，避免用户 body HTML 里恰好
/// 出现同串导致 stale 文件被误判为 fresh 而永不自愈。
fn head_contains_marker(html: &str, marker: &str) -> bool {
    html.split("<body").next().unwrap_or(html).contains(marker)
}

/// 打开产物时自愈：若磁盘 `index.html` 的渲染版本落后当前 `RENDER_VERSION`（如 inspector bridge
/// 升级），用当前 renderer 从磁盘源重渲染 `index.html` + `oidmap.json`（**内容不变、不新增版本、
/// 不动 source**），使编辑工具层升级无需用户重编辑即对老产物生效——bridge 烧死在 index.html，
/// 否则老产物永远用旧工具。仅 `ready` / `needs_review` 的可编辑 kind 执行；已最新或不适用即 no-op。
/// 返回是否发生重渲染（前端据此决定是否重载 iframe）。
///
/// **并发安全（review 修复）**：整段 RMW 持 `artifact_lock`（与 update/restyle/finalize/patch 互斥）。
/// 锁内**双检**重读磁盘标记——由于每个写者（`render` 均烧当前 `data-ds-r`）落盘即带最新标记，若
/// 并发编辑已在锁外快路径窗口写了新内容，锁内重读即见 fresh → no-op，绝不用旧源覆盖新编辑。
pub fn ensure_artifact_render_fresh(id: &str) -> Result<bool> {
    let Some(a) = open_db()?.get_artifact(id)? else {
        return Ok(false);
    };
    // ready + needs_review 都是稳定可编辑态（有 bridge、可微调/批注）；generating/planned/failed 不碰。
    if !matches!(a.status.as_str(), "ready" | "needs_review") {
        return Ok(false);
    }
    let Some(kind) = ArtifactKind::from_str(&a.kind) else {
        return Ok(false);
    };
    // Image/Audio 无 inspector bridge、Component 是编译产物无 oid → 无 bridge 可自愈。
    if matches!(
        kind,
        ArtifactKind::Image | ArtifactKind::Audio | ArtifactKind::Component
    ) {
        return Ok(false);
    }
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let index_path = dir.join("index.html");
    let marker = format!("data-ds-r=\"{}\"", renderer::RENDER_VERSION);
    // 快路径：锁外读一次，已最新直接 no-op（避免无谓锁竞争，绝大多数打开走这里）。
    if head_contains_marker(
        &std::fs::read_to_string(&index_path).unwrap_or_default(),
        &marker,
    ) {
        return Ok(false);
    }
    // 慢路径：持锁串行化，锁内双检。
    let lock = artifact_lock(id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if head_contains_marker(
        &std::fs::read_to_string(&index_path).unwrap_or_default(),
        &marker,
    ) {
        return Ok(false); // 并发写者已刷新（其 render 亦带当前标记）
    }
    let parts = read_source(&dir)?;
    let tokens = resolve_tokens(a.system_id.as_deref());
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()))?;
    write_atomic(&index_path, html.as_bytes())?;
    write_atomic(&dir.join("oidmap.json"), oidmap_json.as_bytes())?;
    crate::app_info!(
        "design",
        "service",
        "self-healed stale render (bridge v{}) for artifact {}",
        renderer::RENDER_VERSION,
        id
    );
    Ok(true)
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
    /// 属性编辑（B5：`href`/`src`/`alt`，白名单外静默跳过）。空值 = 清除该属性。
    #[serde(default)]
    pub attrs: Option<Vec<(String, String)>>,
    /// 删除元素（Wave 3-⑫）：为 true 时整段剔除 oid 元素（与其它字段互斥、优先处理）。
    #[serde(default)]
    pub remove: Option<bool>,
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
    let mut map = oidmap;
    // 删除元素（Wave 3-⑫）：与其它字段互斥，优先处理。删后 body 无任何元素则拒（最后可见元素保护）。
    if p.remove == Some(true) {
        let r = patch::apply_remove_patch(&new_body, &map, p.oid, None)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if patch::annotate(&r.new_source).1.is_empty() {
            anyhow::bail!("cannot remove the last remaining element");
        }
        return update_artifact(UpdateArtifactInput {
            id: a.id.clone(),
            title: None,
            body_html: Some(r.new_source),
            css: None,
            js: None,
            message: Some("Visual edit: remove element".to_string()),
            origin: Some("manual".to_string()),
            prompt_summary: None,
            expected_body_hash: Some(base_hash),
        });
    }
    // 先文本（改内部内容，位于 open tag 之后），后属性 / 样式（都改 open tag）。**每次改动 open tag
    // 后 re-annotate 拿新 offset**——attrs 与 styles 同改一个 open tag，若共用旧 map 第二次会用到
    // 被第一次改动移位的字节范围（值仅变、结构不变故 oid 稳定，re-annotate 给回同一 oid 的新偏移）。
    if let Some(text) = &p.text {
        let r = patch::apply_text_patch(&new_body, &map, p.oid, text, None)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        new_body = r.new_source;
        map = patch::annotate(&new_body).1;
    }
    if let Some(attrs) = &p.attrs {
        if !attrs.is_empty() {
            let r = patch::apply_attr_patch(&new_body, &map, p.oid, attrs, None)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            new_body = r.new_source;
            map = patch::annotate(&new_body).1;
        }
    }
    if let Some(styles) = &p.styles {
        if !styles.is_empty() {
            let r = patch::apply_style_patch(&new_body, &map, p.oid, styles, None)
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
        origin: Some("manual".to_string()),
        prompt_summary: None,
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
    /// 版本溯源标签（`ai`/`manual`/`restore`）。缺省视为 `ai`（此路径主用于模型重生成 /
    /// 精修）；可视化编辑与回滚在各自入口显式覆盖为 `manual`/`restore`。
    #[serde(default)]
    pub origin: Option<String>,
    /// 该版本对应的 prompt 摘要（AI 版本供历史面板 popover 展示）。
    #[serde(default)]
    pub prompt_summary: Option<String>,
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
    let (html, oidmap_json) = render(kind, &title, &parts, &tokens, is_rtl(a.metadata.as_deref()))?;
    write_working(&dir, &html, &parts, &oidmap_json)?;

    let next = a.current_version + 1;
    write_version_snapshot(&dir, next, &html, &parts, &oidmap_json)?;

    let ts = now();
    // 编辑落新版本：重跑确定性自查——改好的正文清 selfCheck 标记回 ready，仍 slop 保持标记。
    let (status, self_check_meta) = resolve_self_check(a.metadata.as_deref(), &parts.body_html, &parts.css);
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
        // 缺省视为 AI（模型重生成 / 精修）；可视化编辑 / 回滚在入口显式覆盖。
        origin: input.origin.or_else(|| Some("ai".to_string())),
        prompt_summary: input.prompt_summary,
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

/// **就地换设计系统**（restyle without rebuilding）：改产物
/// `system_id` + 用新系统 token 重渲染 `index.html`（**源码不变**，换皮靠产物 CSS 的 `var(--ds-*)`
/// + `:root` 注入新值），落新版本快照可回滚。owner 平面。`system_id=None` = 清除设计系统。
pub fn restyle_artifact(artifact_id: &str, system_id: Option<&str>) -> Result<DesignArtifact> {
    if let Some(sid) = system_id {
        if !is_valid_system_id(sid) {
            anyhow::bail!("非法设计系统 id: {sid}");
        }
    }
    // 与 update_artifact 同锁，串行化 read→重渲染→bump→snapshot→prune。
    let lock = artifact_lock(artifact_id);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

    let db = open_db()?;
    let a = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let kind = ArtifactKind::from_str(&a.kind)
        .with_context(|| format!("unknown artifact kind: {}", a.kind))?;
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let parts = read_source(&dir)?;

    db.set_artifact_system_id(&a.id, system_id)?;
    let tokens = resolve_tokens(system_id);
    let (html, oidmap_json) = render(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()))?;
    write_working(&dir, &html, &parts, &oidmap_json)?;

    let next = a.current_version + 1;
    write_version_snapshot(&dir, next, &html, &parts, &oidmap_json)?;
    let ts = now();
    // 正文未改：status / selfCheck 保持不变，仅 bump 版本。
    db.update_artifact_review(
        &a.id,
        None,
        &a.status,
        Some(next),
        a.metadata.as_deref(),
        &ts,
    )?;
    db.create_version(&DesignArtifactVersion {
        id: 0,
        artifact_id: a.id.clone(),
        version_number: next,
        message: Some("Restyle".to_string()),
        critique_score: None,
        // 换设计系统由用户发起（确定性重渲染，非 AI 生成内容）→ manual。
        origin: Some("manual".to_string()),
        prompt_summary: None,
        created_at: ts.clone(),
    })?;
    let keep = crate::config::cached_config()
        .design
        .max_versions_per_artifact
        .max(1);
    let _ = db.cleanup_old_versions(&a.id, keep);
    if let Ok(remaining) = db.list_versions(&a.id) {
        let keep_set: std::collections::HashSet<i64> =
            remaining.iter().map(|v| v.version_number).collect();
        prune_version_dirs_to_db(&dir, &keep_set);
    }
    db.touch_project(&a.project_id, &ts)?;
    emit("design:reload", json!({ "artifactId": a.id }));
    db.get_artifact(&a.id)?
        .context("artifact gone after restyle")
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
            let html = render_clean(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()));
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
    let (items, index_html): (Vec<super::export::ZipArtifact>, Option<String>) =
        if let Some(aid) = artifact_id.filter(|s| !s.is_empty()) {
            let a = db
                .get_artifact(aid)?
                .with_context(|| format!("artifact not found: {aid}"))?;
            let kind = ArtifactKind::from_str(&a.kind)
                .with_context(|| format!("unknown kind: {}", a.kind))?;
            let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
            let parts = read_source(&dir)?;
            let tokens = resolve_tokens(a.system_id.as_deref());
            let html = render_clean(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()));
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
                let html = render_clean(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()));
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

/// 批量导出选中产物为**一个** ZIP（每产物一目录 + 根 index.html 画廊），供文件面批量导出
/// （Wave 1-③）。集来自显式 id 列表而非整项目，其余目录/画廊结构与整包导出一致；不存在 /
/// 未知 kind 的 id 跳过，全空则报错。
pub fn export_selected_zip(ids: &[String]) -> Result<String> {
    use base64::Engine;
    if ids.is_empty() {
        anyhow::bail!("export_selected_zip needs at least one artifact id");
    }
    let db = open_db()?;
    let mut zitems = Vec::new();
    let mut gallery = String::new();
    for id in ids {
        let Some(a) = db.get_artifact(id)? else {
            continue;
        };
        let Some(kind) = ArtifactKind::from_str(&a.kind) else {
            continue;
        };
        let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
        let parts = read_source(&dir)?;
        let tokens = resolve_tokens(a.system_id.as_deref());
        let html = render_clean(kind, &a.title, &parts, &tokens, is_rtl(a.metadata.as_deref()));
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
            source: Some((parts.body_html, parts.css, parts.js)),
            title: a.title.clone(),
            kind: a.kind.clone(),
        });
    }
    if zitems.is_empty() {
        anyhow::bail!("no exportable artifacts in selection");
    }
    let title = format!("Selected artifacts ({})", zitems.len());
    let bytes = super::export::build_zip(&zitems, Some(&project_gallery_html(&title, &gallery)))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// 由前端栅格化的整页 PNG（base64，可带 data-uri 前缀）组装 PPTX，返回 base64。
/// PNG/PDF 走前端客户端栅格化；PPTX 因需 zip 打包由此后端构建（见 design/export.rs）。
/// 把一段 HTML 里的可见文本抽出来（剥标签 + 解基本实体 + 折叠空白）。
fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 取某标签第一处出现的内文（如 `<h1>…</h1>`）。大小写不敏感。
fn first_tag_inner<'a>(html: &'a str, tag: &str) -> Option<&'a str> {
    let low = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let start = low.find(&open)?;
    let gt = low[start..].find('>')? + start + 1;
    let close = format!("</{tag}");
    let end = low[gt..].find(&close)? + gt;
    Some(&html[gt..end])
}

/// 把 deck body 切成每页 HTML（按 `ds-slide` 类边界；无标记时整体作一页）。
fn split_deck_slides(body: &str) -> Vec<&str> {
    let low = body.to_ascii_lowercase();
    let mut idxs: Vec<usize> = Vec::new();
    let mut from = 0;
    while let Some(rel) = low[from..].find("ds-slide") {
        idxs.push(from + rel);
        from = from + rel + "ds-slide".len();
    }
    if idxs.is_empty() {
        return vec![body];
    }
    let mut out = Vec::new();
    for (k, &start) in idxs.iter().enumerate() {
        let end = idxs.get(k + 1).copied().unwrap_or(body.len());
        out.push(&body[start..end]);
    }
    out
}

/// 从一页 HTML 抽大纲：首个 h1/h2/h3 作标题，其余 li/p 文本作要点。
fn slide_outline(slide_html: &str) -> super::export::SlideOutline {
    let title = ["h1", "h2", "h3"]
        .iter()
        .find_map(|t| first_tag_inner(slide_html, t))
        .map(html_to_text)
        .unwrap_or_default();
    let mut bullets = Vec::new();
    let low = slide_html.to_ascii_lowercase();
    for tag in ["li", "p"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        let mut from = 0;
        while let Some(rel) = low[from..].find(&open) {
            let start = from + rel;
            let Some(gt) = low[start..].find('>').map(|g| start + g + 1) else {
                break;
            };
            let end = low[gt..].find(&close).map(|e| gt + e).unwrap_or(low.len());
            let text = html_to_text(&slide_html[gt..end]);
            if !text.is_empty() && text != title {
                bullets.push(text);
            }
            from = end;
            if bullets.len() >= 20 {
                break;
            }
        }
    }
    super::export::SlideOutline { title, bullets }
}

/// owner 平面：从 deck 产物**服务端抽大纲**生成可编辑文本 PPTX（结构化双模式的结构化半）。
/// 非 deck 形态 → 拒（图片模式走 `export_pptx`）。返回 base64 pptx。
pub fn export_pptx_outline(artifact_id: &str) -> Result<String> {
    use base64::Engine;
    let a = open_db()?
        .get_artifact(artifact_id)?
        .context("artifact not found")?;
    if a.kind != "deck" {
        anyhow::bail!("结构化 PPTX 仅支持 deck 形态");
    }
    let dir = paths::design_artifact_dir(&a.project_id, &a.id)?;
    let body = read_source(&dir)?.body_html;
    let outlines: Vec<super::export::SlideOutline> =
        split_deck_slides(&body).iter().map(|s| slide_outline(s)).collect();
    if outlines.is_empty() {
        anyhow::bail!("deck 无可导出的页面");
    }
    let bytes = super::export::build_pptx_outline(&outlines, &a.title)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

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
    s.replace('|', "\\|")
        .replace(['\n', '\r'], " ")
        .replace('`', "'")
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
            s.push_str(&format!(
                "| `{}` | `{}` |\n",
                md_table_cell(name),
                md_table_cell(value)
            ));
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
    let html = render_clean(kind, &a.title, &parts, &tokens_vec, is_rtl(a.metadata.as_deref()));

    let tokens_map: std::collections::BTreeMap<String, String> =
        tokens_vec.iter().cloned().collect();
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
        files.push((
            format!("tokens/{}", e.filename),
            e.content.clone().into_bytes(),
        ));
    }
    let bytes = super::export::build_files_zip(&files)?;
    Ok(ExportResult {
        filename: format!("{}-handoff.zip", safe_filename(&a.title)),
        mime: "application/zip".to_string(),
        content: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

// ── Code bindings (工程轴 D：设计系统 → 代码工程 token 同步) ─────────

/// 有效格式 id（token_export 的六目标）。
const BINDING_FORMATS: [&str; 6] = ["css", "scss", "ts", "swift", "android", "dtcg"];

/// 同步结果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingSyncReport {
    pub binding_id: i64,
    pub dir: String,
    pub written: Vec<String>,
    pub synced_at: String,
}

/// 校验并解析绑定的**写入目录**：canonicalize `target_dir`（须存在且是目录），拼相对
/// `subfolder`（拒绝绝对 / `..` 段），创建后再 canonicalize 校验仍在 root 内（防 symlink 逃逸）。
/// **安全边界**：一切写盘都只落在这个被校验、被 `target_dir` 包含的目录里，绝不越界。
fn resolve_binding_write_dir(target_dir: &str, subfolder: &str) -> Result<std::path::PathBuf> {
    let root = std::fs::canonicalize(target_dir)
        .with_context(|| format!("目标目录不存在或不可访问: {target_dir}"))?;
    if !root.is_dir() {
        anyhow::bail!("目标不是目录: {target_dir}");
    }
    let sub = std::path::Path::new(subfolder);
    if sub.is_absolute() {
        anyhow::bail!("子目录必须是相对路径");
    }
    for comp in sub.components() {
        use std::path::Component;
        if !matches!(comp, Component::Normal(_) | Component::CurDir) {
            anyhow::bail!("子目录不得含 '..' 或根段");
        }
    }
    let dir = root.join(sub);
    // **先校验包含性再动文件系统**：canonicalize 最深的已存在祖先，确保仍在 root 内——否则一个
    // 指向 root 外的目录符号链接会让 `create_dir_all` 在 root 外留下空目录副作用（review #2）。
    let mut ancestor = dir.as_path();
    let existing = loop {
        if ancestor.exists() {
            break ancestor;
        }
        match ancestor.parent() {
            Some(p) => ancestor = p,
            None => break ancestor,
        }
    };
    let real_ancestor = std::fs::canonicalize(existing)
        .with_context(|| format!("canonicalize {}", existing.display()))?;
    if !real_ancestor.starts_with(&root) {
        anyhow::bail!("写入路径经符号链接逃出目标目录");
    }
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建写入目录失败: {}", dir.display()))?;
    // 创建后再 canonicalize 校验一次（兜底 TOCTOU / 深层 symlink）。
    let real = std::fs::canonicalize(&dir)?;
    if !real.starts_with(&root) {
        anyhow::bail!("解析后的写入目录逃出目标目录");
    }
    Ok(real)
}

/// 归一化 + 校验格式列表（空 = 全部六种）。
fn normalize_binding_formats(formats: &[String]) -> Result<Vec<String>> {
    if formats.is_empty() {
        return Ok(BINDING_FORMATS.iter().map(|s| s.to_string()).collect());
    }
    for f in formats {
        if !BINDING_FORMATS.contains(&f.as_str()) {
            anyhow::bail!("未知格式 '{f}'（可选 css/scss/ts/swift/android/dtcg）");
        }
    }
    Ok(formats.to_vec())
}

/// 绑定一个设计系统到代码工程目录（**owner 平面专属**）。校验目录 + 系统存在后持久化。
pub fn bind_code_project(
    system_id: &str,
    target_dir: &str,
    subfolder: &str,
    formats: &[String],
) -> Result<DesignCodeBinding> {
    let formats = normalize_binding_formats(formats)?;
    // 校验（并创建）写入目录——绑定即确保目标可写、路径受控。
    let write_dir = resolve_binding_write_dir(target_dir, subfolder)?;
    // 防覆盖用户真实文件（review S2-1）：若目标目录已有同名 token 文件、且无本工具 manifest
    // （非既往本工具管理的目录），拒绝绑定——否则首次 sync 会静默清掉用户手写的 tokens.css 等。
    if !write_dir.join("DESIGN_TOKENS.md").exists() {
        let clash = super::token_export::export_all(&std::collections::BTreeMap::new())
            .iter()
            .any(|e| write_dir.join(&e.filename).exists());
        if clash {
            anyhow::bail!(
                "目标目录已存在同名 token 文件且非本工具管理——请改用一个专用/空子目录（如 design-tokens），或先移除这些文件，避免同步覆盖你的现有内容。"
            );
        }
    }
    let canonical = std::fs::canonicalize(target_dir)?
        .to_string_lossy()
        .into_owned();
    let db = open_db()?;
    system::ensure_builtins(&db)?; // 保证 FK 目标（含内置系统）已落 design_systems
    if system::read_full(&db, system_id).is_err() {
        anyhow::bail!("设计系统不存在: {system_id}");
    }
    let binding = db.add_code_binding(system_id, &canonical, subfolder, &formats, &now())?;
    emit("design:binding_changed", json!({ "systemId": system_id }));
    Ok(binding)
}

/// **严格**读取系统 tokens：区分「tokens.json 不存在=合法空」与「读/解析错误=上抛」。供外部
/// 写盘的绑定同步用——`resolve_tokens` 把读错误静默当空集，会用空骨架覆盖用户工程里的真实
/// token 文件（review #1）；这里 fail-closed 上抛，绝不静默降级为空。
fn parse_tokens_json(raw: &str) -> Result<Vec<(String, String)>> {
    let map: std::collections::BTreeMap<String, String> =
        serde_json::from_str(raw).context("tokens.json 解析失败")?;
    Ok(map.into_iter().collect())
}

fn resolve_tokens_strict(system_id: &str) -> Result<Vec<(String, String)>> {
    if !is_valid_system_id(system_id) {
        anyhow::bail!("非法设计系统 id: {system_id}");
    }
    let dir = paths::design_system_dir(system_id)?;
    let path = dir.join("tokens.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(anyhow::anyhow!("读取 tokens.json 失败: {e}")),
    };
    parse_tokens_json(&raw)
}

/// 同步：把绑定系统的多平台 token 文件写入其代码工程目录（复用 `token_export`）。
pub fn sync_code_binding(id: i64) -> Result<BindingSyncReport> {
    let db = open_db()?;
    let binding = db
        .get_code_binding(id)?
        .with_context(|| format!("绑定不存在: {id}"))?;
    // fail-closed 读 token：读/解析错误上抛、空集拒同步——绝不用空骨架覆盖工程里已有的真实文件。
    let tokens_map: std::collections::BTreeMap<String, String> =
        resolve_tokens_strict(&binding.system_id)?
            .into_iter()
            .collect();
    if tokens_map.is_empty() {
        anyhow::bail!(
            "设计系统「{}」没有可同步的 token，已跳过（避免用空文件覆盖工程里的现有 token）",
            binding.system_id
        );
    }
    let dev = super::token_export::export_all(&tokens_map);
    let dir = resolve_binding_write_dir(&binding.target_dir, &binding.subfolder)?;

    let mut written = Vec::new();
    for e in &dev {
        if !binding.formats.contains(&e.format) {
            continue;
        }
        crate::platform::write_atomic(&dir.join(&e.filename), e.content.as_bytes())?;
        written.push(e.filename.clone());
    }
    // 溯源清单（specific 文件名，避免撞项目 README）。
    let manifest = format!(
        "# Design tokens（自动生成，请勿手改）\n\n由 Hope Agent 设计空间从设计系统「{}」同步。\n\n文件：\n{}\n",
        binding.system_id,
        written.iter().map(|f| format!("- `{f}`")).collect::<Vec<_>>().join("\n")
    );
    crate::platform::write_atomic(&dir.join("DESIGN_TOKENS.md"), manifest.as_bytes())?;

    let synced_at = now();
    db.mark_binding_synced(id, &synced_at)?;
    crate::app_info!(
        "design",
        "binding_sync",
        "synced {} token files to {}",
        written.len(),
        dir.display()
    );
    emit(
        "design:binding_changed",
        json!({ "systemId": binding.system_id }),
    );
    Ok(BindingSyncReport {
        binding_id: id,
        dir: dir.to_string_lossy().into_owned(),
        written,
        synced_at,
    })
}

/// 列出绑定（可按 system 过滤）。
pub fn list_code_bindings(system_id: Option<&str>) -> Result<Vec<DesignCodeBinding>> {
    open_db()?.list_code_bindings(system_id)
}

/// 解绑（删绑定记录；**不删已同步到代码工程的文件**——那是工程侧资产）。
pub fn unbind_code_project(id: i64) -> Result<()> {
    let db = open_db()?;
    db.delete_code_binding(id)?;
    emit("design:binding_changed", json!({ "bindingId": id }));
    Ok(())
}

// ── Design systems ─────────────────────────────────────────────────

/// 列出设计系统（首次调用懒 seed 内置系统）。`swatches` 是 tokens.json 派生（不落库），
/// 逐系统本地读盘填充——毫秒级，选择器行内色点 + 右栏预览即时可用。
pub fn list_systems() -> Result<Vec<DesignSystemMeta>> {
    let db = open_db()?;
    system::ensure_builtins(&db)?;
    let mut systems = db.list_systems()?;
    for s in &mut systems {
        s.swatches = system::system_swatches(&s.id);
    }
    Ok(systems)
}

/// 读取设计系统正文 + token。
pub fn get_system_full(id: &str) -> Result<DesignSystemFull> {
    let db = open_db()?;
    system::ensure_builtins(&db)?;
    system::read_full(&db, id)
}

/// 设计系统「套件视图」自包含 HTML（B1-1）：色板 / 字阶 / 间距 / 圆角+阴影 / 组件 showcase，
/// 全走 `var(--ds-*)`——套件即系统真实视觉。前端进沙箱 iframe 渲染。
pub fn get_system_kit_html(id: &str) -> Result<String> {
    let full = get_system_full(id)?;
    Ok(super::kit::build_kit_html(
        &full.meta.name,
        &full.tokens,
        &full.assets.logos,
        &full.assets.images,
        &full.assets.fonts,
    ))
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
    // B1-4：落盘 harvest 的 logo/配图资产（best-effort，失败不阻断系统创建）。
    let _ = system::write_assets(
        &id,
        &system::DesignAssets {
            logos: extracted.logos,
            images: extracted.images,
            fonts: extracted.fonts,
        },
    );
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
    // fail-closed 读快照：NotFound→空（合法缺文件）；其它 IO 错误→上抛，绝不 unwrap_or_default
    // 把瞬时读错静默写成空正文的新版本（对齐 read_source 的硬化，review S4-3）。
    let read = |name: &str| -> Result<String> {
        match std::fs::read_to_string(vsrc.join(name)) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(anyhow::anyhow!("read version source/{name}: {e}")),
        }
    };
    update_artifact(UpdateArtifactInput {
        id: a.id.clone(),
        title: None,
        body_html: Some(read("body.html")?),
        css: Some(read("style.css")?),
        js: Some(read("script.js")?),
        message: Some(format!("Restored from v{version_number}")),
        origin: Some("restore".to_string()),
        prompt_summary: None,
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
        reference_image_paths: None,
        recipe_id: None,
        aspect_ratio: None,
        audio_duration_secs: None,
        folder: None,
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
        origin: Some("ai".to_string()),
        prompt_summary: Some(crate::truncate_utf8(&comment.body, 2000).to_string()),
        expected_body_hash: Some(patch::body_hash(&current.body_html)),
    })
}

#[cfg(test)]
mod pptx_outline_tests {
    use super::{slide_outline, split_deck_slides};

    #[test]
    fn split_and_outline_extracts_title_and_bullets() {
        let body = "<div class=\"ds-slide\"><h1>第一页</h1><ul><li>要点 A</li><li>要点 B</li></ul></div>\
<div class=\"ds-slide\"><h2>第二页</h2><p>正文一段</p></div>";
        let slides = split_deck_slides(body);
        assert_eq!(slides.len(), 2);
        let o1 = slide_outline(slides[0]);
        assert_eq!(o1.title, "第一页");
        assert_eq!(o1.bullets, vec!["要点 A", "要点 B"]);
        let o2 = slide_outline(slides[1]);
        assert_eq!(o2.title, "第二页");
        assert_eq!(o2.bullets, vec!["正文一段"]);
    }

    #[test]
    fn no_slide_marker_falls_back_to_single() {
        assert_eq!(split_deck_slides("<h1>x</h1>").len(), 1);
    }
}

#[cfg(test)]
mod inpaint_tests {
    use super::extract_image_from_body;
    use base64::Engine;

    #[test]
    fn extract_image_reads_data_uri() {
        let raw = b"hello-png";
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let body = format!("<img src=\"data:image/png;base64,{b64}\" alt=\"x\">");
        let (bytes, mime) = extract_image_from_body(&body).unwrap();
        assert_eq!(bytes, raw);
        assert_eq!(mime, "image/png");
        // 无内嵌图 → None。
        assert!(extract_image_from_body("<div>no image</div>").is_none());
    }
}

#[cfg(test)]
mod page_style_tests {
    use super::apply_page_style_css;

    #[test]
    fn apply_page_style_appends_marker_block() {
        let css = ".x{color:red}";
        let out = apply_page_style_css(
            css,
            &[
                ("background".into(), "#111".into()),
                ("max-width".into(), "1200px".into()),
            ],
        );
        assert!(out.starts_with(".x{color:red}"));
        assert!(out.contains("/*ds-page*/body{background:#111;max-width:1200px;}"));
    }

    #[test]
    fn apply_page_style_rewrites_not_duplicates() {
        let css = apply_page_style_css("", &[("background".into(), "#000".into())]);
        let css2 = apply_page_style_css(&css, &[("background".into(), "#fff".into())]);
        assert_eq!(css2.matches("/*ds-page*/").count(), 1, "标记块唯一");
        assert!(css2.contains("background:#fff;"));
        assert!(!css2.contains("#000"));
    }

    #[test]
    fn apply_page_style_sanitizes_and_drops_empty() {
        // 非法属性名过滤 + 值里的 } 剥除 + 空值移除该属性。
        let out = apply_page_style_css(
            "",
            &[
                ("color".into(), "red}评论<".into()),
                ("BAD PROP".into(), "x".into()),
                ("background".into(), "".into()),
            ],
        );
        assert!(out.contains("color:red评论;"));
        assert!(!out.contains("BAD PROP"));
        assert!(!out.contains("background"));
    }

    #[test]
    fn apply_page_style_empty_props_strips_block() {
        let css = apply_page_style_css(".x{}", &[("color".into(), "red".into())]);
        let cleared = apply_page_style_css(&css, &[]);
        assert!(!cleared.contains("/*ds-page*/"));
        assert!(cleared.contains(".x{}"));
    }
}

#[cfg(test)]
mod rtl_tests {
    use super::is_rtl;
    use crate::design::renderer::apply_document_dir;

    #[test]
    fn is_rtl_reads_metadata_dir() {
        assert!(is_rtl(Some(r#"{"dir":"rtl"}"#)));
        assert!(!is_rtl(Some(r#"{"dir":"ltr"}"#)));
        assert!(!is_rtl(Some(r#"{"other":"x"}"#)));
        assert!(!is_rtl(None));
        assert!(!is_rtl(Some("not json")));
    }

    #[test]
    fn apply_document_dir_injects_once() {
        let html = "<!doctype html>\n<html lang=\"zh\" data-ds-kind=\"web\">\n<head>".to_string();
        let out = apply_document_dir(html.clone(), true);
        assert!(out.contains("<html dir=\"rtl\" lang=\"zh\""));
        // 幂等：再次应用不重复。
        assert_eq!(apply_document_dir(out.clone(), true).matches("dir=\"rtl\"").count(), 1);
        // LTR 原样返回。
        assert_eq!(apply_document_dir(html.clone(), false), html);
    }
}

#[cfg(test)]
mod brand_pack_tests {
    use super::normalize_brand_pack_kinds;

    #[test]
    fn normalize_filters_dedups_caps_and_preserves_order() {
        let out = normalize_brand_pack_kinds(vec![
            "web".into(),
            "image".into(),   // 媒体形态过滤
            "deck".into(),
            "web".into(),      // 重复去掉
            "component".into(), // 过滤
            "poster".into(),
        ]);
        assert_eq!(out, vec!["web", "deck", "poster"]);
        // 空 / 全非法 → 空。
        assert!(normalize_brand_pack_kinds(vec!["image".into(), "audio".into()]).is_empty());
        // 超上限钳到 6。
        let many: Vec<String> = ["web", "mobile", "deck", "dashboard", "poster", "document", "email"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(normalize_brand_pack_kinds(many).len(), 6);
    }
}

#[cfg(test)]
mod lineage_tests {
    use super::merge_derived_from;

    #[test]
    fn merge_derived_from_adds_and_preserves() {
        // 空 metadata → 从 {} 起，只含 derivedFrom。
        let out = merge_derived_from(None, "src-1", "源产物").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["derivedFrom"]["id"], "src-1");
        assert_eq!(v["derivedFrom"]["title"], "源产物");
        // 既有 metadata（含 selfCheck）→ 保留其它键，追加 derivedFrom。
        let existing = r#"{"selfCheck":{"detail":"x"}}"#;
        let out2 = merge_derived_from(Some(existing), "src-2", "B").unwrap();
        let v2: serde_json::Value = serde_json::from_str(&out2).unwrap();
        assert_eq!(v2["selfCheck"]["detail"], "x");
        assert_eq!(v2["derivedFrom"]["id"], "src-2");
        // 非对象 metadata → 从 {} 起（不 panic）。
        let out3 = merge_derived_from(Some("[1,2,3]"), "src-3", "C").unwrap();
        let v3: serde_json::Value = serde_json::from_str(&out3).unwrap();
        assert_eq!(v3["derivedFrom"]["id"], "src-3");
    }
}

#[cfg(test)]
mod handoff_tests {
    use super::{css_var_referenced, referenced_tokens};
    use crate::design::ArtifactParts;

    #[test]
    fn css_var_ref_avoids_prefix_false_match() {
        // 精确边界：紧跟 ) / , / 空白 / 结尾算命中；作为更长名的前缀不算。
        assert!(css_var_referenced(
            "color: var(--ds-color-primary)",
            "--ds-color-primary"
        ));
        assert!(css_var_referenced(
            "var(--ds-color-primary, #fff)",
            "--ds-color-primary"
        ));
        assert!(css_var_referenced("var(--ds-radius )", "--ds-radius"));
        // 容 `(` 后空白（合法 CSS，review #3/#6/#7）。
        assert!(css_var_referenced(
            "var( --ds-color-primary )",
            "--ds-color-primary"
        ));
        assert!(css_var_referenced(
            "var(\n  --ds-space-4\n)",
            "--ds-space-4"
        ));
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
        assert_eq!(
            got,
            vec![("--ds-color-primary".to_string(), "#2563eb".to_string())]
        );
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
mod binding_tests {
    use super::{
        is_valid_system_id, normalize_binding_formats, parse_tokens_json, resolve_binding_write_dir,
    };

    #[test]
    fn valid_system_id_rejects_traversal() {
        // builtins + slugify 产出（[a-z0-9-]）均合法。
        assert!(is_valid_system_id("minimal-modern"));
        assert!(is_valid_system_id("brand-linear"));
        assert!(is_valid_system_id("t-dark"));
        // 路径穿越 / 分隔符 / 空 一律拒（review S4-2）。
        assert!(!is_valid_system_id("../../../etc/passwd"));
        assert!(!is_valid_system_id("a/b"));
        assert!(!is_valid_system_id(".."));
        assert!(!is_valid_system_id("a.b"));
        assert!(!is_valid_system_id(""));
    }

    #[test]
    fn parse_tokens_json_fails_closed_on_corrupt() {
        // 合法 → 有序键值。
        let ok = parse_tokens_json(r##"{"--ds-color-primary":"#2563eb"}"##).unwrap();
        assert_eq!(
            ok,
            vec![("--ds-color-primary".to_string(), "#2563eb".to_string())]
        );
        // 损坏 JSON → Err（绝不静默当空集，否则同步会用空骨架覆盖工程真实文件，review #1）。
        assert!(parse_tokens_json("{ not json").is_err());
        // 空对象 → 空集（同步侧的 is_empty 守卫会据此拒同步）。
        assert!(parse_tokens_json("{}").unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_write_dir_rejects_symlink_escape_without_side_effect() {
        use std::os::unix::fs::symlink;
        let root_tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::tempdir().unwrap();
        // root/link → 外部目录。
        symlink(outside_tmp.path(), root_tmp.path().join("link")).unwrap();
        let res = resolve_binding_write_dir(&root_tmp.path().to_string_lossy(), "link/tokens");
        assert!(res.is_err(), "经符号链接逃逸应被拒");
        // 关键（review #2）：拒绝前不得在 root 外留下 mkdir 副作用。
        assert!(
            !outside_tmp.path().join("tokens").exists(),
            "拒绝前不应在 root 外创建目录"
        );
    }

    #[test]
    fn resolve_write_dir_contains_and_rejects_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().into_owned();
        // 正常子目录 → 创建并落在 root 内。
        let dir = resolve_binding_write_dir(&root, "src/tokens").unwrap();
        assert!(dir.starts_with(std::fs::canonicalize(&root).unwrap()));
        assert!(dir.ends_with("tokens"));
        // 空子目录 = 根。
        assert!(resolve_binding_write_dir(&root, "").is_ok());
        // 拒绝 .. 逃逸。
        assert!(resolve_binding_write_dir(&root, "../evil").is_err());
        assert!(resolve_binding_write_dir(&root, "a/../../evil").is_err());
        // 拒绝绝对路径。
        assert!(resolve_binding_write_dir(&root, "/etc").is_err());
        // 不存在的目标目录 → 报错。
        assert!(resolve_binding_write_dir("/no/such/dir/xyz", "").is_err());
    }

    #[test]
    fn normalize_formats_defaults_and_validates() {
        assert_eq!(normalize_binding_formats(&[]).unwrap().len(), 6);
        assert_eq!(
            normalize_binding_formats(&["css".into(), "swift".into()]).unwrap(),
            vec!["css".to_string(), "swift".to_string()]
        );
        assert!(normalize_binding_formats(&["bogus".into()]).is_err());
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
