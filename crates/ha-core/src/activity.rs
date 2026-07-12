//! Read-only projection of Goal / Workflow / Task / Loop / Job activity.
//!
//! This module intentionally owns no lifecycle state. It rebuilds one bounded,
//! user-facing status from the durable control-plane truth sources so the
//! composer, Workspace, and model do not invent competing interpretations.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::async_jobs::{JobKind, JobManager, JobStatus};
use crate::goal::{GoalSnapshot, GoalState};
use crate::loop_control::{LoopSchedule, LoopState};
use crate::session::{SessionDB, Task};
use crate::workflow::{WorkflowRun, WorkflowRunState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyActivityState {
    Idle,
    Active,
    WaitingUser,
    WaitingExternal,
    Evaluating,
    Paused,
    Blocked,
    Terminal,
}

impl AutonomyActivityState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Active => "active",
            Self::WaitingUser => "waiting_user",
            Self::WaitingExternal => "waiting_external",
            Self::Evaluating => "evaluating",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySourceRef {
    pub kind: String,
    pub id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDirective {
    pub kind: String,
    pub reason_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCounts {
    pub active_workflows: usize,
    pub active_tasks: usize,
    pub active_loops: usize,
    pub active_jobs: usize,
    pub awaiting_approval: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomyActivity {
    pub session_id: String,
    pub state: AutonomyActivityState,
    pub headline_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<ActivityDirective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<ActivityDirective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_wakeup_at: Option<String>,
    pub needs_user: bool,
    pub counts: ActivityCounts,
    #[serde(default)]
    pub source_refs: Vec<ActivitySourceRef>,
    pub projected_at: String,
}

impl SessionDB {
    pub fn autonomy_activity_for_session(&self, session_id: &str) -> Result<AutonomyActivity> {
        let goal = match self.active_goal_for_session(session_id)? {
            Some(goal) => Some(goal),
            None => self
                .latest_goal_for_session(session_id)?
                .filter(|snapshot| {
                    matches!(
                        snapshot.goal.state,
                        GoalState::Failed | GoalState::Cancelled
                    ) || (snapshot.goal.state == GoalState::Completed
                        && snapshot.goal.closure_decision.is_some())
                }),
        };
        let workflows = self.list_workflow_runs_for_session(session_id, 50)?;
        let loops = match crate::get_cron_db() {
            Some(cron_db) => {
                self.list_loop_schedules_for_session_with_cron(cron_db, session_id, 50)?
            }
            None => self.list_loop_schedules_for_session(session_id, 50)?,
        };
        let jobs = JobManager::list_active_by_session_limited(session_id, 50)?;
        let tasks = goal
            .as_ref()
            .map(|snapshot| snapshot.tasks.clone())
            .unwrap_or_else(|| self.list_tasks(session_id).unwrap_or_default());

        let active_workflows = workflows
            .iter()
            .filter(|run| workflow_is_live(run.state))
            .count();
        let active_tasks = tasks
            .iter()
            .filter(|task| task.status == "in_progress")
            .count();
        let active_loops = loops
            .iter()
            .filter(|schedule| {
                matches!(
                    schedule.state,
                    LoopState::Active | LoopState::Paused | LoopState::Blocked
                )
            })
            .count();
        let active_jobs = jobs
            .iter()
            .filter(|job| job.kind != JobKind::Monitor)
            .count();
        let awaiting_approval = jobs
            .iter()
            .filter(|job| job.status == JobStatus::AwaitingApproval)
            .count()
            + workflows
                .iter()
                .filter(|run| {
                    matches!(
                        run.state,
                        WorkflowRunState::AwaitingApproval | WorkflowRunState::AwaitingUser
                    )
                })
                .count();
        let counts = ActivityCounts {
            active_workflows,
            active_tasks,
            active_loops,
            active_jobs,
            awaiting_approval,
        };
        let source_refs = activity_source_refs(&goal, &workflows, &tasks, &loops, &jobs);
        let next_wakeup_at = loops
            .iter()
            .filter(|schedule| schedule.state == LoopState::Active)
            .filter_map(|schedule| schedule.next_run_at.as_ref())
            .min()
            .cloned();

        let projection = if let Some(job) = jobs
            .iter()
            .find(|job| job.status == JobStatus::AwaitingApproval)
        {
            activity(
                session_id,
                AutonomyActivityState::WaitingUser,
                "waiting_job_approval",
                Some(job.tool_name.clone()),
                Some(directive(
                    "approval",
                    "approval_required",
                    &job.job_id,
                    &job.tool_name,
                )),
                Some(directive(
                    "user",
                    "review_approval",
                    &job.job_id,
                    &job.tool_name,
                )),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if let Some(run) = workflows.iter().find(|run| {
            matches!(
                run.state,
                WorkflowRunState::AwaitingApproval | WorkflowRunState::AwaitingUser
            )
        }) {
            let reason = if run.state == WorkflowRunState::AwaitingApproval {
                "workflow_approval_required"
            } else {
                "workflow_user_input_required"
            };
            activity(
                session_id,
                AutonomyActivityState::WaitingUser,
                "waiting_workflow_user",
                Some(run.kind.clone()),
                Some(directive("workflow", reason, &run.id, &run.kind)),
                Some(directive("user", reason, &run.id, &run.kind)),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if let Some(snapshot) = goal.as_ref().filter(|snapshot| {
            snapshot.goal.state == GoalState::Completed && snapshot.goal.closure_decision.is_none()
        }) {
            activity(
                session_id,
                AutonomyActivityState::WaitingUser,
                "waiting_goal_acceptance",
                Some(snapshot.goal.objective.clone()),
                Some(directive(
                    "goal",
                    "goal_acceptance_required",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                Some(directive(
                    "user",
                    "review_goal_closure",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if goal
            .as_ref()
            .is_some_and(|snapshot| snapshot.goal.state == GoalState::Evaluating)
        {
            let snapshot = goal.as_ref().expect("checked above");
            activity(
                session_id,
                AutonomyActivityState::Evaluating,
                "evaluating_goal",
                Some(snapshot.goal.objective.clone()),
                None,
                Some(directive(
                    "goal",
                    "finish_evaluation",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(run) = workflows.iter().find(|run| {
            matches!(
                run.state,
                WorkflowRunState::Running | WorkflowRunState::Recovering
            )
        }) {
            activity(
                session_id,
                AutonomyActivityState::Active,
                "running_workflow",
                Some(run.kind.clone()),
                None,
                Some(directive(
                    "workflow",
                    "continue_workflow",
                    &run.id,
                    &run.kind,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(task) = tasks.iter().find(|task| task.status == "in_progress") {
            activity(
                session_id,
                AutonomyActivityState::Active,
                "running_task",
                Some(task.content.clone()),
                None,
                Some(directive(
                    "task",
                    "continue_task",
                    &task.id.to_string(),
                    &task.content,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(job) = jobs.iter().find(|job| {
            job.kind != JobKind::Monitor
                && matches!(
                    job.status,
                    JobStatus::Queued | JobStatus::Running | JobStatus::Cancelling
                )
        }) {
            activity(
                session_id,
                AutonomyActivityState::WaitingExternal,
                "waiting_background_work",
                Some(job.tool_name.clone()),
                Some(directive(
                    "job",
                    "background_work_running",
                    &job.job_id,
                    &job.tool_name,
                )),
                Some(directive(
                    "job",
                    "consume_background_result",
                    &job.job_id,
                    &job.tool_name,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(schedule) = loops
            .iter()
            .find(|schedule| schedule.state == LoopState::Active)
        {
            activity(
                session_id,
                AutonomyActivityState::WaitingExternal,
                "waiting_loop_trigger",
                Some(short_label(&schedule.prompt, 160)),
                Some(directive(
                    "loop",
                    "next_loop_trigger",
                    &schedule.id,
                    &schedule.prompt,
                )),
                Some(directive(
                    "loop",
                    "run_on_trigger",
                    &schedule.id,
                    &schedule.prompt,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(snapshot) = goal
            .as_ref()
            .filter(|snapshot| snapshot.goal.state == GoalState::Paused)
        {
            activity(
                session_id,
                AutonomyActivityState::Paused,
                "goal_paused",
                Some(snapshot.goal.objective.clone()),
                Some(directive(
                    "goal",
                    "goal_paused",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                Some(directive(
                    "user",
                    "resume_goal",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if let Some(run) = workflows
            .iter()
            .find(|run| run.state == WorkflowRunState::Paused)
        {
            activity(
                session_id,
                AutonomyActivityState::Paused,
                "workflow_paused",
                Some(run.kind.clone()),
                Some(directive("workflow", "workflow_paused", &run.id, &run.kind)),
                Some(directive("user", "resume_workflow", &run.id, &run.kind)),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if let Some(run) = workflows
            .iter()
            .find(|run| run.state == WorkflowRunState::Blocked)
        {
            activity(
                session_id,
                AutonomyActivityState::Blocked,
                "workflow_blocked",
                Some(run.kind.clone()),
                Some(directive(
                    "workflow",
                    run.blocked_reason.as_deref().unwrap_or("workflow_blocked"),
                    &run.id,
                    &run.kind,
                )),
                Some(directive(
                    "user",
                    "review_workflow_blocker",
                    &run.id,
                    &run.kind,
                )),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if let Some(snapshot) = goal
            .as_ref()
            .filter(|snapshot| snapshot.goal.state == GoalState::Blocked)
        {
            activity(
                session_id,
                AutonomyActivityState::Blocked,
                "goal_blocked",
                Some(snapshot.goal.objective.clone()),
                Some(directive(
                    "goal",
                    snapshot
                        .goal
                        .blocked_reason
                        .as_deref()
                        .unwrap_or("goal_blocked"),
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                Some(directive(
                    "goal",
                    "resolve_goal_blocker",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(schedule) = loops
            .iter()
            .find(|schedule| matches!(schedule.state, LoopState::Paused | LoopState::Blocked))
        {
            let state = if schedule.state == LoopState::Paused {
                AutonomyActivityState::Paused
            } else {
                AutonomyActivityState::Blocked
            };
            activity(
                session_id,
                state,
                if schedule.state == LoopState::Paused {
                    "loop_paused"
                } else {
                    "loop_blocked"
                },
                Some(short_label(&schedule.prompt, 160)),
                Some(directive(
                    "loop",
                    schedule
                        .blocked_reason
                        .as_deref()
                        .unwrap_or("loop_not_active"),
                    &schedule.id,
                    &schedule.prompt,
                )),
                Some(directive(
                    "user",
                    "review_loop",
                    &schedule.id,
                    &schedule.prompt,
                )),
                next_wakeup_at,
                true,
                counts,
                source_refs,
            )
        } else if let Some(snapshot) = goal.as_ref().filter(|snapshot| {
            matches!(
                snapshot.goal.state,
                GoalState::Failed | GoalState::Cancelled
            ) || (snapshot.goal.state == GoalState::Completed
                && snapshot.goal.closure_decision.is_some())
        }) {
            activity(
                session_id,
                AutonomyActivityState::Terminal,
                "goal_terminal",
                Some(snapshot.goal.objective.clone()),
                None,
                None,
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else if let Some(snapshot) = goal.as_ref() {
            activity(
                session_id,
                AutonomyActivityState::Active,
                "active_goal",
                Some(snapshot.goal.objective.clone()),
                None,
                Some(directive(
                    "goal",
                    "continue_goal",
                    &snapshot.goal.id,
                    &snapshot.goal.objective,
                )),
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        } else {
            activity(
                session_id,
                AutonomyActivityState::Idle,
                "idle",
                None,
                None,
                None,
                next_wakeup_at,
                false,
                counts,
                source_refs,
            )
        };
        Ok(projection)
    }
}

#[allow(clippy::too_many_arguments)]
fn activity(
    session_id: &str,
    state: AutonomyActivityState,
    headline_code: &str,
    current_step: Option<String>,
    waiting_on: Option<ActivityDirective>,
    next_action: Option<ActivityDirective>,
    next_wakeup_at: Option<String>,
    needs_user: bool,
    counts: ActivityCounts,
    source_refs: Vec<ActivitySourceRef>,
) -> AutonomyActivity {
    AutonomyActivity {
        session_id: session_id.to_string(),
        state,
        headline_code: headline_code.to_string(),
        current_step,
        waiting_on,
        next_action,
        next_wakeup_at,
        needs_user,
        counts,
        source_refs,
        projected_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn directive(kind: &str, reason_code: &str, source_id: &str, label: &str) -> ActivityDirective {
    ActivityDirective {
        kind: kind.to_string(),
        reason_code: reason_code.to_string(),
        source_id: Some(source_id.to_string()),
        label: Some(short_label(label, 160)),
    }
}

fn workflow_is_live(state: WorkflowRunState) -> bool {
    matches!(
        state,
        WorkflowRunState::AwaitingApproval
            | WorkflowRunState::Running
            | WorkflowRunState::AwaitingUser
            | WorkflowRunState::Paused
            | WorkflowRunState::Recovering
    )
}

fn short_label(value: &str, max_chars: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn activity_source_refs(
    goal: &Option<GoalSnapshot>,
    workflows: &[WorkflowRun],
    tasks: &[Task],
    loops: &[LoopSchedule],
    jobs: &[crate::async_jobs::BackgroundJob],
) -> Vec<ActivitySourceRef> {
    let mut refs = Vec::new();
    if let Some(snapshot) = goal {
        refs.push(ActivitySourceRef {
            kind: "goal".to_string(),
            id: snapshot.goal.id.clone(),
            state: snapshot.goal.state.as_str().to_string(),
            label: Some(short_label(&snapshot.goal.objective, 160)),
        });
    }
    refs.extend(
        workflows
            .iter()
            .filter(|run| workflow_is_live(run.state) || run.state == WorkflowRunState::Blocked)
            .take(4)
            .map(|run| ActivitySourceRef {
                kind: "workflow".to_string(),
                id: run.id.clone(),
                state: run.state.as_str().to_string(),
                label: Some(short_label(&run.kind, 160)),
            }),
    );
    refs.extend(
        tasks
            .iter()
            .filter(|task| task.status == "in_progress")
            .take(3)
            .map(|task| ActivitySourceRef {
                kind: "task".to_string(),
                id: task.id.to_string(),
                state: task.status.clone(),
                label: Some(short_label(&task.content, 160)),
            }),
    );
    refs.extend(
        loops
            .iter()
            .filter(|schedule| {
                matches!(
                    schedule.state,
                    LoopState::Active | LoopState::Paused | LoopState::Blocked
                )
            })
            .take(2)
            .map(|schedule| ActivitySourceRef {
                kind: "loop".to_string(),
                id: schedule.id.clone(),
                state: schedule.state.as_str().to_string(),
                label: Some(short_label(&schedule.prompt, 160)),
            }),
    );
    refs.extend(jobs.iter().take(2).map(|job| ActivitySourceRef {
        kind: if job.kind == JobKind::Monitor {
            "monitor".to_string()
        } else {
            "job".to_string()
        },
        id: job.job_id.clone(),
        state: job.status.as_str().to_string(),
        label: Some(short_label(&job.tool_name, 160)),
    }));
    refs.truncate(12);
    refs
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::goal::CreateGoalInput;
    use crate::workflow::{CreateWorkflowRunInput, WorkflowRunState};

    #[test]
    fn projection_prioritizes_user_wait_over_active_goal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare a release decision".to_string(),
                completion_criteria: "decision is reviewed".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        let initial = db
            .autonomy_activity_for_session(&session.id)
            .expect("project active goal");
        assert_eq!(initial.state, AutonomyActivityState::Active);
        assert_eq!(initial.headline_code, "active_goal");

        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "decision.review".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({"max_script_secs": 30, "max_ops": 8}),
                parent_run_id: None,
                origin: None,
                goal_id: Some(goal.goal.id),
                goal_criterion_id: None,
                worktree_id: None,
            })
            .expect("create workflow");
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("start"))
            .expect("start workflow");
        db.transition_workflow_run(
            &run.id,
            WorkflowRunState::AwaitingUser,
            Some("choose_release_route"),
        )
        .expect("await user");

        let waiting = db
            .autonomy_activity_for_session(&session.id)
            .expect("project user wait");
        assert_eq!(waiting.state, AutonomyActivityState::WaitingUser);
        assert_eq!(waiting.headline_code, "waiting_workflow_user");
        assert!(waiting.needs_user);
        assert_eq!(waiting.counts.active_workflows, 1);
        assert!(waiting
            .source_refs
            .iter()
            .any(|source| source.kind == "workflow" && source.id == run.id));
    }

    #[test]
    fn projection_is_idle_without_control_plane_work() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
        let session = db.create_session("ha-main").expect("create session");
        let projection = db
            .autonomy_activity_for_session(&session.id)
            .expect("project idle session");
        assert_eq!(projection.state, AutonomyActivityState::Idle);
        assert_eq!(projection.headline_code, "idle");
        assert!(!projection.needs_user);
        assert!(projection.source_refs.is_empty());
    }

    #[test]
    fn projection_exposes_standalone_blocked_workflow_instead_of_idle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
        let session = db.create_session("ha-main").expect("create session");
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "research.review".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({"max_script_secs": 30, "max_ops": 8}),
                parent_run_id: None,
                origin: None,
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .expect("create workflow");
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("start"))
            .expect("start workflow");
        db.transition_workflow_run(
            &run.id,
            WorkflowRunState::Blocked,
            Some("workflow_children_wait_timeout"),
        )
        .expect("block workflow");

        let projection = db
            .autonomy_activity_for_session(&session.id)
            .expect("project blocked workflow");
        assert_eq!(projection.state, AutonomyActivityState::Blocked);
        assert_eq!(projection.headline_code, "workflow_blocked");
        assert!(projection.needs_user);
        assert!(projection.source_refs.iter().any(|source| {
            source.kind == "workflow"
                && source.id == run.id
                && source.state == WorkflowRunState::Blocked.as_str()
        }));
    }

    #[test]
    fn projection_distinguishes_pending_acceptance_from_sealed_terminal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open db");
        let session = db.create_session("ha-main").expect("create session");
        let goal = db
            .create_goal(CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Return a concise answer".to_string(),
                completion_criteria: "workflow completes with evidence".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("create goal");
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "answer.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: "export default async function main(workflow) {}".to_string(),
                budget: json!({"max_script_secs": 30, "max_ops": 8}),
                parent_run_id: None,
                origin: None,
                goal_id: Some(goal.goal.id.clone()),
                goal_criterion_id: None,
                worktree_id: None,
            })
            .expect("create evidence workflow");
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("start"))
            .expect("start workflow");
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, Some("finish"))
            .expect("complete workflow and goal audit");

        let pending = db
            .autonomy_activity_for_session(&session.id)
            .expect("project pending acceptance");
        assert_eq!(pending.state, AutonomyActivityState::WaitingUser);
        assert_eq!(pending.headline_code, "waiting_goal_acceptance");
        assert!(pending.needs_user);

        db.clear_goal(&goal.goal.id).expect("seal goal");
        let terminal = db
            .autonomy_activity_for_session(&session.id)
            .expect("project terminal goal");
        assert_eq!(terminal.state, AutonomyActivityState::Terminal);
        assert_eq!(terminal.headline_code, "goal_terminal");
        assert!(!terminal.needs_user);
    }
}
