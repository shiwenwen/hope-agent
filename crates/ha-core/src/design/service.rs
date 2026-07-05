//! 设计空间 owner 平面业务入口（Tauri / HTTP 薄壳统一调用）。
//!
//! owner 平面 = 本机 / API key 信任，负责 UI 的项目/产物 CRUD、可视化编辑回写、
//! 导出——**不经 agent 访问检查**（见 `docs/architecture/design-space.md` §3）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::db::{DesignArtifact, DesignArtifactVersion, DesignDb, DesignProject, DesignSystemMeta};
use super::patch;
use super::renderer::{self, ArtifactKind, ArtifactParts};
use super::system::{self, DesignSystemFull};
use crate::paths;
use crate::platform::write_atomic;
use std::collections::BTreeMap;

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
    let editable = kind != ArtifactKind::Image;
    let (html, oidmap) = renderer::build_artifact_html(kind, title, parts, tokens, editable);
    let oidmap_json = serde_json::to_string(&oidmap)?;
    Ok((html, oidmap_json))
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
}

/// 若 image 形态且无 body，用 prompt/title 调 image_generate 生成后再落库。
/// owner（Tauri/HTTP）与 agent 工具共用此入口。
pub async fn create_artifact_generating(mut input: CreateArtifactInput) -> Result<DesignArtifact> {
    if input.kind == "image" && input.body_html.as_deref().unwrap_or("").trim().is_empty() {
        let prompt = input
            .prompt
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| input.title.clone());
        let parts = super::image::generate_image_parts(&prompt, &input.title).await?;
        input.body_html = Some(parts.body_html);
    }
    create_artifact(input)
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

    let parts = if input.body_html.as_deref().unwrap_or("").trim().is_empty() {
        renderer::placeholder_parts(kind, &title)
    } else {
        ArtifactParts {
            body_html: input.body_html.unwrap_or_default(),
            css: input.css.unwrap_or_default(),
            js: input.js.unwrap_or_default(),
        }
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
        status: "ready".to_string(),
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

pub fn list_artifacts(project_id: &str) -> Result<Vec<DesignArtifact>> {
    open_db()?.list_artifacts(project_id)
}

pub fn list_all_artifacts() -> Result<Vec<DesignArtifact>> {
    open_db()?.list_all_artifacts()
}

pub fn get_artifact(id: &str) -> Result<Option<DesignArtifact>> {
    open_db()?.get_artifact(id)
}

pub fn delete_artifact(id: &str) -> Result<()> {
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
    db.update_artifact(
        &a.id,
        input.title.as_deref(),
        Some("ready"),
        Some(next),
        None,
        None,
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
            // editable=false → 无 inspector bridge / 无 oid，干净可交付。
            let (html, _) = renderer::build_artifact_html(kind, &a.title, &parts, &tokens, false);
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
        let (html, _) = renderer::build_artifact_html(kind, &a.title, &parts, &tokens, false);
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
            let (html, _) = renderer::build_artifact_html(kind, &a.title, &parts, &tokens, false);
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
                source: None,
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
