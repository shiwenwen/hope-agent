//! design 的 MCP `ToolProvider`（平台级 `hope-agent mcp` 首个消费者）。
//!
//! 让外部编码 agent（Claude Code / Cursor）经标准 MCP 读/改设计空间产物。全部薄包
//! `crate::design::service`（owner 平面），与 HTTP / Tauri 平级复用、零新逻辑。
//!
//! **红线**：`--allow-writes` 才暴露写工具；**恒不暴露** implement_to_code / 代码绑定写 /
//! deploy / share / delete_project / delete_artifact / save_to_knowledge / extract_system
//! （scoped_local_path 以会话为根，MCP 无会话无法安全界定读根）/ export_*（写 Downloads）——
//! 外部 agent 不得经 MCP 写用户代码仓库、对外发布或删除容器。见 docs/architecture/mcp-server.md。

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::service;
use crate::mcp_server::{McpCtx, ToolDef, ToolProvider};

/// generating 壳超时判定阈值（与 `reconcile_orphaned_generating` 的 600s grace 对齐）。
const GENERATING_STALE_SECS: i64 = 600;

pub struct DesignToolProvider;

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("missing required string arg: {key}"))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// `{prop: "val"}` → `Vec<(prop, val)>`（值 to_string），供 edit_element 的 style/attrs。
fn parse_kv(args: &Value, key: &str) -> Option<Vec<(String, String)>> {
    let obj = args.get(key)?.as_object()?;
    if obj.is_empty() {
        return None;
    }
    Some(
        obj.iter()
            .map(|(k, v)| {
                let val = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), val)
            })
            .collect(),
    )
}

