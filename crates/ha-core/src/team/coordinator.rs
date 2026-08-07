use anyhow::{Context, Result};
use std::sync::Arc;

use super::events::emit_team_event;
use super::types::*;
use super::{
    pick_member_color, MAX_ACTIVE_TEAMS, RESUME_BLOCK_MISSING_RUN_RECORD,
    RESUME_BLOCK_OLD_ATTEMPT_ACTIVE, RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN,
};
use crate::globals::get_subagent_cancels;
use crate::session::SessionDB;
use crate::subagent::{self, SpawnParams, SubagentCancelRegistry};

const RESUME_BLOCKED_PREFIX: &str = "team_resume_blocked:";

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
    let active_count = {
        let db = db.clone();
        let lead_agent_id = lead_agent_id.to_string();
        db.run(move |db| db.count_active_teams_for_agent(&lead_agent_id))
            .await?
    };
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

    {
        let db = db.clone();
        let team = team.clone();
        db.run(move |db| db.insert_team(&team)).await?;
    }
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
    // Reserve the target before persisting the TeamMember row. Subagent spawn
    // acquires its own admission later, but doing that only after this insert
    // leaves a deletion race that can strand an active Team on a removed id.
    let _agent_admission = crate::agent_lifecycle::begin_agent_run(agent_id)?;
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

    {
        let db = db.clone();
        let member = member.clone();
        db.run(move |db| db.insert_team_member(&member)).await?;
    }
    emit_team_event("member_joined", &member);

    launch_member_attempt(
        db,
        cancel_registry,
        team,
        member,
        task,
        MemberStatus::Idle,
        true,
    )
    .await
}

