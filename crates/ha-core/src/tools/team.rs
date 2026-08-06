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
        "dissolve" => action_dissolve(args, ctx).await,
        "add_member" => action_add_member(args, ctx).await,
        "remove_member" => action_remove_member(args, ctx).await,
        "send_message" => action_send_message(args, ctx).await,
        "create_task" => action_create_task(args, ctx).await,
        "update_task" => action_update_task(args, ctx).await,
        "list_tasks" => action_list_tasks(args, ctx).await,
        "list_members" => action_list_members(args, ctx).await,
        "status" => action_status(args, ctx).await,
        "pause" => action_pause(args, ctx).await,
        "resume" => action_resume(args, ctx).await,
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

const TEAM_NOT_CONTROLLED: &str = "Team was not found or is not controlled by the current session";

#[derive(Debug, PartialEq, Eq)]
enum TeamCaller {
    Lead,
    Member(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // collaboration writes enforce the same variants inside typed DB transactions
enum TeamToolAction {
    Dissolve,
    AddMember,
    RemoveMember,
    SendMessage,
    CreateTask,
    UpdateTask,
    ListTasks,
    ListMembers,
    Status,
    Pause,
    Resume,
}

fn team_caller_for_session(
    team: &team::Team,
    members: &[team::TeamMember],
    session_id: Option<&str>,
) -> Option<TeamCaller> {
    let session_id = session_id?;
    if team.lead_session_id == session_id {
        return Some(TeamCaller::Lead);
    }
    members
        .iter()
        .find(|member| {
            member.team_id == team.team_id && member.session_id.as_deref() == Some(session_id)
        })
        .map(|member| TeamCaller::Member(member.member_id.clone()))
}

fn team_action_allowed(
    action: TeamToolAction,
    team_status: &team::TeamStatus,
    caller: &TeamCaller,
) -> bool {
    match caller {
        TeamCaller::Lead => match action {
            TeamToolAction::ListTasks | TeamToolAction::ListMembers | TeamToolAction::Status => {
                true
            }
            TeamToolAction::Dissolve => {
                matches!(
                    team_status,
                    team::TeamStatus::Active | team::TeamStatus::Paused
                )
            }
            TeamToolAction::AddMember
            | TeamToolAction::RemoveMember
            | TeamToolAction::SendMessage
            | TeamToolAction::CreateTask
            | TeamToolAction::UpdateTask
            | TeamToolAction::Pause => team_status == &team::TeamStatus::Active,
            TeamToolAction::Resume => team_status == &team::TeamStatus::Paused,
        },
        TeamCaller::Member(_) => {
            team_status == &team::TeamStatus::Active
                && matches!(
                    action,
                    TeamToolAction::SendMessage
                        | TeamToolAction::CreateTask
                        | TeamToolAction::UpdateTask
                        | TeamToolAction::ListTasks
                        | TeamToolAction::ListMembers
                        | TeamToolAction::Status
                )
        }
    }
}

/// Authenticate every opaque `team_id` action at the tool boundary. A team is
/// visible only to its exact lead session and the durable child sessions of
/// that same team. Keeping the error generic avoids turning status/mutation
/// calls into a cross-session existence oracle.
async fn require_team_caller(
    db: &Arc<SessionDB>,
    team_id: &str,
    ctx: &ToolExecContext,
    action: TeamToolAction,
) -> Result<TeamCaller> {
    let db = db.clone();
    let team_id = team_id.to_string();
    let session_id = ctx.session_id.clone();
    db.run(move |db| require_team_caller_in_db(db, &team_id, session_id.as_deref(), action))
        .await
}

fn require_team_caller_in_db(
    db: &SessionDB,
    team_id: &str,
    session_id: Option<&str>,
    action: TeamToolAction,
) -> Result<TeamCaller> {
    let Some(team) = db.get_team(team_id)? else {
        return Err(anyhow::anyhow!(TEAM_NOT_CONTROLLED));
    };
    let members = db.list_team_members(team_id)?;
    let caller = team_caller_for_session(&team, &members, session_id)
        .ok_or_else(|| anyhow::anyhow!(TEAM_NOT_CONTROLLED))?;
    if !team_action_allowed(action, &team.status, &caller) {
        return Err(anyhow::anyhow!(TEAM_NOT_CONTROLLED));
    }
    if let TeamCaller::Member(ref member_id) = caller {
        let member = members
            .iter()
            .find(|member| member.member_id == *member_id)
            .ok_or_else(|| anyhow::anyhow!(TEAM_NOT_CONTROLLED))?;
        if !member.status.is_active() {
            return Err(anyhow::anyhow!(TEAM_NOT_CONTROLLED));
        }
        let (Some(run_id), Some(member_session_id)) =
            (member.run_id.as_deref(), member.session_id.as_deref())
        else {
            return Err(anyhow::anyhow!(TEAM_NOT_CONTROLLED));
        };
        let run = db
            .get_subagent_run(run_id)?
            .ok_or_else(|| anyhow::anyhow!(TEAM_NOT_CONTROLLED))?;
        if run.status.is_terminal()
            || run.child_session_id != member_session_id
            || run.parent_session_id != team.lead_session_id
            || run.owner_kind != crate::subagent::SubagentOwnerKind::Team
            || run.owner_id != team.team_id
        {
            return Err(anyhow::anyhow!(TEAM_NOT_CONTROLLED));
        }
    }
    Ok(caller)
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
    let agent_id = ctx
        .agent_id
        .as_deref()
        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);

    // Resolved template (used both as DB source and to stamp team.template_id)
    let template = if let Some(key) = args.get("template").and_then(|v| v.as_str()) {
        let key = key.to_string();
        let templates = {
            let db = db.clone();
            db.run(|db| team::templates::all_templates(db)).await
        };
        let found = templates
            .into_iter()
            .find(|t| t.template_id == key || t.name.eq_ignore_ascii_case(&key))
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

    let members = {
        let db = db.clone();
        let team_id = created.team_id.clone();
        db.run(move |db| db.list_team_members(&team_id)).await?
    };

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
    let templates = db.run(team::templates::all_templates).await;

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

async fn action_dissolve(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    require_team_caller(&db, &team_id, ctx, TeamToolAction::Dissolve).await?;
    let result = team::coordinator::dissolve_team(&db, &team_id).await?;
    Ok(serde_json::to_string_pretty(&result)?)
}

async fn action_add_member(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    require_team_caller(&db, &team_id, ctx, TeamToolAction::AddMember).await?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'name' is required"))?;
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);
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

async fn action_remove_member(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    require_team_caller(&db, &team_id, ctx, TeamToolAction::RemoveMember).await?;
    let member_id = args
        .get("member_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'member_id' is required"))?;

    let result = team::coordinator::remove_member(&db, &team_id, member_id).await?;
    Ok(serde_json::to_string_pretty(&result)?)
}

async fn action_send_message(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let to = args.get("to").and_then(|v| v.as_str());
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'content' is required"))?;
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(TEAM_NOT_CONTROLLED))?;
    let msg =
        team::messaging::send_message_as_session(&db, &team_id, session_id, to, content).await?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "sent",
        "messageId": msg.message_id,
        "to": to.unwrap_or("*"),
    }))?)
}