impl ToolProvider for DesignToolProvider {
    fn name(&self) -> &'static str {
        "design"
    }

    fn enabled(&self) -> bool {
        crate::config::cached_config().design.enabled
    }

    fn instructions(&self) -> Option<&'static str> {
        Some(
            "Use design_* tools to read and edit Hope Agent Design Space artifacts. \
             Call design_get_active_context to see what the user is currently viewing. \
             design_get_artifact returns oid-annotated source for precise design_edit_element edits.",
        )
    }

    fn tools(&self, ctx: &McpCtx) -> Vec<ToolDef> {
        let mut tools = vec![
            ToolDef {
                name: "design_list_projects",
                description: "List all design projects (most-recently-updated first).".into(),
                input_schema: json!({ "type": "object", "properties": {} }),
                read_only: true,
            },
            ToolDef {
                name: "design_list_artifacts",
                description: "List artifacts in a design project.".into(),
                input_schema: json!({
                    "type": "object", "required": ["projectId"],
                    "properties": { "projectId": { "type": "string" } }
                }),
                read_only: true,
            },
            ToolDef {
                name: "design_get_artifact",
                description: "Read one artifact: metadata, oid-annotated source (for edit_element), and open comments. status=='generating' means still rendering — poll until it changes.".into(),
                input_schema: json!({
                    "type": "object", "required": ["artifactId"],
                    "properties": {
                        "artifactId": { "type": "string" },
                        "includeSource": { "type": "boolean", "default": true },
                        "includeComments": { "type": "boolean", "default": true }
                    }
                }),
                read_only: true,
            },
            ToolDef {
                name: "design_get_active_context",
                description: "What the user is currently viewing in the Design Space: the active project + artifact summary, open comments, and code binding.".into(),
                input_schema: json!({ "type": "object", "properties": {} }),
                read_only: true,
            },
            ToolDef {
                name: "design_list_systems",
                description: "List design systems (built-in + user + extracted).".into(),
                input_schema: json!({ "type": "object", "properties": {} }),
                read_only: true,
            },
            ToolDef {
                name: "design_get_system",
                description: "Read one design system: DESIGN.md contract + tokens. Optional tokenFormat filters to one platform export (css/scss/ts/swift/android/dtcg).".into(),
                input_schema: json!({
                    "type": "object", "required": ["systemId"],
                    "properties": {
                        "systemId": { "type": "string" },
                        "tokenFormat": { "type": "string", "enum": ["css", "scss", "ts", "swift", "android", "dtcg"] }
                    }
                }),
                read_only: true,
            },
            ToolDef {
                name: "design_list_comments",
                description: "List canvas comments on an artifact. openOnly filters to unresolved.".into(),
                input_schema: json!({
                    "type": "object", "required": ["artifactId"],
                    "properties": {
                        "artifactId": { "type": "string" },
                        "openOnly": { "type": "boolean", "default": false }
                    }
                }),
                read_only: true,
            },
            ToolDef {
                name: "design_list_versions",
                description: "List an artifact's version history.".into(),
                input_schema: json!({
                    "type": "object", "required": ["artifactId"],
                    "properties": { "artifactId": { "type": "string" } }
                }),
                read_only: true,
            },
        ];

        if ctx.allow_writes {
            tools.extend([
                ToolDef {
                    name: "design_generate_artifact",
                    description: "Generate a new self-contained design artifact from a brief. Static HTML kinds return a generating shell immediately — poll design_get_artifact until status != 'generating'. image/audio/component kinds block until complete.".into(),
                    input_schema: json!({
                        "type": "object", "required": ["projectId", "title", "kind", "brief"],
                        "properties": {
                            "projectId": { "type": "string" },
                            "title": { "type": "string" },
                            "kind": { "type": "string", "enum": ["web","mobile","deck","dashboard","poster","document","email","image","motion","audio","component"] },
                            "brief": { "type": "string" },
                            "systemId": { "type": "string" },
                            "recipeId": { "type": "string" },
                            "aspectRatio": { "type": "string" },
                            "folder": { "type": "string" }
                        }
                    }),
                    read_only: false,
                },
                ToolDef {
                    name: "design_update_artifact",
                    description: "Replace an artifact's body/css/js wholesale (creates a new version). Prefer design_edit_element for targeted changes.".into(),
                    input_schema: json!({
                        "type": "object", "required": ["artifactId"],
                        "properties": {
                            "artifactId": { "type": "string" },
                            "title": { "type": "string" },
                            "bodyHtml": { "type": "string" },
                            "css": { "type": "string" },
                            "js": { "type": "string" },
                            "versionMessage": { "type": "string" },
                            "expectedBodyHash": { "type": "string" }
                        }
                    }),
                    read_only: false,
                },
                ToolDef {
                    name: "design_edit_element",
                    description: "Precisely edit one element by oid (from design_get_artifact source): change style/text/attrs or remove it, keeping everything else. expectedBodyHash is required (read it from design_get_artifact first) to guard against a stale write.".into(),
                    input_schema: json!({
                        "type": "object", "required": ["artifactId", "oid", "expectedBodyHash"],
                        "properties": {
                            "artifactId": { "type": "string" },
                            "oid": { "type": "integer" },
                            "expectedBodyHash": { "type": "string" },
                            "text": { "type": "string" },
                            "style": { "type": "object" },
                            "attrs": { "type": "object" },
                            "remove": { "type": "boolean" }
                        }
                    }),
                    read_only: false,
                },
                ToolDef {
                    name: "design_restyle",
                    description: "Re-render an artifact under a different design system (no source edit). Omit systemId to clear the system.".into(),
                    input_schema: json!({
                        "type": "object", "required": ["artifactId"],
                        "properties": {
                            "artifactId": { "type": "string" },
                            "systemId": { "type": "string" }
                        }
                    }),
                    read_only: false,
                },
                ToolDef {
                    name: "design_restore_version",
                    description: "Restore an earlier version (non-destructive: creates a new version from the snapshot).".into(),
                    input_schema: json!({
                        "type": "object", "required": ["artifactId", "versionNumber"],
                        "properties": {
                            "artifactId": { "type": "string" },
                            "versionNumber": { "type": "integer" }
                        }
                    }),
                    read_only: false,
                },
                ToolDef {
                    name: "design_add_comment",
                    description: "Add a canvas comment to an artifact (optionally anchored to an oid).".into(),
                    input_schema: json!({
                        "type": "object", "required": ["artifactId", "body"],
                        "properties": {
                            "artifactId": { "type": "string" },
                            "body": { "type": "string" },
                            "oid": { "type": "integer" },
                            "tag": { "type": "string" },
                            "snippet": { "type": "string" },
                            "relX": { "type": "number" },
                            "relY": { "type": "number" }
                        }
                    }),
                    read_only: false,
                },
                ToolDef {
                    name: "design_resolve_comment",
                    description: "Mark a comment resolved (or reopen it) after addressing it.".into(),
                    input_schema: json!({
                        "type": "object", "required": ["artifactId", "commentId", "resolved"],
                        "properties": {
                            "artifactId": { "type": "string" },
                            "commentId": { "type": "integer" },
                            "resolved": { "type": "boolean" }
                        }
                    }),
                    read_only: false,
                },
            ]);
        }
        tools
    }

    fn call(&self, name: &str, args: Value, ctx: &McpCtx) -> Result<Value> {
        match name {
            // ── read ──
            "design_list_projects" => Ok(json!({ "projects": service::list_projects()? })),
            "design_list_artifacts" => {
                let pid = req_str(&args, "projectId")?;
                Ok(json!({ "projectId": pid, "artifacts": service::list_artifacts(pid)? }))
            }
            "design_get_artifact" => call_get_artifact(&args),
            "design_get_active_context" => {
                Ok(serde_json::to_value(service::get_active_context()?)?)
            }
            "design_list_systems" => Ok(json!({ "systems": service::list_systems()? })),
            "design_get_system" => call_get_system(&args),
            "design_list_comments" => {
                let aid = req_str(&args, "artifactId")?;
                let open_only = args
                    .get("openOnly")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut comments = service::list_comments(aid)?;
                if open_only {
                    comments.retain(|c| !c.resolved);
                }
                Ok(json!({ "artifactId": aid, "comments": comments }))
            }
            "design_list_versions" => {
                let aid = req_str(&args, "artifactId")?;
                Ok(json!({ "artifactId": aid, "versions": service::list_versions(aid)? }))
            }

            // ── write ──
            "design_generate_artifact" => call_generate(&args, ctx),
            "design_update_artifact" => call_update(&args),
            "design_edit_element" => call_edit_element(&args),
            "design_restyle" => {
                let aid = req_str(&args, "artifactId")?;
                let a = service::restyle_artifact(aid, opt_str(&args, "systemId"))?;
                Ok(
                    json!({ "status": "restyled", "artifactId": a.id, "version": a.current_version }),
                )
            }
            "design_restore_version" => {
                let aid = req_str(&args, "artifactId")?;
                let v = args
                    .get("versionNumber")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow!("missing required integer arg: versionNumber"))?;
                let a = service::restore_version(aid, v)?;
                Ok(
                    json!({ "status": "restored", "artifactId": a.id, "restoredFrom": v, "version": a.current_version }),
                )
            }
            "design_add_comment" => call_add_comment(&args),
            "design_resolve_comment" => {
                let aid = req_str(&args, "artifactId")?;
                let cid = args
                    .get("commentId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow!("missing required integer arg: commentId"))?;
                let resolved = args
                    .get("resolved")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| anyhow!("missing required boolean arg: resolved"))?;
                let ok = service::set_comment_resolved(aid, cid, resolved)?;
                Ok(
                    json!({ "artifactId": aid, "commentId": cid, "resolved": resolved, "updated": ok }),
                )
            }

            _ => Err(anyhow!("unknown design MCP tool: {name}")),
        }
    }
}