/// Launch a fresh immutable attempt for an existing roster member and attach
/// it with a conditional DB write. Resume reuses this path with
/// `expected_status=Paused`; it never inserts a second TeamMember row.
async fn launch_member_attempt(
    db: &Arc<SessionDB>,
    cancel_registry: &Arc<SubagentCancelRegistry>,
    team: &Team,
    member: TeamMember,
    task: &str,
    expected_status: MemberStatus,
    admission_already_held: bool,
) -> Result<TeamMember> {
    if expected_status == MemberStatus::Paused {
        let blocker = {
            let db = db.clone();
            let team_id = team.team_id.clone();
            let member_id = member.member_id.clone();
            let old_run_id = member.run_id.clone();
            db.run(move |db| {
                db.team_member_resume_blocker(&team_id, &member_id, old_run_id.as_deref())
            })
            .await?
        };
        if let Some((reason, status)) = blocker {
            anyhow::bail!("{RESUME_BLOCKED_PREFIX}{reason}:{status}");
        }
    }

    let _agent_admission = if admission_already_held {
        None
    } else {
        Some(crate::agent_lifecycle::begin_agent_run(&member.agent_id)?)
    };

    // Build team context for the member's system prompt
    let extra_context = {
        let db = db.clone();
        let team = team.clone();
        let member = member.clone();
        let task = task.to_string();
        db.run(move |db| build_member_context(db, &team, &member, &task))
            .await?
    };

    let params = SpawnParams {
        task: task.to_string(),
        agent_id: member.agent_id.clone(),
        parent_session_id: team.lead_session_id.clone(),
        parent_agent_id: team.lead_agent_id.clone(),
        depth: 1,
        timeout_secs: None, // use agent default
        model_override: member.model_override.clone(),
        label: Some(format!("team:{}/{}", team.name, member.name)),
        isolate_worktree: false,
        attachments: Vec::new(),
        plan_agent_mode: None,
        plan_mode_allow_paths: Vec::new(),
        lock_plan_agent_mode: false,
        skip_parent_injection: true, // coordinator handles result routing
        extra_system_context: Some(extra_context),
        skill_allowed_tools: Vec::new(),
        reasoning_effort: None,
        skill_name: None,
        origin_source: None,
        origin_channel_kb_context: None,
        // Internal team-member subagent (skip_parent_injection) — never grouped (R5).
        group_id: None,
        owner_kind: crate::subagent::SubagentOwnerKind::Team,
        owner_id: team.team_id.clone(),
        delivery_kind: crate::subagent::SubagentDeliveryKind::None,
    };

    // Phase 1: materialize the child session + immutable run and register its
    // cancel flag, but do not enqueue, fire hooks, or spawn an executor.
    let prepared = subagent::prepare_subagent(params, db.clone(), cancel_registry.clone()).await?;
    let run_id = prepared.run_id().to_string();
    let session_id = prepared.child_session_id().to_string();
    let previous_run_id = member.run_id.clone();
    let previous_session_id = member.session_id.clone();

    // Phase 2: attach the exact prepared identity while the Team/member
    // capability is still live. A pause/dissolve/remove that commits first
    // makes this conditional write fail.
    let activated = {
        let db = db.clone();
        let team_id = team.team_id.clone();
        let member_id = member.member_id.clone();
        let expected_status = expected_status.clone();
        let update_run_id = run_id.clone();
        let update_session_id = session_id.clone();
        let expected_previous_run_id = previous_run_id.clone();
        let expected_previous_session_id = previous_session_id.clone();
        db.run(move |db| {
            db.activate_team_member_attempt(
                &team_id,
                &member_id,
                &expected_status,
                expected_previous_run_id.as_deref(),
                expected_previous_session_id.as_deref(),
                &update_run_id,
                &update_session_id,
            )
        })
        .await
    };
    match activated {
        Ok(true) => {}
        Ok(false) => {
            subagent::discard_prepared_subagent(
                prepared,
                "Team/member state changed before prepared attempt attachment",
            )
            .await
            .context("Failed to settle unattached Team attempt")?;
            anyhow::bail!("Team/member state changed before the fresh attempt could attach");
        }
        Err(error) => {
            subagent::discard_prepared_subagent(
                prepared,
                "Team attempt attachment failed before execution",
            )
            .await
            .context("Failed to settle Team attempt after attachment error")?;
            return Err(error);
        }
    }

    // Phase 3: atomically claim execution with the exact Team attachment in the
    // SQL predicate. This closes attach -> launch races for both immediate and
    // queued attempts; no SubagentStart hook can fire before this succeeds.
    if let Err(error) = subagent::launch_prepared_subagent(
        prepared,
        Some(subagent::TeamMemberLaunchFence {
            team_id: team.team_id.clone(),
            member_id: member.member_id.clone(),
        }),
    )
    .await
    {
        let restore_db = db.clone();
        let restore_team_id = team.team_id.clone();
        let restore_member_id = member.member_id.clone();
        let restore_run_id = run_id.clone();
        let restore_session_id = session_id.clone();
        let restore_status = expected_status.clone();
        let previous_run_id = previous_run_id.clone();
        let previous_session_id = previous_session_id.clone();
        let _ = restore_db
            .run(move |db| {
                db.restore_team_member_after_unlaunched_attempt(
                    &restore_team_id,
                    &restore_member_id,
                    &restore_run_id,
                    &restore_session_id,
                    &restore_status,
                    previous_run_id.as_deref(),
                    previous_session_id.as_deref(),
                )
            })
            .await;
        return Err(error);
    }

    let mut updated = member;
    updated.run_id = Some(run_id);
    updated.session_id = Some(session_id);
    updated.status = MemberStatus::Working;

    emit_team_event(
        "member_status",
        &serde_json::json!({
            "teamId": team.team_id,
            "memberId": updated.member_id,
            "status": "working",
        }),
    );

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
    let (team, members) = {
        let db = db.clone();
        let team_id = team_id.to_string();
        db.run(move |db| {
            let team = db
                .get_team(&team_id)?
                .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
            if !team.status.is_active() {
                anyhow::bail!("Team is not active");
            }
            let members = db.list_team_members(&team_id)?;
            Ok((team, members))
        })
        .await?
    };
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

    {
        let db = db.clone();
        let team_id = team_id.to_string();
        let message = format!("{} joined the team", name);
        let _ = db
            .run(move |db| super::messaging::post_system_message(db, &team_id, &message))
            .await;
    }

    Ok(member)
}

/// Remove a member from an Active team. Durable revocation commits before the
/// exact run is sent through the canonical cancellation path.
pub async fn remove_member(
    db: &Arc<SessionDB>,
    team_id: &str,
    member_id: &str,
) -> Result<serde_json::Value> {
    let (member, run_id) = {
        let db = db.clone();
        let team_id = team_id.to_string();
        let member_id = member_id.to_string();
        db.run(move |db| db.remove_active_team_member_and_snapshot_run(&team_id, &member_id))
            .await?
    };
    let cancellations = cancel_team_runs(db, run_id.into_iter().collect()).await;

    emit_team_event(
        "member_status",
        &serde_json::json!({
            "teamId": team_id,
            "memberId": member_id,
            "status": "killed",
        }),
    );

    {
        let db = db.clone();
        let team_id = team_id.to_string();
        let message = format!("{} was removed from the team", member.name);
        let _ = db
            .run(move |db| super::messaging::post_system_message(db, &team_id, &message))
            .await;
    }

    Ok(serde_json::json!({
        "status": "removed",
        "teamId": team_id,
        "memberId": member_id,
        "cancellation": cancellation_summary(&cancellations),
    }))
}

