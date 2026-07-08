//! `design` agent 工具：模型自主创建 / 迭代设计产物。
//!
//! agent 平面入口——逻辑复用 owner 平面 `crate::design::service`（Phase 3 访问门控
//! 从简，Phase 6 接入设计系统注入与访问裁决）。见 docs/architecture/design-space.md §8。

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::design::service::{self, CreateArtifactInput, UpdateArtifactInput};
use crate::design::{recipe, ArtifactKind};

pub(crate) async fn tool_design(
    args: &Value,
    ctx: &super::execution::ToolExecContext,
) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    let session_id = ctx.session_id.as_deref();
    let agent_id = ctx.agent_id.as_deref();

    // 无痕会话 fail-closed：设计空间产物 / 系统落盘落库（session_id 键），与「关闭即焚」冲突，
    // 且 design 是 project 类持久容器、本就与 incognito 互斥（对齐 AGENTS incognito 红线）。
    if crate::session::is_session_incognito(session_id) {
        anyhow::bail!(
            "设计空间在无痕会话中不可用——产物会落盘持久化，与无痕「关闭即焚」冲突。请在普通会话中使用。"
        );
    }

    match action {
        "list_recipes" => action_list_recipes(args),
        "get_recipe" => action_get_recipe(args),
        "list_systems" => action_list_systems(),
        "get_system" => action_get_system(args),
        "extract_system" => action_extract_system(args, session_id).await,
        "import_design_md" => action_import_design_md(args).await,
        "export_system" => action_export_system(args),
        "export_tokens" => action_export_tokens(args),
        "propose_directions" => action_propose_directions(args).await,
        "list_projects" => action_list_projects(),
        "list_artifacts" => action_list_artifacts(args, session_id),
        "get_artifact" => action_get_artifact(args),
        "create_artifact" => action_create_artifact(args, session_id, agent_id).await,
        "update_artifact" => action_update_artifact(args),
        "restyle" => action_restyle(args),
        "delete_artifact" => action_delete_artifact(args),
        "versions" => action_versions(args),
        "restore" => action_restore(args),
        "critique" => action_critique(args).await,
        "save_to_knowledge" => action_save_to_knowledge(args),
        "show" => action_show(args, session_id),
        other => Err(anyhow::anyhow!("Unknown design action: '{}'", other)),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    str_arg(args, key).with_context(|| format!("Missing '{key}' parameter"))
}

fn ok(value: Value) -> Result<String> {
    Ok(serde_json::to_string(&value)?)
}

// ── Recipes / systems ──────────────────────────────────────────────

fn action_list_recipes(args: &Value) -> Result<String> {
    let kind = str_arg(args, "kind");
    let mut recipes = recipe::builtin_recipes();
    if let Some(k) = kind {
        recipes.retain(|r| r.kind == k);
    }
    ok(json!({
        "recipes": recipes,
        "commonGuidance": recipe::COMMON_GUIDANCE,
    }))
}

fn action_get_recipe(args: &Value) -> Result<String> {
    let id = require_str(args, "recipe_id")?;
    match recipe::get_recipe(id) {
        Some(r) => ok(json!({ "recipe": r, "commonGuidance": recipe::COMMON_GUIDANCE })),
        None => Err(anyhow::anyhow!("recipe not found: {id}")),
    }
}

fn action_list_systems() -> Result<String> {
    let systems = service::list_systems()?;
    ok(json!({ "systems": systems }))
}

fn action_get_system(args: &Value) -> Result<String> {
    let id = require_str(args, "system_id")?;
    let full = service::get_system_full(id)?;
    ok(serde_json::to_value(full)?)
}

/// 导入一份 DESIGN.md（`content` 或 `brief` 传文本）为设计系统（互通格式）。
async fn action_import_design_md(args: &Value) -> Result<String> {
    let md = str_arg(args, "content")
        .or_else(|| str_arg(args, "brief"))
        .context("Missing 'content' (DESIGN.md text) parameter")?;
    let name = str_arg(args, "title").unwrap_or("").to_string();
    let meta = service::import_design_md(&name, md).await?;
    ok(json!({ "status": "imported", "systemId": meta.id, "name": meta.name }))
}

/// 导出一个设计系统为规范 DESIGN.md 文本。
fn action_export_system(args: &Value) -> Result<String> {
    let id = require_str(args, "system_id")?;
    let md = service::export_design_md(id)?;
    ok(json!({ "systemId": id, "designMd": md }))
}

