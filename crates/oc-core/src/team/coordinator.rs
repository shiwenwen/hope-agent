use anyhow::Result;
use std::sync::Arc;

use crate::globals::{get_session_db, get_subagent_cancels};
use crate::session::SessionDB;
use crate::subagent::{self, SpawnParams, SubagentCancelRegistry};
use super::events::emit_team_event;
use super::types::*;
use super::{pick_member_color, MAX_ACTIVE_TEAMS};

/// Create a new team with initial members.
pub async fn create_team(
    db: &Arc<SessionDB>,
    name: &str,
    description: Option<&str>,
    lead_session_id: &str,
    lead_agent_id: &str,
    member_specs: &[CreateTeamMemberSpec],
    template_id: Option<&str>,
    config: Option<TeamConfig>,
) -> Result<Team> {
    // Check active team limit
    let active_count = db.count_active_teams_for_agent(lead_agent_id)?;
    if active_count >= MAX_ACTIVE_TEAMS as usize {
        return Err(anyhow::anyhow!(
            "Maximum active teams ({}) reached for this agent",
            MAX_ACTIVE_TEAMS
        ));
    }

    let team_config = config.unwrap_or_default();
    if member_specs.len() > team_config.max_members as usize {
        return Err(anyhow::anyhow!(
            "Too many members ({}), max is {}",
            member_specs.len(),
            team_config.max_members
        ));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let team = Team {
        team_id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        description: description.map(|s| s.to_string()),
        lead_session_id: lead_session_id.to_string(),
        lead_agent_id: lead_agent_id.to_string(),
        status: TeamStatus::Active,
        created_at: now.clone(),
        updated_at: now,
        template_id: template_id.map(|s| s.to_string()),
        config: team_config,
    };

    db.insert_team(&team)?;
    emit_team_event("created", &team);

    // Spawn each member
    let cancel_registry = get_subagent_cancels()
        .cloned()
        .unwrap_or_else(|| Arc::new(SubagentCancelRegistry::new()));

    for (i, spec) in member_specs.iter().enumerate() {
        let role = spec
            .role
            .as_deref()
            .map(MemberRole::from_str)
            .unwrap_or(MemberRole::Worker);

        if let Err(e) = spawn_member(
            db,
            &cancel_registry,
            &team,
            &spec.name,
            &spec.agent_id,
            role,
            &spec.task,
            spec.model.as_deref(),
            spec.description.as_deref(),
            i,
        )
        .await
        {
            app_warn!(
                "team",
                "coordinator",
                "Failed to spawn member {}: {}",
                spec.name,
                e
            );
        }
    }

    Ok(team)
}

/// Spawn a single team member as a subagent.
pub async fn spawn_member(
    db: &Arc<SessionDB>,
    cancel_registry: &Arc<SubagentCancelRegistry>,
    team: &Team,
    name: &str,
    agent_id: &str,
    role: MemberRole,
    task: &str,
    model_override: Option<&str>,
    role_description: Option<&str>,
    color_index: usize,
) -> Result<TeamMember> {
    let member_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let member = TeamMember {
        member_id: member_id.clone(),
        team_id: team.team_id.clone(),
        name: name.to_string(),
        agent_id: agent_id.to_string(),
        role: role.clone(),
        status: MemberStatus::Idle,
        run_id: None,
        session_id: None,
        color: pick_member_color(color_index).to_string(),
        current_task_id: None,
        model_override: model_override.map(|s| s.to_string()),
        role_description: role_description
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        joined_at: now,
        last_active_at: None,
        input_tokens: Some(0),
        output_tokens: Some(0),
    };

    db.insert_team_member(&member)?;
    emit_team_event("member_joined", &member);

    // Build team context for the member's system prompt
    let extra_context = build_member_context(db, team, &member, task)?;

    let params = SpawnParams {
        task: task.to_string(),
        agent_id: agent_id.to_string(),
        parent_session_id: team.lead_session_id.clone(),
        parent_agent_id: team.lead_agent_id.clone(),
        depth: 1,
        timeout_secs: None, // use agent default
        model_override: model_override.map(|s| s.to_string()),
        label: Some(format!("team:{}/{}", team.name, name)),
        attachments: Vec::new(),
        plan_agent_mode: None,
        plan_mode_allow_paths: Vec::new(),
        skip_parent_injection: true, // coordinator handles result routing
        extra_system_context: Some(extra_context),
        skill_allowed_tools: Vec::new(),
        reasoning_effort: None,
        skill_name: None,
    };

    let run_id = subagent::spawn_subagent(params, db.clone(), cancel_registry.clone()).await?;

    // Look up the child_session_id from the subagent run
    let session_id = db
        .get_subagent_run(&run_id)?
        .map(|r| r.child_session_id)
        .unwrap_or_default();

    db.update_team_member_run(&member_id, &run_id, &session_id)?;

    let mut updated = member;
    updated.run_id = Some(run_id);
    updated.session_id = Some(session_id);
    updated.status = MemberStatus::Working;

    emit_team_event("member_status", &serde_json::json!({
        "teamId": team.team_id,
        "memberId": member_id,
        "status": "working",
    }));

    Ok(updated)
}

/// Add a member to an existing active team.
pub async fn add_member(
    db: &Arc<SessionDB>,
    team_id: &str,
    name: &str,
    agent_id: &str,
    role: MemberRole,
    task: &str,
    model_override: Option<&str>,
    role_description: Option<&str>,
) -> Result<TeamMember> {
    let team = db
        .get_team(team_id)?
        .ok_or_else(|| anyhow::anyhow!("Team {} not found", team_id))?;

    if !team.status.is_active() {
        return Err(anyhow::anyhow!("Team is not active"));
    }

    let members = db.list_team_members(team_id)?;
    if members.len() >= team.config.max_members as usize {
        return Err(anyhow::anyhow!(
            "Team already has {} members (max {})",
            members.len(),
            team.config.max_members
        ));
    }

    let cancel_registry = get_subagent_cancels()
        .cloned()
        .unwrap_or_else(|| Arc::new(SubagentCancelRegistry::new()));

    let member = spawn_member(
        db,
        &cancel_registry,
        &team,
        name,
        agent_id,
        role,
        task,
        model_override,
        role_description,
        members.len(),
    )
    .await?;

    let _ = super::messaging::post_system_message(
        db,
        team_id,
        &format!("{} joined the team", name),
    );

    Ok(member)
}

/// Remove a member from the team (kills the subagent).
pub fn remove_member(db: &Arc<SessionDB>, team_id: &str, member_id: &str) -> Result<()> {
    let member = db
        .get_team_member(member_id)?
        .ok_or_else(|| anyhow::anyhow!("Member {} not found", member_id))?;

    if member.team_id != team_id {
        return Err(anyhow::anyhow!("Member does not belong to this team"));
    }

    // Kill the subagent if active
    if member.status.is_active() {
        if let Some(ref run_id) = member.run_id {
            if let Some(registry) = get_subagent_cancels() {
                registry.cancel(run_id);
            }
        }
    }

    db.update_team_member_status(member_id, &MemberStatus::Killed)?;

    emit_team_event("member_status", &serde_json::json!({
        "teamId": team_id,
        "memberId": member_id,
        "status": "killed",
    }));

    let _ = super::messaging::post_system_message(
        db,
        team_id,
        &format!("{} was removed from the team", member.name),
    );

    Ok(())
}

/// Dissolve a team — kill all active members and mark as dissolved.
pub fn dissolve_team(db: &Arc<SessionDB>, team_id: &str) -> Result<()> {
    let team = db
        .get_team(team_id)?
        .ok_or_else(|| anyhow::anyhow!("Team {} not found", team_id))?;

    let members = db.list_team_members(team_id)?;

    // Kill all active members
    if let Some(registry) = get_subagent_cancels() {
        for member in &members {
            if member.status.is_active() {
                if let Some(ref run_id) = member.run_id {
                    registry.cancel(run_id);
                }
                let _ = db.update_team_member_status(&member.member_id, &MemberStatus::Killed);
            }
        }
    }

    db.update_team_status(team_id, &TeamStatus::Dissolved)?;

    emit_team_event("dissolved", &serde_json::json!({
        "teamId": team_id,
        "name": team.name,
    }));

    app_info!("team", "coordinator", "Team '{}' dissolved", team.name);
    Ok(())
}

/// Pause a team — kill all active members but mark as paused (can resume).
pub fn pause_team(db: &Arc<SessionDB>, team_id: &str) -> Result<()> {
    let members = db.list_team_members(team_id)?;

    if let Some(registry) = get_subagent_cancels() {
        for member in &members {
            if member.status.is_active() {
                if let Some(ref run_id) = member.run_id {
                    registry.cancel(run_id);
                }
                let _ = db.update_team_member_status(&member.member_id, &MemberStatus::Paused);
            }
        }
    }

    db.update_team_status(team_id, &TeamStatus::Paused)?;

    emit_team_event("paused", &serde_json::json!({ "teamId": team_id }));

    Ok(())
}

/// Resume a paused team — re-spawn paused members with their original tasks.
pub async fn resume_team(db: &Arc<SessionDB>, team_id: &str) -> Result<()> {
    let team = db
        .get_team(team_id)?
        .ok_or_else(|| anyhow::anyhow!("Team {} not found", team_id))?;

    if team.status != TeamStatus::Paused {
        return Err(anyhow::anyhow!("Team is not paused"));
    }

    db.update_team_status(team_id, &TeamStatus::Active)?;

    let members = db.list_team_members(team_id)?;
    let cancel_registry = get_subagent_cancels()
        .cloned()
        .unwrap_or_else(|| Arc::new(SubagentCancelRegistry::new()));

    for (i, member) in members.iter().enumerate() {
        if member.status != MemberStatus::Paused {
            continue;
        }

        // Find the member's current task
        let task_text = if let Some(task_id) = member.current_task_id {
            db.get_team_task(task_id)?
                .map(|t| t.content)
                .unwrap_or_else(|| format!("Continue previous work as {}", member.name))
        } else {
            format!("Continue previous work as {}", member.name)
        };

        if let Err(e) = spawn_member(
            db,
            &cancel_registry,
            &team,
            &member.name,
            &member.agent_id,
            member.role.clone(),
            &task_text,
            member.model_override.as_deref(),
            member.role_description.as_deref(),
            i,
        )
        .await
        {
            app_warn!(
                "team",
                "coordinator",
                "Failed to resume member {}: {}",
                member.name,
                e
            );
        }
    }

    emit_team_event("resumed", &serde_json::json!({ "teamId": team_id }));

    Ok(())
}

/// Get a summary of the team status.
pub fn get_team_status(db: &Arc<SessionDB>, team_id: &str) -> Result<serde_json::Value> {
    let team = db
        .get_team(team_id)?
        .ok_or_else(|| anyhow::anyhow!("Team {} not found", team_id))?;
    let members = db.list_team_members(team_id)?;
    let tasks = db.list_team_tasks(team_id)?;

    let active = members.iter().filter(|m| m.status.is_active()).count();
    let completed = members
        .iter()
        .filter(|m| m.status == MemberStatus::Completed)
        .count();
    let tasks_done = tasks.iter().filter(|t| t.status == "completed").count();
    let total_input: u64 = members.iter().filter_map(|m| m.input_tokens).sum();
    let total_output: u64 = members.iter().filter_map(|m| m.output_tokens).sum();

    Ok(serde_json::json!({
        "team": team,
        "members": members,
        "tasks": tasks,
        "summary": {
            "totalMembers": members.len(),
            "activeMembers": active,
            "completedMembers": completed,
            "totalTasks": tasks.len(),
            "completedTasks": tasks_done,
            "totalInputTokens": total_input,
            "totalOutputTokens": total_output,
        }
    }))
}

/// Build the extra system context injected into a team member's subagent.
fn build_member_context(
    db: &Arc<SessionDB>,
    team: &Team,
    member: &TeamMember,
    task: &str,
) -> Result<String> {
    let members = db.list_team_members(&team.team_id)?;
    // Batch-fetch all tasks to avoid N+1 queries
    let tasks = db.list_team_tasks(&team.team_id)?;
    let task_map: std::collections::HashMap<i64, &str> = tasks
        .iter()
        .map(|t| (t.id, t.content.as_str()))
        .collect();

    let teammates: Vec<String> = members
        .iter()
        .filter(|m| m.member_id != member.member_id)
        .map(|m| {
            let task_desc = m
                .current_task_id
                .and_then(|id| task_map.get(&id).copied())
                .unwrap_or("awaiting assignment");
            format!("- {} ({:?}): {}", m.name, m.role, task_desc)
        })
        .collect();

    let shared_ctx = team
        .config
        .shared_context
        .as_deref()
        .unwrap_or("");

    let role_identity_block = member
        .role_description
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| format!("### Your Role Identity\n{}\n\n", s))
        .unwrap_or_default();

    Ok(format!(
        r#"## Team Collaboration Context
You are a member of team "{}".
- Your name: {}
- Your role: {:?}

{}### Teammates
{}

### Communication
- Send message to a teammate: team(action="send_message", team_id="{}", to="<name>", content="...")
- Broadcast to all: team(action="send_message", team_id="{}", to="*", content="...")
- Update your task: team(action="update_task", team_id="{}", task_id=<id>, status="completed")
- Create a new task: team(action="create_task", team_id="{}", content="...", owner="{}")

### Your Assignment
{}

{}
"#,
        team.name,
        member.name,
        member.role,
        role_identity_block,
        if teammates.is_empty() {
            "(no other members yet)".to_string()
        } else {
            teammates.join("\n")
        },
        team.team_id,
        team.team_id,
        team.team_id,
        team.team_id,
        member.name,
        task,
        if shared_ctx.is_empty() {
            String::new()
        } else {
            format!("### Shared Context\n{}", shared_ctx)
        },
    ))
}