/// Dissolve a live team. The DB transaction revokes the team and member
/// capabilities before exact run cancellation begins.
pub async fn dissolve_team(db: &Arc<SessionDB>, team_id: &str) -> Result<serde_json::Value> {
    let (team, run_ids) = {
        let db = db.clone();
        let team_id = team_id.to_string();
        db.run(move |db| db.dissolve_team_and_snapshot_runs(&team_id))
            .await?
    };
    let cancellations = cancel_team_runs(db, run_ids).await;

    emit_team_event(
        "dissolved",
        &serde_json::json!({
            "teamId": team_id,
            "name": team.name,
        }),
    );

    app_info!("team", "coordinator", "Team '{}' dissolved", team.name);
    Ok(serde_json::json!({
        "status": "dissolved",
        "teamId": team_id,
        "cancellation": cancellation_summary(&cancellations),
    }))
}

/// Pause an Active team. Durable fencing happens first; canonical cancellation
/// may still be pending when this method returns and is reported as such.
pub async fn pause_team(db: &Arc<SessionDB>, team_id: &str) -> Result<serde_json::Value> {
    let (paused_members, run_ids) = {
        let db = db.clone();
        let team_id = team_id.to_string();
        db.run(move |db| db.pause_active_team_and_snapshot_runs(&team_id))
            .await?
    };
    let cancellations = cancel_team_runs(db, run_ids).await;

    emit_team_event("paused", &serde_json::json!({ "teamId": team_id }));

    Ok(serde_json::json!({
        "status": "paused",
        "teamId": team_id,
        "pausedMemberCount": paused_members,
        "cancellation": cancellation_summary(&cancellations),
        "message": "The team is durably paused. Cancellation requests may still be settling; inspect cancellation.pendingCount before claiming every member attempt is terminal."
    }))
}