fn call_get_artifact(args: &Value) -> Result<Value> {
    let id = req_str(args, "artifactId")?;
    let include_source = args
        .get("includeSource")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let include_comments = args
        .get("includeComments")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let Some(view) = service::get_artifact_view(id)? else {
        return Err(anyhow!("artifact not found: {id}"));
    };
    let mut out = serde_json::to_value(&view)?;
    // generating 且 updated_at 落后过久 → 提示可能是被杀进程留下的孤儿（桌面产物墙才对账）。
    if view.artifact.status == "generating" {
        let stale = chrono::DateTime::parse_from_rfc3339(&view.artifact.updated_at)
            .map(|t| {
                (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds()
                    > GENERATING_STALE_SECS
            })
            .unwrap_or(false);
        if stale {
            out["maybeOrphaned"] = json!(true);
        }
    }
    if include_source {
        if let Some(src) = service::get_artifact_source_for_agent(id)? {
            out["source"] = serde_json::to_value(src)?;
        }
    }
    if include_comments {
        let comments = service::list_comments(id)?;
        out["comments"] = serde_json::to_value(comments)?;
    }
    Ok(out)
}

fn call_get_system(args: &Value) -> Result<Value> {
    let id = req_str(args, "systemId")?;
    let full = service::get_system_full(id)?;
    let mut out = json!({
        "meta": full.meta,
        "designMd": service::export_design_md(id)?,
    });
    if let Some(fmt) = opt_str(args, "tokenFormat") {
        let mut exports = service::export_tokens(id)?;
        exports.retain(|e| e.format == fmt);
        if exports.is_empty() {
            return Err(anyhow!(
                "unknown tokenFormat '{fmt}'; expected css/scss/ts/swift/android/dtcg"
            ));
        }
        out["tokens"] = serde_json::to_value(exports)?;
    }
    Ok(out)
}

fn call_generate(args: &Value, ctx: &McpCtx) -> Result<Value> {
    // brief → prompt；其余 camelCase 字段 serde default。session_id/model_override 恒缺省。
    let mut input = json!({
        "projectId": req_str(args, "projectId")?,
        "title": req_str(args, "title")?,
        "kind": req_str(args, "kind")?,
        "prompt": req_str(args, "brief")?,
    });
    for k in ["systemId", "recipeId", "aspectRatio", "folder"] {
        if let Some(v) = opt_str(args, k) {
            input[k] = json!(v);
        }
    }
    let create: service::CreateArtifactInput = serde_json::from_value(input)?;
    let artifact = ctx
        .runtime
        .block_on(service::generate_design_artifact(create))?;
    Ok(json!({
        "status": artifact.status, // "generating"（HTML 形态壳）| "ready"/"needs_review"（媒体阻塞完成）
        "artifactId": artifact.id,
        "kind": artifact.kind,
        "version": artifact.current_version,
        "hint": "Poll design_get_artifact until status != 'generating'."
    }))
}

fn call_update(args: &Value) -> Result<Value> {
    let id = req_str(args, "artifactId")?;
    let a = service::update_artifact(service::UpdateArtifactInput {
        id: id.to_string(),
        title: opt_str(args, "title").map(str::to_string),
        body_html: opt_str(args, "bodyHtml").map(str::to_string),
        css: opt_str(args, "css").map(str::to_string),
        js: opt_str(args, "js").map(str::to_string),
        message: opt_str(args, "versionMessage").map(str::to_string),
        origin: Some("ai".to_string()),
        prompt_summary: None,
        expected_body_hash: opt_str(args, "expectedBodyHash").map(str::to_string),
    })?;
    Ok(json!({ "status": "updated", "artifactId": a.id, "version": a.current_version }))
}

fn call_edit_element(args: &Value) -> Result<Value> {
    let id = req_str(args, "artifactId")?;
    let oid = args
        .get("oid")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| {
            anyhow!("missing or out-of-range integer arg: oid (from design_get_artifact source)")
        })?;
    // 跨进程无共享 artifact_lock → schema 层强制 expectedBodyHash（主动收紧）。
    let expected_hash = req_str(args, "expectedBodyHash")?.to_string();
    let text = opt_str(args, "text").map(str::to_string);
    let styles = parse_kv(args, "style");
    let attrs = parse_kv(args, "attrs");
    let remove = args.get("remove").and_then(Value::as_bool);
    if text.is_none() && styles.is_none() && attrs.is_none() && remove != Some(true) {
        return Err(anyhow!(
            "edit_element needs at least one of: style, text, attrs, or remove"
        ));
    }
    let a = service::patch_element(service::ElementPatch {
        artifact_id: id.to_string(),
        oid,
        text,
        styles,
        attrs,
        remove,
        text_node: None, // owner 可视化专属，MCP 面不暴露
        expected_hash: Some(expected_hash),
    })?;
    Ok(json!({ "status": "patched", "artifactId": a.id, "oid": oid, "version": a.current_version }))
}

fn call_add_comment(args: &Value) -> Result<Value> {
    let aid = req_str(args, "artifactId")?;
    let body = req_str(args, "body")?;
    let oid = args.get("oid").and_then(Value::as_i64);
    let rel_x = args.get("relX").and_then(Value::as_f64).unwrap_or(0.5);
    let rel_y = args.get("relY").and_then(Value::as_f64).unwrap_or(0.5);
    let c = service::add_comment(
        aid,
        oid,
        rel_x,
        rel_y,
        opt_str(args, "tag"),
        opt_str(args, "snippet"),
        body,
    )?;
    Ok(json!({ "status": "commented", "artifactId": aid, "commentId": c.id }))
}