/// 导出设计系统 Token 为多平台开发者格式（CSS/SCSS/TS/Swift/Android/DTCG）。
/// 可选 `format` 只取单个目标；缺省返回全部。
fn action_export_tokens(args: &Value) -> Result<String> {
    let id = require_str(args, "system_id")?;
    let mut exports = service::export_tokens(id)?;
    if let Some(fmt) = str_arg(args, "format") {
        exports.retain(|e| e.format == fmt);
        if exports.is_empty() {
            return Err(anyhow::anyhow!(
                "Unknown format '{fmt}'; expected one of css/scss/ts/swift/android/dtcg"
            ));
        }
    }
    ok(json!({ "systemId": id, "exports": exports }))
}

async fn action_extract_system(args: &Value, session_id: Option<&str>) -> Result<String> {
    let from = require_str(args, "from")?;
    let name = str_arg(args, "title")
        .unwrap_or("提取的设计系统")
        .to_string();
    // Agent-plane path guard: `from=image|codebase` reads a local file/dir and ships
    // it to a remote (vision) model. Scope the path to the session working directory
    // or its attachments so a prompt-injected model cannot read unrelated files
    // (credentials / SSH keys / DBs) and exfiltrate them. The owner plane (Tauri /
    // HTTP → `service::extract_system` directly) stays unrestricted (local trust).
    let path = match from {
        "image" | "codebase" => Some(
            scoped_local_path(session_id, require_str(args, "path")?)?
                .to_string_lossy()
                .into_owned(),
        ),
        _ => str_arg(args, "path").map(str::to_string),
    };
    let meta = service::extract_system(service::ExtractSystemInput {
        name,
        from: from.to_string(),
        brief: str_arg(args, "brief").map(str::to_string),
        path,
        url: str_arg(args, "url").map(str::to_string),
    })
    .await?;
    ok(json!({ "status": "extracted", "systemId": meta.id, "name": meta.name }))
}

/// Agent-plane filesystem guard for design extraction. Resolves `raw` (absolute, or
/// relative to the session working directory) to a canonical path and requires it to
/// live under the session working directory or that session's attachments directory;
/// anything else is rejected fail-closed. This is what keeps the approval-exempt
/// `design` tool from becoming an arbitrary-local-file-read + exfiltration primitive.
fn scoped_local_path(session_id: Option<&str>, raw: &str) -> Result<std::path::PathBuf> {
    let sid = session_id.context("a session is required to read local files for extraction")?;
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("empty path");
    }
    // Allowed roots: session working directory (if any) ∪ session attachments dir.
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(wd) = crate::session::effective_session_working_dir(Some(sid)) {
        if let Ok(c) = std::path::Path::new(&wd).canonicalize() {
            roots.push(c);
        }
    }
    if let Ok(c) = crate::paths::attachments_dir(sid).and_then(|d| Ok(d.canonicalize()?)) {
        roots.push(c);
    }
    if roots.is_empty() {
        anyhow::bail!("no scoped directory is available for reading local files in this session");
    }
    let p = std::path::Path::new(raw);
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        roots[0].join(p)
    };
    let canon = candidate
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("path not found or inaccessible: {raw}"))?;
    if !roots.iter().any(|r| canon.starts_with(r)) {
        anyhow::bail!(
            "path is outside the session working directory / attachments — design \
             extraction is scoped for safety (move the file into the working directory): {raw}"
        );
    }
    Ok(canon)
}

// ── Projects / artifacts ───────────────────────────────────────────

fn action_list_projects() -> Result<String> {
    let projects = service::list_projects()?;
    ok(json!({ "projects": projects }))
}

fn action_list_artifacts(args: &Value, session_id: Option<&str>) -> Result<String> {
    let project_id = match str_arg(args, "project_id") {
        Some(p) => p.to_string(),
        None => service::get_or_create_session_project(session_id, None)?.id,
    };
    let artifacts = service::list_artifacts(&project_id)?;
    ok(json!({ "projectId": project_id, "artifacts": artifacts }))
}

fn action_get_artifact(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    match service::get_artifact_view(id)? {
        Some(v) => ok(serde_json::to_value(v)?),
        None => Err(anyhow::anyhow!("artifact not found: {id}")),
    }
}