/// Resume a paused team — re-spawn paused members with their original tasks.
pub async fn resume_team(db: &Arc<SessionDB>, team_id: &str) -> Result<serde_json::Value> {
    let (team, members, pending_members, completed_members) = {
        let db = db.clone();
        let team_id = team_id.to_string();
        db.run(move |db| db.begin_resume_team(&team_id)).await?
    };
    let completed_summaries: Vec<_> = completed_members
        .iter()
        .map(|member| {
            serde_json::json!({
                "memberId": member.member_id,
                "name": member.name,
                "runId": member.run_id,
                "status": "completed",
            })
        })
        .collect();
    for member in &completed_members {
        emit_team_event(
            "member_status",
            &serde_json::json!({
                "teamId": team_id,
                "memberId": member.member_id,
                "status": "completed",
            }),
        );
    }
    if members.is_empty() && pending_members.is_empty() && !completed_summaries.is_empty() {
        return Ok(already_complete_resume_result(team_id, completed_summaries));
    }
    if !pending_members.is_empty() {
        let failures = pending_members
            .into_iter()
            .map(|(member, reason, old_attempt_status)| {
                old_attempt_failure(&member, &reason, &old_attempt_status)
            })
            .collect();
        let mut result = resume_result(team_id, Vec::new(), failures, &TeamStatus::Paused);
        result["completedDuringPauseCount"] =
            serde_json::Value::from(completed_summaries.len() as u64);
        result["completedMembers"] = serde_json::Value::Array(completed_summaries);
        result["message"] = serde_json::Value::String(
            "Resume is pending because the durable state cannot prove every old member attempt is terminal; the team remains paused and no fresh attempt was started. Inspect failures, then retry after active cancellations settle or missing run records are repaired."
                .to_string(),
        );
        result["retryable"] = serde_json::Value::Bool(true);
        return Ok(result);
    }
    let cancel_registry = get_subagent_cancels()
        .cloned()
        .unwrap_or_else(|| Arc::new(SubagentCancelRegistry::new()));

    let mut resumed = Vec::new();
    let mut failures = Vec::new();
    for member in members {
        // Find the member's current task
        let task_text = if let Some(task_id) = member.current_task_id {
            let db = db.clone();
            db.run(move |db| db.get_team_task(task_id))
                .await?
                .map(|t| t.content)
                .unwrap_or_else(|| format!("Continue previous work as {}", member.name))
        } else {
            format!("Continue previous work as {}", member.name)
        };

        match launch_member_attempt(
            db,
            &cancel_registry,
            &team,
            member.clone(),
            &task_text,
            MemberStatus::Paused,
            false,
        )
        .await
        {
            Ok(updated) => resumed.push(serde_json::json!({
                "memberId": updated.member_id,
                "name": updated.name,
                "runId": updated.run_id,
                "sessionId": updated.session_id,
            })),
            Err(error) => {
                let raw_message = error.to_string();
                let message = crate::logging::redact_sensitive(&raw_message);
                app_warn!(
                    "team",
                    "coordinator",
                    "Failed to resume member {}: {}",
                    member.name,
                    message
                );
                if let Some((reason, old_attempt_status)) = raw_message
                    .strip_prefix(RESUME_BLOCKED_PREFIX)
                    .and_then(|encoded| encoded.split_once(':'))
                {
                    failures.push(old_attempt_failure(&member, reason, old_attempt_status));
                } else {
                    failures.push(serde_json::json!({
                        "memberId": member.member_id,
                        "name": member.name,
                        "reason": message,
                    }));
                }
            }
        }
    }

    if resumed.is_empty() {
        {
            let db = db.clone();
            let team_id = team_id.to_string();
            db.run(move |db| db.restore_paused_if_no_active_members(&team_id))
                .await?;
        }
    }

    // Re-read the durable lifecycle state after all launch attempts. A pause or
    // dissolve racing this resume supersedes its presentation: even if one
    // attempt attached transiently, the result must not claim an Active team.
    let final_team_status = {
        let db = db.clone();
        let team_id = team_id.to_string();
        db.run(move |db| {
            db.get_team(&team_id)?
                .map(|team| team.status)
                .ok_or_else(|| anyhow::anyhow!("Team not found"))
        })
        .await?
    };
    let mut result = resume_result(team_id, resumed, failures, &final_team_status);
    result["completedDuringPauseCount"] = serde_json::Value::from(completed_summaries.len() as u64);
    result["completedMembers"] = serde_json::Value::Array(completed_summaries);
    if matches!(result["disposition"].as_str(), Some("resumed" | "partial")) {
        emit_team_event(
            "resumed",
            &serde_json::json!({
                "teamId": team_id,
                "disposition": result["disposition"],
                "resumedMemberCount": result["resumedMemberCount"],
                "failedMemberCount": result["failedMemberCount"],
            }),
        );
    }
    Ok(result)
}

fn resume_result(
    team_id: &str,
    resumed: Vec<serde_json::Value>,
    failures: Vec<serde_json::Value>,
    team_status: &TeamStatus,
) -> serde_json::Value {
    let resumed_count = resumed.len();
    let failed_count = failures.len();
    let (status, disposition, message) = if team_status != &TeamStatus::Active {
        (
            if team_status == &TeamStatus::Paused {
                "paused"
            } else {
                "refused"
            },
            "refused",
            "The resume operation did not leave the team Active; no resumed outcome is claimed.",
        )
    } else if resumed_count == 0 {
        (
            "refused",
            "refused",
            "No paused member could start a fresh attempt; the team was not reported as resumed.",
        )
    } else if failed_count == 0 {
        (
            "resumed",
            "resumed",
            "Every paused roster member started a fresh immutable attempt.",
        )
    } else {
        (
            "partially_resumed",
            "partial",
            "Only some paused roster members started fresh attempts; failed members remain paused.",
        )
    };

    serde_json::json!({
        "status": status,
        "teamStatus": team_status.as_str(),
        "disposition": disposition,
        "teamId": team_id,
        "resumedMemberCount": resumed_count,
        "failedMemberCount": failed_count,
        "resumedMembers": resumed,
        "failures": failures,
        "message": message,
    })
}