async fn action_create_task(args: &Value, ctx: &ToolExecContext) -> Result<String> {
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
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(TEAM_NOT_CONTROLLED))?;
    let task = team::tasks::create_task_as_session(
        &db, &team_id, session_id, content, owner, priority, blocked_by,
    )
    .await?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "created",
        "taskId": task.id,
        "content": task.content,
        "owner": task.owner_member_id,
        "column": task.column_name,
    }))?)
}

async fn action_update_task(args: &Value, ctx: &ToolExecContext) -> Result<String> {
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
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(TEAM_NOT_CONTROLLED))?;
    let task = team::tasks::update_task_as_session(
        &db, &team_id, session_id, task_id, status, owner, column, content,
    )
    .await?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "updated",
        "task": task,
    }))?)
}

async fn action_list_tasks(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let session_id = ctx.session_id.clone();
    let status_filter = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let tasks = {
        let db = db.clone();
        let query_team_id = team_id.clone();
        db.run(move |db| {
            require_team_caller_in_db(
                db,
                &query_team_id,
                session_id.as_deref(),
                TeamToolAction::ListTasks,
            )?;
            team::tasks::list_tasks(db, &query_team_id, status_filter.as_deref())
        })
        .await?
    };

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "teamId": team_id,
        "total": tasks.len(),
        "tasks": tasks,
    }))?)
}

