use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

use super::ToolExecContext;
use crate::session::SessionDB;
use crate::team;

/// Tool handler for the `team` tool.
pub(crate) async fn tool_team(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "create" => action_create(args, ctx).await,
        "dissolve" => action_dissolve(args).await,
        "add_member" => action_add_member(args).await,
        "remove_member" => action_remove_member(args).await,
        "send_message" => action_send_message(args, ctx).await,
        "create_task" => action_create_task(args).await,
        "update_task" => action_update_task(args).await,
        "list_tasks" => action_list_tasks(args).await,
        "list_members" => action_list_members(args).await,
        "status" => action_status(args).await,
        "pause" => action_pause(args).await,
        "resume" => action_resume(args).await,
        "list_templates" => action_list_templates().await,
        _ => Err(anyhow::anyhow!(
            "Unknown team action '{}'. Valid actions: create, dissolve, add_member, remove_member, \
             send_message, create_task, update_task, list_tasks, list_members, status, pause, \
             resume, list_templates",
            action
        )),
    }
}

fn require_db() -> Result<Arc<SessionDB>> {
    crate::require_session_db().map(Arc::clone)
}

fn require_team_id(args: &Value) -> Result<String> {
    args.get("team_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("'team_id' is required"))
}

// ── Actions ─────────────────────────────────────────────────────

async fn action_create(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'name' is required for create"))?;
    let description = args.get("description").and_then(|v| v.as_str());

    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;
    let agent_id = ctx.agent_id.as_deref().unwrap_or("default");

    // Resolved template (used both as DB source and to stamp team.template_id)
    let template = if let Some(key) = args.get("template").and_then(|v| v.as_str()) {
        let templates = team::templates::all_templates(&db);
        let found = templates
            .into_iter()
            .find(|t| t.template_id == key || t.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Template '{}' not found. Call team(action=\"list_templates\") to see available presets.",
                    key
                )
            })?;
        Some(found)
    } else {
        None
    };

    // Parse member specs (inline members override template members)
    let member_specs: Vec<team::CreateTeamMemberSpec> = if let Some(members) = args.get("members") {
        serde_json::from_value(members.clone())?
    } else if let Some(tpl) = template.as_ref() {
        tpl.members
            .iter()
            .map(|m| team::CreateTeamMemberSpec {
                name: m.name.clone(),
                agent_id: m.agent_id.clone(),
                role: Some(m.role.as_str().to_string()),
                task: m
                    .default_task_template
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| {
                        format!("Work on your role '{}' as part of team '{}'.", m.name, name)
                    }),
                model: m.model_override.clone(),
                description: Some(m.description.clone()).filter(|s| !s.trim().is_empty()),
            })
            .collect()
    } else {
        return Err(anyhow::anyhow!(
            "'members' array or 'template' name is required for create. \
             Call team(action=\"list_templates\") first to check for a matching preset."
        ));
    };

    let template_id = template.as_ref().map(|t| t.template_id.as_str());

    let created = team::coordinator::create_team(
        &db,
        name,
        description,
        session_id,
        agent_id,
        &member_specs,
        template_id,
        None,
    )
    .await?;

    let members = db.list_team_members(&created.team_id)?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "created",
        "teamId": created.team_id,
        "name": created.name,
        "templateId": created.template_id,
        "memberCount": members.len(),
        "members": members.iter().map(|m| serde_json::json!({
            "name": m.name,
            "memberId": m.member_id,
            "agentId": m.agent_id,
            "role": m.role.as_str(),
            "status": m.status.as_str(),
        })).collect::<Vec<_>>(),
    }))?)
}

async fn action_list_templates() -> Result<String> {
    let db = require_db()?;
    let templates = team::templates::all_templates(&db);

    let summaries: Vec<serde_json::Value> = templates
        .iter()
        .map(|t| {
            serde_json::json!({
                "templateId": t.template_id,
                "name": t.name,
                "description": t.description,
                "memberCount": t.members.len(),
                "members": t.members.iter().map(|m| serde_json::json!({
                    "name": m.name,
                    "role": m.role.as_str(),
                    "agentId": m.agent_id,
                    "description": m.description,
                    "modelOverride": m.model_override,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "total": summaries.len(),
        "templates": summaries,
        "hint": if summaries.is_empty() {
            "No user-configured team templates. Define members inline via the `members` argument in action=\"create\"."
        } else {
            "Pick a template whose member roles match your task, then call team(action=\"create\", name=..., template=\"<templateId>\"). Override per-member `task` via the `members` array if needed."
        },
    }))?)
}

async fn action_dissolve(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    team::coordinator::dissolve_team(&db, &team_id)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "dissolved",
        "teamId": team_id,
    }))?)
}