fn already_complete_resume_result(
    team_id: &str,
    completed_members: Vec<serde_json::Value>,
) -> serde_json::Value {
    // `completedDuringPauseCount` is a compatibility wire name. For a stable
    // already-complete no-op it counts the full Completed roster skipped by
    // resume (the same set returned in `completedMembers`), including members
    // that had already completed before pause. Returning only the newly
    // reconciled delta would make an immediate retry produce a different body.
    serde_json::json!({
        "status": "already_complete",
        "teamStatus": TeamStatus::Paused.as_str(),
        "disposition": "no_op",
        "teamId": team_id,
        "resumedMemberCount": 0,
        "failedMemberCount": 0,
        "resumedMembers": [],
        "failures": [],
        "completedDuringPauseCount": completed_members.len(),
        "completedMembers": completed_members,
        "message": "Every member in this non-empty team roster is already completed. Their success is preserved and no fresh attempt was launched, preventing duplicate side effects. The team remains paused."
    })
}

fn old_attempt_failure(member: &TeamMember, reason: &str, status: &str) -> serde_json::Value {
    let (reason, status) = match reason {
        RESUME_BLOCK_OLD_ATTEMPT_ACTIVE => (RESUME_BLOCK_OLD_ATTEMPT_ACTIVE, status),
        RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN => (
            RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN,
            if status.is_empty() {
                RESUME_BLOCK_MISSING_RUN_RECORD
            } else {
                status
            },
        ),
        _ => (
            RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN,
            RESUME_BLOCK_MISSING_RUN_RECORD,
        ),
    };
    serde_json::json!({
        "memberId": member.member_id,
        "name": member.name,
        "runId": member.run_id,
        "reason": reason,
        "oldAttemptStatus": status,
    })
}

async fn cancel_team_runs(db: &Arc<SessionDB>, run_ids: Vec<String>) -> Vec<serde_json::Value> {
    let mut outcomes = Vec::with_capacity(run_ids.len());
    for run_id in run_ids {
        let cancel_id = run_id.clone();
        let requested =
            crate::blocking::run_blocking(move || subagent::request_cancel_run(&cancel_id)).await;
        let observed = {
            let db = db.clone();
            let lookup_id = run_id.clone();
            db.run(move |db| db.get_subagent_run(&lookup_id)).await
        };
        let (terminal, status) = match observed {
            Ok(Some(run)) => (
                run.status.is_terminal(),
                Some(run.status.as_str().to_string()),
            ),
            Ok(None) => (false, None),
            Err(error) => {
                app_warn!(
                    "team",
                    "coordinator",
                    "Failed to observe cancelled team run {}: {}",
                    run_id,
                    crate::logging::redact_sensitive(&error.to_string())
                );
                (false, None)
            }
        };
        let disposition = if requested {
            "requested"
        } else if terminal {
            "already_terminal"
        } else {
            "refused"
        };
        outcomes.push(serde_json::json!({
            "runId": run_id,
            "disposition": disposition,
            "requested": requested,
            "terminal": terminal,
            "status": status,
            "finalStatus": terminal.then_some(status).flatten(),
        }));
    }
    outcomes
}

fn cancellation_summary(outcomes: &[serde_json::Value]) -> serde_json::Value {
    let requested_count = outcomes
        .iter()
        .filter(|outcome| outcome["requested"] == true)
        .count();
    let terminal_count = outcomes
        .iter()
        .filter(|outcome| outcome["terminal"] == true)
        .count();
    let pending_count = outcomes
        .iter()
        .filter(|outcome| outcome["requested"] == true && outcome["terminal"] == false)
        .count();
    let refused_count = outcomes
        .iter()
        .filter(|outcome| outcome["disposition"] == "refused")
        .count();
    serde_json::json!({
        "targetCount": outcomes.len(),
        "requestedCount": requested_count,
        "terminalCount": terminal_count,
        "pendingCount": pending_count,
        "refusedCount": refused_count,
        "runs": outcomes,
    })
}