async fn action_list_members(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let session_id = ctx.session_id.clone();
    let members = {
        let db = db.clone();
        let query_team_id = team_id.clone();
        db.run(move |db| {
            require_team_caller_in_db(
                db,
                &query_team_id,
                session_id.as_deref(),
                TeamToolAction::ListMembers,
            )?;
            db.list_team_members(&query_team_id)
        })
        .await?
    };

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

async fn action_status(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    let session_id = ctx.session_id.clone();
    let status = {
        let db = db.clone();
        let query_team_id = team_id.clone();
        db.run(move |db| {
            require_team_caller_in_db(
                db,
                &query_team_id,
                session_id.as_deref(),
                TeamToolAction::Status,
            )?;
            team::coordinator::get_team_status(db, &query_team_id)
        })
        .await?
    };
    Ok(serde_json::to_string_pretty(&status)?)
}

async fn action_pause(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    require_team_caller(&db, &team_id, ctx, TeamToolAction::Pause).await?;
    let result = team::coordinator::pause_team(&db, &team_id).await?;
    Ok(serde_json::to_string_pretty(&result)?)
}

async fn action_resume(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let db = require_db()?;
    let team_id = require_team_id(args)?;
    require_team_caller(&db, &team_id, ctx, TeamToolAction::Resume).await?;
    let result = team::coordinator::resume_team(&db, &team_id).await?;
    Ok(serde_json::to_string_pretty(&result)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team_fixture() -> team::Team {
        team::Team {
            team_id: "team-1".into(),
            name: "test".into(),
            description: None,
            lead_session_id: "lead-session".into(),
            lead_agent_id: "agent-1".into(),
            status: team::TeamStatus::Active,
            created_at: String::new(),
            updated_at: String::new(),
            template_id: None,
            config: team::TeamConfig::default(),
        }
    }

    fn member_fixture() -> team::TeamMember {
        team::TeamMember {
            member_id: "member-1".into(),
            team_id: "team-1".into(),
            name: "worker".into(),
            agent_id: "agent-1".into(),
            role: team::MemberRole::Worker,
            status: team::MemberStatus::Idle,
            run_id: None,
            session_id: Some("member-session".into()),
            color: String::new(),
            current_task_id: None,
            model_override: None,
            role_description: None,
            joined_at: String::new(),
            last_active_at: None,
            input_tokens: None,
            output_tokens: None,
        }
    }

    #[test]
    fn team_control_accepts_only_exact_lead_or_same_team_member_session() {
        let team = team_fixture();
        let members = vec![member_fixture()];

        assert_eq!(
            team_caller_for_session(&team, &members, Some("lead-session")),
            Some(TeamCaller::Lead)
        );
        assert_eq!(
            team_caller_for_session(&team, &members, Some("member-session")),
            Some(TeamCaller::Member("member-1".into()))
        );
        assert_eq!(
            team_caller_for_session(&team, &members, Some("other-session")),
            None
        );
        assert_eq!(team_caller_for_session(&team, &members, None), None);
    }

    #[test]
    fn member_session_from_another_team_does_not_grant_control() {
        let team = team_fixture();
        let mut foreign_member = member_fixture();
        foreign_member.team_id = "team-2".into();

        // The authorization helper is always passed rows loaded by team_id;
        // retain a defensive team-id check so even a future broader query
        // cannot accidentally turn member session lineage into global access.
        assert_eq!(
            team_caller_for_session(&team, &[foreign_member], Some("member-session")),
            None
        );
    }

    #[test]
    fn team_action_policy_is_lead_only_for_lifecycle_and_state_aware() {
        let lead = TeamCaller::Lead;
        let member = TeamCaller::Member("member-1".into());
        let collaboration = [
            TeamToolAction::SendMessage,
            TeamToolAction::CreateTask,
            TeamToolAction::UpdateTask,
            TeamToolAction::ListTasks,
            TeamToolAction::ListMembers,
            TeamToolAction::Status,
        ];
        let lifecycle = [
            TeamToolAction::Dissolve,
            TeamToolAction::AddMember,
            TeamToolAction::RemoveMember,
            TeamToolAction::Pause,
            TeamToolAction::Resume,
        ];

        for action in collaboration {
            assert!(team_action_allowed(
                action,
                &team::TeamStatus::Active,
                &lead
            ));
            assert!(team_action_allowed(
                action,
                &team::TeamStatus::Active,
                &member
            ));
            assert!(!team_action_allowed(
                action,
                &team::TeamStatus::Paused,
                &member
            ));
        }
        for action in lifecycle {
            assert!(!team_action_allowed(
                action,
                &team::TeamStatus::Active,
                &member
            ));
        }

        assert!(team_action_allowed(
            TeamToolAction::Pause,
            &team::TeamStatus::Active,
            &lead
        ));
        assert!(!team_action_allowed(
            TeamToolAction::Resume,
            &team::TeamStatus::Active,
            &lead
        ));
        assert!(team_action_allowed(
            TeamToolAction::Resume,
            &team::TeamStatus::Paused,
            &lead
        ));
        assert!(team_action_allowed(
            TeamToolAction::Dissolve,
            &team::TeamStatus::Paused,
            &lead
        ));
        assert!(!team_action_allowed(
            TeamToolAction::Pause,
            &team::TeamStatus::Paused,
            &lead
        ));
        assert!(team_action_allowed(
            TeamToolAction::Status,
            &team::TeamStatus::Dissolved,
            &lead
        ));
        assert!(!team_action_allowed(
            TeamToolAction::Dissolve,
            &team::TeamStatus::Dissolved,
            &lead
        ));
    }
}
