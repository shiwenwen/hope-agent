use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::artifacts::{
    ArtifactImportSource, ArtifactKind, ArtifactService, CreateArtifactInput, ListArtifactsInput,
    UpdateArtifactInput,
};

use super::execution::ToolExecContext;

pub(crate) async fn tool_artifact(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if ctx.incognito {
        return Err(anyhow!(
            "artifact tool is unavailable in incognito sessions because Artifacts are durable"
        ));
    }
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing 'action' parameter"))?;

    match action {
        "create_from_file" => create_from_file(args, ctx),
        "update_from_file" => update_from_file(args, ctx),
        "show" => show(args),
        "list" => list(args),
        "versions" => versions(args),
        "restore" => restore(args),
        "verify" => verify(args),
        _ => Err(anyhow!("Unknown artifact action: '{action}'")),
    }
}

fn create_from_file(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let raw_path = required_str(args, "file_path")?;
    let mut service = ArtifactService::open()?;
    let artifact = service.create_from_source(CreateArtifactInput {
        source: ArtifactImportSource::Path {
            file_path: PathBuf::from(ctx.resolve_path(raw_path)),
            allowed_roots: Some(allowed_roots(ctx)),
        },
        title: optional_string(args, "title"),
        kind: ArtifactKind::parse(args.get("kind").and_then(Value::as_str)),
        privacy: args
            .get("privacy")
            .and_then(Value::as_str)
            .unwrap_or("local_private")
            .to_string(),
        session_id: ctx.session_id.clone(),
        project_id: ctx.project_id.clone(),
        agent_id: ctx.agent_id.clone(),
        goal_id: None,
        producer: producer(ctx),
        incognito: ctx.incognito,
    })?;
    emit_show(&artifact);
    Ok(json!({
        "status": "created",
        "artifact": artifact,
        "message": "Artifact was copied into managed storage and opened in the preview."
    })
    .to_string())
}

fn update_from_file(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let artifact_id = required_str(args, "artifact_id")?;
    let raw_path = required_str(args, "file_path")?;
    let expected_version = args
        .get("expected_version")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("Missing 'expected_version' parameter"))?;
    let mut service = ArtifactService::open()?;
    let current = service
        .get(artifact_id)?
        .ok_or_else(|| anyhow!("Artifact '{artifact_id}' not found"))?;
    if current.current_version != expected_version {
        return Ok(json!({
            "status": "conflict",
            "artifact_id": artifact_id,
            "expected_version": expected_version,
            "current_version": current.current_version,
            "current_hash": current.current_hash,
            "message": "Re-read the current Artifact, merge changes, and retry with current_version."
        })
        .to_string());
    }
    let update = service.update_from_source(UpdateArtifactInput {
        artifact_id: artifact_id.to_string(),
        source: ArtifactImportSource::Path {
            file_path: PathBuf::from(ctx.resolve_path(raw_path)),
            allowed_roots: Some(allowed_roots(ctx)),
        },
        expected_version,
        title: optional_string(args, "title"),
        message: optional_string(args, "version_message"),
        producer: producer(ctx),
        incognito: ctx.incognito,
    });
    let artifact = match update {
        Ok(artifact) => artifact,
        Err(error) if error.to_string().contains("version conflict") => {
            let current = service
                .get(artifact_id)?
                .ok_or_else(|| anyhow!("Artifact '{artifact_id}' not found after conflict"))?;
            return Ok(json!({
                "status": "conflict",
                "artifact_id": artifact_id,
                "expected_version": expected_version,
                "current_version": current.current_version,
                "current_hash": current.current_hash,
                "message": "Re-read the current Artifact, merge changes, and retry with current_version."
            })
            .to_string());
        }
        Err(error) => return Err(error),
    };
    emit_show(&artifact);
    Ok(json!({ "status": "updated", "artifact": artifact }).to_string())
}

fn show(args: &Value) -> Result<String> {
    let artifact_id = required_str(args, "artifact_id")?;
    let artifact = ArtifactService::open()?
        .get(artifact_id)?
        .ok_or_else(|| anyhow!("Artifact '{artifact_id}' not found"))?;
    emit_show(&artifact);
    Ok(json!({ "status": "shown", "artifact": artifact }).to_string())
}

fn list(args: &Value) -> Result<String> {
    let artifacts = ArtifactService::open()?.list(ListArtifactsInput {
        limit: args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize,
        offset: args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
        kind: optional_string(args, "kind"),
        lifecycle_state: optional_string(args, "lifecycle_state"),
    })?;
    Ok(json!({ "artifacts": artifacts }).to_string())
}

fn versions(args: &Value) -> Result<String> {
    let artifact_id = required_str(args, "artifact_id")?;
    let versions = ArtifactService::open()?.versions(artifact_id)?;
    Ok(json!({ "artifact_id": artifact_id, "versions": versions }).to_string())
}

fn restore(args: &Value) -> Result<String> {
    let artifact_id = required_str(args, "artifact_id")?;
    let version = args
        .get("version")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("Missing 'version' parameter"))?;
    let artifact = ArtifactService::open()?.restore(artifact_id, version)?;
    emit_show(&artifact);
    Ok(json!({
        "status": "restored",
        "restored_from": version,
        "artifact": artifact
    })
    .to_string())
}

fn verify(args: &Value) -> Result<String> {
    let artifact_id = required_str(args, "artifact_id")?;
    let report = ArtifactService::open()?.verify(artifact_id)?;
    Ok(json!({
        "status": report.status,
        "artifact_id": artifact_id,
        "verification": report
    })
    .to_string())
}

fn emit_show(artifact: &crate::artifacts::ArtifactRecord) {
    super::canvas::emit_canvas_event(
        "canvas_show",
        &json!({
            "projectId": artifact.id,
            "artifactId": artifact.id,
            "title": artifact.title,
            "contentType": artifact.content_type,
            "projectPath": artifact.project_path,
            "sessionId": artifact.session_id,
        }),
    );
}

fn allowed_roots(ctx: &ToolExecContext) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = ctx.session_working_dir.as_deref() {
        roots.push(PathBuf::from(path));
    }
    if let Some(path) = ctx.home_dir.as_deref() {
        roots.push(PathBuf::from(path));
    }
    if roots.is_empty() {
        roots.push(PathBuf::from(ctx.default_path()));
    }
    roots
}

fn producer(ctx: &ToolExecContext) -> Value {
    json!({
        "type": "agent_tool",
        "tool": "artifact",
        "agentId": ctx.agent_id,
        "sessionId": ctx.session_id,
        "generatedAt": chrono::Utc::now().to_rfc3339()
    })
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Missing '{key}' parameter"))
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