async fn action_add_member(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'name' is required"))?;
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let role = args
        .get("role")
        .and_then(|v| v.as_str())
        .map(team::MemberRole::from_str)
        .unwrap_or(team::MemberRole::Worker);
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'task' is required"))?;
    let model = args.get("model").and_then(|v| v.as_str());
    let description = args.get("description").and_then(|v| v.as_str());

    let member = team::coordinator::add_member(
        &db,
        &team_id,
        name,
        agent_id,
        role,
        task,
        model,
        description,
    )
    .await?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "added",
        "memberId": member.member_id,
        "name": member.name,
        "role": member.role.as_str(),
    }))?)
}

async fn action_remove_member(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let member_id = args
        .get("member_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'member_id' is required"))?;

    team::coordinator::remove_member(&db, &team_id, member_id)?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "removed",
        "memberId": member_id,
    }))?)
}

async fn action_send_message(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let to = args.get("to").and_then(|v| v.as_str());
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'content' is required"))?;

    // Determine sender: check if we're a team member (via session_id) or the lead
    let from = if let Some(session_id) = ctx.session_id.as_deref() {
        // Check if this session belongs to a team member
        let members = db.list_team_members(&team_id)?;
        members
            .iter()
            .find(|m| m.session_id.as_deref() == Some(session_id))
            .map(|m| m.member_id.clone())
            .unwrap_or_else(|| "*lead*".to_string())
    } else {
        "*lead*".to_string()
    };

    // Resolve 'to' — could be a member name
    let to_resolved = if let Some(name) = to {
        if name == "*" {
            None
        } else {
            // Try to find member by name
            let member = db.find_team_member_by_name(&team_id, name)?;
            Some(
                member
                    .map(|m| m.member_id)
                    .unwrap_or_else(|| name.to_string()),
            )
        }
    } else {
        None
    };

    let msg = team::messaging::send_message(
        &db,
        &team_id,
        &from,
        to_resolved.as_deref(),
        content,
        team::TeamMessageType::Chat,
    )?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "sent",
        "messageId": msg.message_id,
        "to": to.unwrap_or("*"),
    }))?)
}

async fn action_create_task(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'content' is required"))?;
    let owner = args.get("owner").and_then(|v| v.as_str());
    let priority = args
        .get("priority")
        .and_then(|v| v.as_u64())
        .map(|p| p as u32);
    let blocked_by: Vec<i64> = args
        .get("blocked_by")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default();

    // Resolve owner name to member_id
    let owner_id = if let Some(name) = owner {
        db.find_team_member_by_name(&team_id, name)?
            .map(|m| m.member_id)
            .or_else(|| Some(name.to_string()))
    } else {
        None
    };

    let task = team::tasks::create_task(
        &db,
        &team_id,
        content,
        owner_id.as_deref(),
        priority,
        blocked_by,
    )?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "created",
        "taskId": task.id,
        "content": task.content,
        "owner": task.owner_member_id,
        "column": task.column_name,
    }))?)
}

async fn action_update_task(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("'task_id' is required"))?;
    let status = args.get("status").and_then(|v| v.as_str());
    let owner = args.get("owner").and_then(|v| v.as_str());
    let column = args.get("column").and_then(|v| v.as_str());
    let content = args.get("content").and_then(|v| v.as_str());

    let task = team::tasks::update_task(&db, &team_id, task_id, status, owner, column, content)?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "updated",
        "task": task,
    }))?)
}

async fn action_list_tasks(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let status_filter = args.get("status").and_then(|v| v.as_str());

    let tasks = team::tasks::list_tasks(&db, &team_id, status_filter)?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "teamId": team_id,
        "total": tasks.len(),
        "tasks": tasks,
    }))?)
}

async fn action_list_members(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let members = db.list_team_members(&team_id)?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "teamId": team_id,
        "total": members.len(),
        "members": members.iter().map(|m| serde_json::json!({
            "memberId": m.member_id,
            "name": m.name,
            "role": m.role.as_str(),
            "status": m.status.as_str(),
            "currentTaskId": m.current_task_id,
            "inputTokens": m.input_tokens,
            "outputTokens": m.output_tokens,
        })).collect::<Vec<_>>(),
    }))?)
}

async fn action_status(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let status = team::coordinator::get_team_status(&db, &team_id)?;
    Ok(serde_json::to_string_pretty(&status)?)
}

async fn action_pause(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    team::coordinator::pause_team(&db, &team_id)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "paused",
        "teamId": team_id,
    }))?)
}

async fn action_resume(args: &Value) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    team::coordinator::resume_team(&db, &team_id).await?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "resumed",
        "teamId": team_id,
    }))?)
}