/// Get a summary of the team status.
pub fn get_team_status(db: &SessionDB, team_id: &str) -> Result<serde_json::Value> {
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
    db: &SessionDB,
    team: &Team,
    member: &TeamMember,
    task: &str,
) -> Result<String> {
    let members = db.list_team_members(&team.team_id)?;
    // Batch-fetch all tasks to avoid N+1 queries
    let tasks = db.list_team_tasks(&team.team_id)?;
    let task_map: std::collections::HashMap<i64, &str> =
        tasks.iter().map(|t| (t.id, t.content.as_str())).collect();

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

    let shared_ctx = team.config.shared_context.as_deref().unwrap_or("");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent::{SubagentDeliveryKind, SubagentOwnerKind, SubagentRun, SubagentStatus};

    fn resumed_member() -> serde_json::Value {
        serde_json::json!({
            "memberId": "member-1",
            "name": "worker",
            "runId": "run-new",
            "sessionId": "session-new",
        })
    }

    fn failed_member() -> serde_json::Value {
        serde_json::json!({
            "memberId": "member-2",
            "name": "reviewer",
            "reason": "spawn failed",
        })
    }

    #[test]
    fn resume_wire_distinguishes_full_partial_and_refused_outcomes() {
        let full = resume_result(
            "team-1",
            vec![resumed_member()],
            Vec::new(),
            &TeamStatus::Active,
        );
        assert_eq!(full["status"], "resumed");
        assert_eq!(full["teamStatus"], "active");
        assert_eq!(full["disposition"], "resumed");

        let partial = resume_result(
            "team-1",
            vec![resumed_member()],
            vec![failed_member()],
            &TeamStatus::Active,
        );
        assert_eq!(partial["status"], "partially_resumed");
        assert_eq!(partial["teamStatus"], "active");
        assert_eq!(partial["disposition"], "partial");

        let refused = resume_result(
            "team-1",
            Vec::new(),
            vec![failed_member()],
            &TeamStatus::Paused,
        );
        assert_eq!(refused["status"], "paused");
        assert_eq!(refused["teamStatus"], "paused");
        assert_eq!(refused["disposition"], "refused");

        let superseded = resume_result(
            "team-1",
            vec![resumed_member()],
            Vec::new(),
            &TeamStatus::Dissolved,
        );
        assert_eq!(superseded["status"], "refused");
        assert_eq!(superseded["teamStatus"], "dissolved");
        assert_eq!(superseded["disposition"], "refused");
    }

    #[test]
    fn resume_wire_reports_late_success_as_already_complete_no_op() {
        let value = already_complete_resume_result(
            "team-1",
            vec![serde_json::json!({
                "memberId": "member-1",
                "runId": "run-old",
                "status": "completed",
            })],
        );
        assert_eq!(value["status"], "already_complete");
        assert_eq!(value["disposition"], "no_op");
        assert_eq!(value["teamStatus"], "paused");
        assert_eq!(value["resumedMemberCount"], 0);
        assert_eq!(value["completedDuringPauseCount"], 1);
        assert_eq!(value["completedMembers"][0]["runId"], "run-old");
    }

    #[tokio::test]
    async fn resume_team_is_an_idempotent_no_op_for_an_already_completed_roster() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&temp.path().join("sessions.db"))
                .expect("open test db"),
        );
        db.insert_team(&Team {
            team_id: "team-complete".to_string(),
            name: "Complete Team".to_string(),
            description: None,
            lead_session_id: "lead-complete".to_string(),
            lead_agent_id: "lead-agent".to_string(),
            status: TeamStatus::Active,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            template_id: None,
            config: TeamConfig::default(),
        })
        .expect("insert team");
        let seed_member =
            |member_id: &str, run_id: &str, session_id: &str, run_status: SubagentStatus| {
                db.insert_subagent_run(&SubagentRun {
                    run_id: run_id.to_string(),
                    thread_id: session_id.to_string(),
                    parent_session_id: "lead-complete".to_string(),
                    parent_agent_id: "lead-agent".to_string(),
                    child_agent_id: "member-agent".to_string(),
                    child_session_id: session_id.to_string(),
                    task: "work".to_string(),
                    status: run_status,
                    started_at: "2026-01-01T00:00:00Z".to_string(),
                    delivery_kind: SubagentDeliveryKind::None,
                    owner_kind: SubagentOwnerKind::Team,
                    owner_id: "team-complete".to_string(),
                    ..SubagentRun::default()
                })
                .expect("insert run");
                db.insert_team_member(&TeamMember {
                    member_id: member_id.to_string(),
                    team_id: "team-complete".to_string(),
                    name: member_id.to_string(),
                    agent_id: "member-agent".to_string(),
                    role: MemberRole::Worker,
                    status: MemberStatus::Working,
                    run_id: Some(run_id.to_string()),
                    session_id: Some(session_id.to_string()),
                    color: "#3B82F6".to_string(),
                    current_task_id: None,
                    model_override: None,
                    role_description: None,
                    joined_at: "2026-01-01T00:00:00Z".to_string(),
                    last_active_at: None,
                    input_tokens: Some(0),
                    output_tokens: Some(0),
                })
                .expect("insert working member");
            };
        seed_member(
            "member-completed-before-pause",
            "run-completed-before-pause",
            "session-completed-before-pause",
            SubagentStatus::Completed,
        );
        seed_member(
            "member-late-complete",
            "run-late-complete",
            "session-late-complete",
            SubagentStatus::Running,
        );
        db.pause_active_team_and_snapshot_runs("team-complete")
            .expect("pause team before late completion");
        db.update_subagent_status(
            "run-late-complete",
            SubagentStatus::Completed,
            Some("done"),
            None,
            None,
            Some(1),
        )
        .expect("complete run after pause");

        let run_count = db
            .list_subagent_runs("lead-complete")
            .expect("list initial runs")
            .len();
        let first = resume_team(&db, "team-complete")
            .await
            .expect("first no-op resume");
        let repeated = resume_team(&db, "team-complete")
            .await
            .expect("repeated no-op resume");

        for result in [&first, &repeated] {
            assert_eq!(result["status"], "already_complete");
            assert_eq!(result["disposition"], "no_op");
            assert_eq!(result["teamStatus"], "paused");
            assert_eq!(result["resumedMemberCount"], 0);
            assert_eq!(result["resumedMembers"], serde_json::json!([]));
            assert_eq!(
                result["completedDuringPauseCount"], 2,
                "compatibility field counts the full Completed roster skipped by resume"
            );
            assert_eq!(
                result["completedMembers"]
                    .as_array()
                    .expect("completed roster")
                    .iter()
                    .map(|member| member["runId"].as_str().expect("run id"))
                    .collect::<Vec<_>>(),
                vec!["run-completed-before-pause", "run-late-complete"]
            );
        }
        assert_eq!(first, repeated, "refresh/retry must be wire-stable");
        assert_eq!(
            db.list_subagent_runs("lead-complete")
                .expect("list final runs")
                .len(),
            run_count,
            "no-op resume must not materialize a fresh attempt"
        );
        assert_eq!(
            db.get_team("team-complete").unwrap().unwrap().status,
            TeamStatus::Paused,
            "no-op resume must not emit a resumed lifecycle outcome"
        );
    }

    #[test]
    fn cancellation_summary_never_equates_request_with_terminal_completion() {
        let summary = cancellation_summary(&[
            serde_json::json!({
                "runId": "run-pending",
                "disposition": "requested",
                "requested": true,
                "terminal": false,
            }),
            serde_json::json!({
                "runId": "run-done",
                "disposition": "already_terminal",
                "requested": false,
                "terminal": true,
            }),
        ]);
        assert_eq!(summary["requestedCount"], 1);
        assert_eq!(summary["terminalCount"], 1);
        assert_eq!(summary["pendingCount"], 1);
        assert_eq!(summary["refusedCount"], 0);
    }

    #[test]
    fn pending_old_attempt_failure_is_machine_readable() {
        let member = TeamMember {
            member_id: "member-pending".into(),
            team_id: "team-1".into(),
            name: "worker".into(),
            agent_id: "agent-1".into(),
            role: MemberRole::Worker,
            status: MemberStatus::Paused,
            run_id: Some("run-old".into()),
            session_id: Some("session-old".into()),
            color: String::new(),
            current_task_id: None,
            model_override: None,
            role_description: None,
            joined_at: String::new(),
            last_active_at: None,
            input_tokens: None,
            output_tokens: None,
        };
        let failure = old_attempt_failure(&member, RESUME_BLOCK_OLD_ATTEMPT_ACTIVE, "running");
        assert_eq!(failure["reason"], RESUME_BLOCK_OLD_ATTEMPT_ACTIVE);
        assert_eq!(failure["oldAttemptStatus"], "running");
        assert_eq!(failure["runId"], "run-old");

        let missing = old_attempt_failure(
            &member,
            RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN,
            RESUME_BLOCK_MISSING_RUN_RECORD,
        );
        assert_eq!(missing["reason"], RESUME_BLOCK_OLD_ATTEMPT_UNKNOWN);
        assert_eq!(missing["oldAttemptStatus"], RESUME_BLOCK_MISSING_RUN_RECORD);
    }
}
