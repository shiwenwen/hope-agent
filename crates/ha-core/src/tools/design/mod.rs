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

    match action {
        "list_recipes" => action_list_recipes(args),
        "get_recipe" => action_get_recipe(args),
        "list_systems" => action_list_systems(),
        "get_system" => action_get_system(args),
        "extract_system" => action_extract_system(args).await,
        "propose_directions" => action_propose_directions(args).await,
        "list_projects" => action_list_projects(),
        "list_artifacts" => action_list_artifacts(args, session_id),
        "get_artifact" => action_get_artifact(args),
        "create_artifact" => action_create_artifact(args, session_id, agent_id),
        "update_artifact" => action_update_artifact(args),
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

async fn action_extract_system(args: &Value) -> Result<String> {
    let from = require_str(args, "from")?;
    let name = str_arg(args, "title")
        .unwrap_or("提取的设计系统")
        .to_string();
    let meta = service::extract_system(service::ExtractSystemInput {
        name,
        from: from.to_string(),
        brief: str_arg(args, "brief").map(str::to_string),
        path: str_arg(args, "path").map(str::to_string),
    })
    .await?;
    ok(json!({ "status": "extracted", "systemId": meta.id, "name": meta.name }))
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

fn action_create_artifact(
    args: &Value,
    session_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<String> {
    let kind = require_str(args, "kind")?;
    ArtifactKind::from_str(kind).with_context(|| format!("unknown kind: {kind}"))?;

    // 项目：显式 > 会话默认（自动创建草稿项目）。
    let project_id = match str_arg(args, "project_id") {
        Some(p) => p.to_string(),
        None => service::get_or_create_session_project(session_id, agent_id)?.id,
    };

    let title = str_arg(args, "title").unwrap_or("未命名产物").to_string();
    let input = CreateArtifactInput {
        project_id: project_id.clone(),
        title,
        kind: kind.to_string(),
        system_id: str_arg(args, "system_id").map(str::to_string),
        body_html: str_arg(args, "body_html").map(str::to_string),
        css: str_arg(args, "css").map(str::to_string),
        js: str_arg(args, "js").map(str::to_string),
        session_id: session_id.map(str::to_string),
    };
    let artifact = service::create_artifact(input)?;
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
    })?;
    ok(json!({
        "status": "updated",
        "artifactId": artifact.id,
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