async fn action_create_artifact(
    args: &Value,
    session_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<String> {
    let kind = require_str(args, "kind")?;
    let kind_enum =
        ArtifactKind::from_str(kind).with_context(|| format!("unknown kind: {kind}"))?;

    // 项目：显式 > 会话默认（自动创建草稿项目）。
    let project_id = match str_arg(args, "project_id") {
        Some(p) => p.to_string(),
        None => service::get_or_create_session_project(session_id, agent_id)?.id,
    };

    let _ = kind_enum;
    let title = str_arg(args, "title").unwrap_or("未命名产物").to_string();

    // image 形态的生成在 service::create_artifact_generating 内统一处理（owner/agent 共用）。
    let input = CreateArtifactInput {
        project_id: project_id.clone(),
        title,
        kind: kind.to_string(),
        system_id: str_arg(args, "system_id").map(str::to_string),
        body_html: str_arg(args, "body_html").map(str::to_string),
        css: str_arg(args, "css").map(str::to_string),
        js: str_arg(args, "js").map(str::to_string),
        session_id: session_id.map(str::to_string),
        prompt: str_arg(args, "prompt")
            .or_else(|| str_arg(args, "brief"))
            .map(str::to_string),
        reference_image_b64: None,
        reference_image_mime: None,
        recipe_id: str_arg(args, "recipe_id").map(str::to_string),
        aspect_ratio: str_arg(args, "aspect_ratio").map(str::to_string),
        audio_duration_secs: args.get("audio_duration_secs").and_then(|v| v.as_f64()),
    };
    let artifact = service::create_artifact_generating(input).await?;
    ok(json!({
        "status": "created",
        "projectId": project_id,
        "artifactId": artifact.id,
        "kind": artifact.kind,
        "version": artifact.current_version,
    }))
}

fn action_update_artifact(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let artifact = service::update_artifact(UpdateArtifactInput {
        id: id.to_string(),
        title: str_arg(args, "title").map(str::to_string),
        body_html: str_arg(args, "body_html").map(str::to_string),
        css: str_arg(args, "css").map(str::to_string),
        js: str_arg(args, "js").map(str::to_string),
        message: str_arg(args, "version_message").map(str::to_string),
        origin: Some("ai".to_string()),
        prompt_summary: None,
        expected_body_hash: None,
    })?;
    ok(json!({
        "status": "updated",
        "artifactId": artifact.id,
        "version": artifact.current_version,
    }))
}

/// 就地换设计系统（restyle）：不改源码，用新系统 token 重渲染既有产物。省略 `system_id` = 清除。
fn action_restyle(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let artifact = service::restyle_artifact(id, str_arg(args, "system_id"))?;
    ok(json!({
        "status": "restyled",
        "artifactId": artifact.id,
        "systemId": artifact.system_id,
        "version": artifact.current_version,
    }))
}

fn action_delete_artifact(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    service::delete_artifact(id)?;
    ok(json!({ "status": "deleted", "artifactId": id }))
}

fn action_versions(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let versions = service::list_versions(id)?;
    ok(json!({ "artifactId": id, "versions": versions }))
}

fn action_restore(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let version = args
        .get("version_id")
        .and_then(|v| v.as_i64())
        .context("Missing 'version_id' parameter")?;
    let artifact = service::restore_version(id, version)?;
    ok(json!({
        "status": "restored",
        "artifactId": artifact.id,
        "restoredFrom": version,
        "version": artifact.current_version,
    }))
}

async fn action_propose_directions(args: &Value) -> Result<String> {
    let brief = require_str(args, "brief")?;
    let n = args.get("count").and_then(|v| v.as_u64()).unwrap_or(4) as usize;
    let directions = service::propose_directions(brief, n).await?;
    ok(json!({ "directions": directions }))
}

async fn action_critique(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let result = service::critique_artifact(id).await?;
    ok(serde_json::to_value(result)?)
}

fn action_save_to_knowledge(args: &Value) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let path = service::save_to_knowledge(id, str_arg(args, "kb_id"))?;
    ok(json!({ "status": "saved", "artifactId": id, "note": path }))
}

fn action_show(args: &Value, session_id: Option<&str>) -> Result<String> {
    let id = require_str(args, "artifact_id")?;
    let view =
        service::get_artifact_view(id)?.with_context(|| format!("artifact not found: {id}"))?;
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "design:show",
            json!({
                "projectId": view.artifact.project_id,
                "artifactId": view.artifact.id,
                "sessionId": session_id,
            }),
        );
    }
    ok(json!({ "status": "shown", "artifactId": id }))
}
