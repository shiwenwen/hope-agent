//! Deterministic coding control-plane eval harness.
//!
//! Phase 3.7 turns the coding control plane into something we can regress-test
//! instead of merely describing. Fixtures create temporary git repositories,
//! seed real session / goal / task / workflow state, then drive the production
//! Context Retrieval, Review, and Smart Verification APIs. No LLM and no
//! project validation command execution are involved, so this layer is stable
//! enough for default CI.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent_loader::DEFAULT_AGENT_ID;
use crate::coding_improvement::{
    ApplyCodingImprovementProposalResult, CodingTrendReport,
    GenerateCodingImprovementProposalsResult, PromoteCodingImprovementProposalResult,
    RecordCodingEvalRunInput,
};
use crate::context_retrieval::{self, ContextCandidate, ContextCandidateKind};
use crate::goal::CreateGoalInput;
use crate::review::{self, RunReviewInput};
use crate::session::{SessionDB, SessionIdeContext, TaskStatus};
use crate::verification::{self, PlanVerificationInput};
use crate::workflow::{
    self, CreateWorkflowRunInput, UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowRunState,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingEvalFixture {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub repo: RepoFixture,
    #[serde(default)]
    pub setup: FixtureSetup,
    #[serde(default)]
    pub runs: FixtureRuns,
    #[serde(default)]
    pub checks: FixtureChecks,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFixture {
    #[serde(default)]
    pub files: Vec<FileFixture>,
    #[serde(default)]
    pub changes: Vec<FileFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFixture {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureSetup {
    #[serde(default)]
    pub goal: Option<GoalFixture>,
    #[serde(default)]
    pub tasks: Vec<TaskFixture>,
    #[serde(default)]
    pub workflow: Option<WorkflowFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalFixture {
    pub objective: String,
    #[serde(default)]
    pub completion_criteria: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFixture {
    pub content: String,
    #[serde(default)]
    pub active_form: Option<String>,
    #[serde(default = "default_pending_status")]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowFixture {
    #[serde(default = "default_workflow_kind")]
    pub kind: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default = "default_workflow_script")]
    pub script_source: String,
    #[serde(default)]
    pub ops: Vec<WorkflowOpFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOpFixture {
    pub op_key: String,
    pub op_type: String,
    #[serde(default = "default_effect_class")]
    pub effect_class: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureRuns {
    #[serde(default)]
    pub workflow: Option<WorkflowScriptEvalRun>,
    #[serde(default)]
    pub review: Option<ReviewEvalRun>,
    #[serde(default)]
    pub verification: Option<VerificationEvalRun>,
    #[serde(default)]
    pub context: Option<ContextEvalRun>,
    #[serde(default)]
    pub improvement: Option<ImprovementEvalRun>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScriptEvalRun {
    pub script_source: String,
    #[serde(default = "default_workflow_kind")]
    pub kind: String,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default)]
    pub budget: Value,
    #[serde(default)]
    pub allow_terminal_error: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEvalRun {
    #[serde(default)]
    pub focus_paths: Vec<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub ide_context: Option<SessionIdeContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvalRun {
    #[serde(default)]
    pub focus_paths: Vec<String>,
    #[serde(default)]
    pub max_commands: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEvalRun {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub ide_context: Option<SessionIdeContext>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImprovementEvalRun {
    #[serde(default)]
    pub window_days: Option<u32>,
    #[serde(default)]
    pub generate_proposals: bool,
    #[serde(default)]
    pub apply_first_proposal: bool,
    #[serde(default)]
    pub promote_applied_proposal: bool,
    #[serde(default)]
    pub apply_proposal_kind: Option<String>,
    #[serde(default)]
    pub seed_eval_runs: Vec<RecordCodingEvalRunInput>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureChecks {
    #[serde(default)]
    pub workflow: Option<WorkflowCheck>,
    #[serde(default)]
    pub context: Option<ContextCheck>,
    #[serde(default)]
    pub review: Option<ReviewCheck>,
    #[serde(default)]
    pub verification: Option<VerificationCheck>,
    #[serde(default)]
    pub improvement: Option<ImprovementCheck>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheck {
    #[serde(default)]
    pub expected_state: Option<String>,
    #[serde(default)]
    pub expected_blocked_reason: Option<String>,
    #[serde(default)]
    pub expected_op_types: Vec<String>,
    #[serde(default)]
    pub expected_commands: Vec<String>,
    #[serde(default)]
    pub min_finding_count: Option<usize>,
    #[serde(default)]
    pub expect_review_ok: Option<bool>,
    #[serde(default)]
    pub expected_goal_relations: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheck {
    #[serde(default)]
    pub critical: Vec<CandidateExpectation>,
    #[serde(default)]
    pub min_critical_recall: Option<f64>,
    #[serde(default)]
    pub min_precision: Option<f64>,
    #[serde(default)]
    pub max_candidates: Option<usize>,
    #[serde(default)]
    pub expect_action_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateExpectation {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title_contains: Option<String>,
    #[serde(default)]
    pub path_suffix: Option<String>,
    #[serde(default)]
    pub status_contains: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCheck {
    #[serde(default)]
    pub min_findings: Option<usize>,
    #[serde(default)]
    pub max_findings: Option<usize>,
    #[serde(default)]
    pub expect_focused: Option<bool>,
    #[serde(default)]
    pub expected_profiles: Vec<String>,
    #[serde(default)]
    pub expect_ide_context: Option<bool>,
    #[serde(default)]
    pub expected_titles: Vec<String>,
    #[serde(default)]
    pub expected_categories: Vec<String>,
    #[serde(default)]
    pub expected_files: Vec<String>,
    #[serde(default)]
    pub forbidden_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCheck {
    #[serde(default)]
    pub expected_commands: Vec<String>,
    #[serde(default)]
    pub forbidden_commands: Vec<String>,
    #[serde(default)]
    pub expect_focused: Option<bool>,
    #[serde(default)]
    pub expected_focus_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImprovementCheck {
    #[serde(default)]
    pub expected_scope: Option<String>,
    #[serde(default)]
    pub min_failures: Option<usize>,
    #[serde(default)]
    pub expected_failure_categories: Vec<String>,
    #[serde(default)]
    pub min_proposals: Option<usize>,
    #[serde(default)]
    pub min_inserted_proposals: Option<usize>,
    #[serde(default)]
    pub expected_proposal_kinds: Vec<String>,
    #[serde(default)]
    pub expect_draft_only: Option<bool>,
    #[serde(default)]
    pub min_eval_runs: Option<usize>,
    #[serde(default)]
    pub expect_eval_success_rate: Option<f64>,
    #[serde(default)]
    pub min_repair_loop_blocked: Option<usize>,
    #[serde(default)]
    pub expected_applied_status: Option<String>,
    #[serde(default)]
    pub expected_applied_kind: Option<String>,
    #[serde(default)]
    pub min_applied_artifacts: Option<usize>,
    #[serde(default)]
    pub expected_action_target_contains: Option<String>,
    #[serde(default)]
    pub min_retros: Option<usize>,
    #[serde(default)]
    pub min_retro_recommendations: Option<usize>,
    #[serde(default)]
    pub expected_promoted_status: Option<String>,
    #[serde(default)]
    pub min_promoted_artifacts: Option<usize>,
    #[serde(default)]
    pub expected_promotion_target_contains: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct EvalMetrics {
    pub context_precision: Option<f64>,
    pub critical_context_recall: Option<f64>,
    pub review_findings: Option<usize>,
    pub verification_commands: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FixtureReport {
    pub name: String,
    pub metrics: EvalMetrics,
    pub outcomes: Vec<CheckOutcome>,
}

impl FixtureReport {
    pub fn passed(&self) -> bool {
        self.outcomes.iter().all(|outcome| outcome.passed)
    }

    pub fn failures(&self) -> Vec<&CheckOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| !outcome.passed)
            .collect()
    }
}

struct EvalRunArtifacts {
    repo_root: PathBuf,
    workflow: Option<workflow::WorkflowRuntimeResult>,
    review: Option<review::ReviewRunSnapshot>,
    verification: Option<verification::VerificationRunSnapshot>,
    context: Option<context_retrieval::ContextRetrievalSnapshot>,
    improvement: Option<CodingTrendReport>,
    improvement_proposals: Option<GenerateCodingImprovementProposalsResult>,
    improvement_apply: Option<ApplyCodingImprovementProposalResult>,
    improvement_promotion: Option<PromoteCodingImprovementProposalResult>,
    goal_evidence_relations: Vec<String>,
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/coding_eval")
}

pub fn load_fixtures() -> Result<Vec<CodingEvalFixture>> {
    let dir = fixtures_dir();
    let mut paths = std::fs::read_dir(&dir)
        .with_context(|| format!("reading fixtures dir {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading fixture {}", path.display()))?;
        let fixture = serde_json::from_str(&raw)
            .with_context(|| format!("parsing fixture {}", path.display()))?;
        out.push(fixture);
    }
    Ok(out)
}

pub async fn evaluate(db: Arc<SessionDB>, fixture: &CodingEvalFixture) -> Result<FixtureReport> {
    let temp = tempfile::tempdir().context("create coding eval tempdir")?;
    let repo_root = prepare_repo(temp.path(), fixture)?;
    let session = db.create_session(DEFAULT_AGENT_ID)?;
    db.update_session_working_dir(&session.id, Some(repo_root.to_string_lossy().to_string()))?;

    let goal_id = if let Some(goal) = &fixture.setup.goal {
        let snapshot = db.create_goal(CreateGoalInput {
            session_id: session.id.clone(),
            objective: goal.objective.clone(),
            completion_criteria: goal.completion_criteria.clone(),
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })?;
        Some(snapshot.goal.id)
    } else {
        None
    };

    seed_tasks(&db, &session.id, &fixture.setup.tasks)?;
    if let Some(workflow) = &fixture.setup.workflow {
        seed_workflow(&db, &session.id, goal_id.as_deref(), workflow)?;
    }

    let mut artifacts = EvalRunArtifacts {
        repo_root,
        workflow: None,
        review: None,
        verification: None,
        context: None,
        improvement: None,
        improvement_proposals: None,
        improvement_apply: None,
        improvement_promotion: None,
        goal_evidence_relations: Vec::new(),
    };

    if let Some(run) = &fixture.runs.workflow {
        let workflow_run = db.create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: run.kind.clone(),
            execution_mode: run.execution_mode.clone(),
            script_source: run.script_source.clone(),
            budget: run.budget.clone(),
            parent_run_id: None,
            origin: Some("eval".to_string()),
            goal_id: goal_id.clone(),
            worktree_id: None,
        })?;
        artifacts.workflow = match workflow::run_workflow_script_async(db.clone(), &workflow_run.id)
            .await
        {
            Ok(result) => Some(result),
            Err(_err) if run.allow_terminal_error => {
                let snapshot = db
                    .workflow_run_snapshot(&workflow_run.id, 500)?
                    .ok_or_else(|| anyhow::anyhow!("workflow run {} not found", workflow_run.id))?;
                Some(workflow::WorkflowRuntimeResult {
                    snapshot,
                    output: None,
                })
            }
            Err(err) => return Err(err),
        };
    }

    if let Some(run) = &fixture.runs.review {
        artifacts.review = Some(
            review::run_review_for_session(
                db.clone(),
                session.id.clone(),
                RunReviewInput {
                    scope: Some("local".to_string()),
                    goal_id: goal_id.clone(),
                    profiles: run.profiles.clone(),
                    focus_paths: resolve_focus_paths(&artifacts.repo_root, &run.focus_paths),
                    ide_context: run.ide_context.clone(),
                    ..Default::default()
                },
            )
            .await?,
        );
    }

    if let Some(run) = &fixture.runs.verification {
        artifacts.verification = Some(
            verification::plan_verification_for_session(
                db.clone(),
                session.id.clone(),
                PlanVerificationInput {
                    scope: Some("local".to_string()),
                    goal_id: goal_id.clone(),
                    max_commands: run.max_commands,
                    focus_paths: resolve_focus_paths(&artifacts.repo_root, &run.focus_paths),
                },
            )
            .await?,
        );
    }

    if let Some(run) = &fixture.runs.context {
        artifacts.context = Some(
            context_retrieval::context_retrieval_for_session(
                db.clone(),
                session.id.clone(),
                context_retrieval::ContextRetrievalInput {
                    query: run.query.clone(),
                    limit: run.limit,
                    ide_context: run.ide_context.clone(),
                },
            )
            .await?,
        );
    }

    if let Some(run) = &fixture.runs.improvement {
        for seed in &run.seed_eval_runs {
            let mut input = seed.clone();
            if input.session_id.is_none() {
                input.session_id = Some(session.id.clone());
            }
            db.record_coding_eval_run(input)?;
        }
        if run.generate_proposals {
            artifacts.improvement_proposals =
                Some(db.generate_coding_improvement_proposals(&session.id, run.window_days)?);
        }
        if run.apply_first_proposal {
            let desired_kind = run.apply_proposal_kind.as_deref();
            let proposal_id = artifacts
                .improvement_proposals
                .as_ref()
                .and_then(|result| {
                    result
                        .proposals
                        .iter()
                        .find(|proposal| {
                            proposal.status == "draft"
                                && desired_kind.is_none_or(|kind| proposal.kind == kind)
                        })
                        .map(|proposal| proposal.id.clone())
                })
                .or_else(|| {
                    db.list_coding_improvement_proposals(&session.id)
                        .ok()
                        .and_then(|proposals| {
                            proposals
                                .into_iter()
                                .find(|proposal| {
                                    proposal.status == "draft"
                                        && desired_kind.is_none_or(|kind| proposal.kind == kind)
                                })
                                .map(|proposal| proposal.id)
                        })
                })
                .ok_or_else(|| {
                    anyhow!("applyFirstProposal requested but no draft proposal exists")
                })?;
            artifacts.improvement_apply = Some(db.apply_coding_improvement_proposal(&proposal_id)?);
        }
        if run.promote_applied_proposal {
            let proposal_id = artifacts
                .improvement_apply
                .as_ref()
                .map(|result| result.proposal.id.clone())
                .or_else(|| {
                    db.list_coding_improvement_proposals(&session.id)
                        .ok()
                        .and_then(|proposals| {
                            proposals
                                .into_iter()
                                .find(|proposal| proposal.status == "applied")
                                .map(|proposal| proposal.id)
                        })
                })
                .ok_or_else(|| {
                    anyhow!("promoteAppliedProposal requested but no applied proposal exists")
                })?;
            artifacts.improvement_promotion =
                Some(db.promote_coding_improvement_proposal(&proposal_id)?);
        }
        artifacts.improvement = Some(db.coding_trend_report(&session.id, run.window_days)?);
    }

    if let Some(goal_id) = goal_id.as_deref() {
        if let Some(snapshot) = db.goal_snapshot(goal_id, 200)? {
            artifacts.goal_evidence_relations = snapshot
                .evidence
                .iter()
                .map(|item| item.relation.clone())
                .collect();
        }
    }

    Ok(check_fixture(fixture, &artifacts))
}

fn check_fixture(fixture: &CodingEvalFixture, artifacts: &EvalRunArtifacts) -> FixtureReport {
    let mut report = FixtureReport {
        name: fixture.name.clone(),
        metrics: EvalMetrics::default(),
        outcomes: Vec::new(),
    };
    if let Some(check) = &fixture.checks.workflow {
        check_workflow(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.review {
        check_review(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.verification {
        check_verification(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.context {
        check_context(&mut report, artifacts, check);
    }
    if let Some(check) = &fixture.checks.improvement {
        check_improvement(&mut report, artifacts, check);
    }
    report
}

fn check_workflow(report: &mut FixtureReport, artifacts: &EvalRunArtifacts, check: &WorkflowCheck) {
    let Some(result) = artifacts.workflow.as_ref() else {
        push_check(
            report,
            "workflow.snapshot",
            false,
            "workflow run was not requested",
        );
        return;
    };
    let expected_state = check.expected_state.as_deref().unwrap_or("completed");
    push_check(
        report,
        "workflow.state",
        result.snapshot.run.state.as_str() == expected_state,
        format!(
            "state={}, expected={expected_state}",
            result.snapshot.run.state.as_str()
        ),
    );
    if let Some(expected) = check.expected_blocked_reason.as_deref() {
        push_check(
            report,
            "workflow.blocked_reason",
            result.snapshot.run.blocked_reason.as_deref() == Some(expected),
            format!(
                "blockedReason={:?}, expected={expected}",
                result.snapshot.run.blocked_reason
            ),
        );
    }

    if !check.expected_op_types.is_empty() {
        let actual = result
            .snapshot
            .ops
            .iter()
            .map(|op| op.op_type.clone())
            .collect::<Vec<_>>();
        push_check(
            report,
            "workflow.op_types",
            actual == check.expected_op_types,
            format!("actual={actual:?}, expected={:?}", check.expected_op_types),
        );
    }

    if let Some(expect) = check.expect_review_ok {
        let actual = result
            .output
            .as_ref()
            .and_then(|output| output.get("reviewOk"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "workflow.review_ok",
            actual == expect,
            format!("reviewOk={actual}, expected={expect}"),
        );
    }

    if let Some(min) = check.min_finding_count {
        let actual = result
            .output
            .as_ref()
            .and_then(|output| output.get("findingCount"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        push_check(
            report,
            "workflow.min_finding_count",
            actual >= min,
            format!("findingCount={actual}, min={min}"),
        );
    }

    for expected in &check.expected_commands {
        let found = result
            .output
            .as_ref()
            .and_then(|output| output.get("commands"))
            .and_then(Value::as_array)
            .is_some_and(|commands| {
                commands
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|command| command == expected)
            });
        push_check(
            report,
            format!("workflow.command.{expected}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("output={:?}", result.output)
            },
        );
    }

    for expected in &check.expected_goal_relations {
        let found = artifacts
            .goal_evidence_relations
            .iter()
            .any(|relation| relation == expected);
        push_check(
            report,
            format!("workflow.goal_relation.{expected}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("relations={:?}", artifacts.goal_evidence_relations)
            },
        );
    }
}

fn check_context(report: &mut FixtureReport, artifacts: &EvalRunArtifacts, check: &ContextCheck) {
    let Some(snapshot) = artifacts.context.as_ref() else {
        push_check(
            report,
            "context.snapshot",
            false,
            "context run was not requested",
        );
        return;
    };
    let candidates = &snapshot.candidates;
    if let Some(max) = check.max_candidates {
        push_check(
            report,
            "context.max_candidates",
            candidates.len() <= max,
            format!("{} candidate(s), max {}", candidates.len(), max),
        );
    }

    let mut matched = HashSet::<usize>::new();
    let mut matched_critical = 0usize;
    for expected in &check.critical {
        let found = candidates
            .iter()
            .enumerate()
            .find(|(_, candidate)| candidate_matches(candidate, expected));
        if let Some((idx, _)) = found {
            matched.insert(idx);
            matched_critical += 1;
            push_check(
                report,
                format!("context.critical.{}", expected.label()),
                true,
                "matched".to_string(),
            );
        } else {
            push_check(
                report,
                format!("context.critical.{}", expected.label()),
                false,
                format!("not found among {}", summarize_candidates(candidates)),
            );
        }
    }

    if !check.critical.is_empty() {
        let recall = matched_critical as f64 / check.critical.len() as f64;
        report.metrics.critical_context_recall = Some(recall);
        if let Some(min) = check.min_critical_recall {
            push_check(
                report,
                "context.critical_recall",
                recall + f64::EPSILON >= min,
                format!("{recall:.3} >= {min:.3}"),
            );
        }
    }

    if !candidates.is_empty() && !check.critical.is_empty() {
        let precision = matched.len() as f64 / candidates.len() as f64;
        report.metrics.context_precision = Some(precision);
        if let Some(min) = check.min_precision {
            push_check(
                report,
                "context.precision",
                precision + f64::EPSILON >= min,
                format!("{precision:.3} >= {min:.3}"),
            );
        }
    }

    for suffix in &check.expect_action_paths {
        let found = candidates.iter().any(|candidate| {
            focus_paths(candidate)
                .iter()
                .any(|path| path_matches_suffix(path, suffix))
        });
        push_check(
            report,
            format!("context.action_path.{suffix}"),
            found,
            if found {
                "matched".to_string()
            } else {
                "missing action focus path".to_string()
            },
        );
    }
}

fn check_review(report: &mut FixtureReport, artifacts: &EvalRunArtifacts, check: &ReviewCheck) {
    let Some(snapshot) = artifacts.review.as_ref() else {
        push_check(
            report,
            "review.snapshot",
            false,
            "review run was not requested",
        );
        return;
    };
    let findings = &snapshot.findings;
    report.metrics.review_findings = Some(findings.len());

    if let Some(min) = check.min_findings {
        push_check(
            report,
            "review.min_findings",
            findings.len() >= min,
            format!("{} finding(s), min {}", findings.len(), min),
        );
    }
    if let Some(max) = check.max_findings {
        push_check(
            report,
            "review.max_findings",
            findings.len() <= max,
            format!("{} finding(s), max {}", findings.len(), max),
        );
    }
    if let Some(expect) = check.expect_focused {
        let focused = snapshot
            .run
            .stats
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "review.focused",
            focused == expect,
            format!("focused={focused}, expected={expect}"),
        );
    }
    for profile in &check.expected_profiles {
        let found = snapshot
            .run
            .stats
            .get("profiles")
            .and_then(Value::as_array)
            .is_some_and(|profiles| {
                profiles
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|p| p == profile)
            });
        push_check(
            report,
            format!("review.profile.{profile}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("stats={}", snapshot.run.stats)
            },
        );
    }
    if let Some(expect) = check.expect_ide_context {
        let present = snapshot
            .run
            .stats
            .get("ideContext")
            .and_then(|value| value.get("present"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "review.ide_context",
            present == expect,
            format!("ideContext.present={present}, expected={expect}"),
        );
    }
    for title in &check.expected_titles {
        let found = findings
            .iter()
            .any(|finding| contains_ci(&finding.title, title));
        push_check(
            report,
            format!("review.title.{title}"),
            found,
            if found {
                "matched".to_string()
            } else {
                summarize_findings(findings)
            },
        );
    }
    for category in &check.expected_categories {
        let found = findings.iter().any(|finding| finding.category == *category);
        push_check(
            report,
            format!("review.category.{category}"),
            found,
            if found {
                "matched".to_string()
            } else {
                summarize_findings(findings)
            },
        );
    }
    for suffix in &check.expected_files {
        let found = findings
            .iter()
            .any(|finding| path_matches_suffix(&finding.file, suffix));
        push_check(
            report,
            format!("review.file.{suffix}"),
            found,
            if found {
                "matched".to_string()
            } else {
                summarize_findings(findings)
            },
        );
    }
    for suffix in &check.forbidden_files {
        let found = findings
            .iter()
            .any(|finding| path_matches_suffix(&finding.file, suffix));
        push_check(
            report,
            format!("review.forbidden_file.{suffix}"),
            !found,
            if found {
                summarize_findings(findings)
            } else {
                "not present".to_string()
            },
        );
    }
}

fn check_verification(
    report: &mut FixtureReport,
    artifacts: &EvalRunArtifacts,
    check: &VerificationCheck,
) {
    let Some(snapshot) = artifacts.verification.as_ref() else {
        push_check(
            report,
            "verification.snapshot",
            false,
            "verification plan was not requested",
        );
        return;
    };
    let commands = snapshot
        .steps
        .iter()
        .map(|step| step.command.clone())
        .collect::<Vec<_>>();
    report.metrics.verification_commands = commands.clone();

    for expected in &check.expected_commands {
        let found = commands.iter().any(|command| command == expected);
        push_check(
            report,
            format!("verification.command.{expected}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("commands={commands:?}")
            },
        );
    }
    for forbidden in &check.forbidden_commands {
        let found = commands.iter().any(|command| command == forbidden);
        push_check(
            report,
            format!("verification.forbidden_command.{forbidden}"),
            !found,
            if found {
                format!("commands={commands:?}")
            } else {
                "not present".to_string()
            },
        );
    }
    if let Some(expect) = check.expect_focused {
        let focused = snapshot
            .run
            .stats
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        push_check(
            report,
            "verification.focused",
            focused == expect,
            format!("focused={focused}, expected={expect}"),
        );
    }
    let focus_paths = snapshot
        .run
        .stats
        .get("focusPaths")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for suffix in &check.expected_focus_paths {
        let found = focus_paths
            .iter()
            .filter_map(Value::as_str)
            .any(|path| path_matches_suffix(path, suffix));
        push_check(
            report,
            format!("verification.focus_path.{suffix}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!("focusPaths={focus_paths:?}")
            },
        );
    }
}

fn check_improvement(
    report: &mut FixtureReport,
    artifacts: &EvalRunArtifacts,
    check: &ImprovementCheck,
) {
    let Some(snapshot) = artifacts.improvement.as_ref() else {
        push_check(
            report,
            "improvement.snapshot",
            false,
            "coding improvement report was not requested",
        );
        return;
    };

    if let Some(expected) = check.expected_scope.as_deref() {
        push_check(
            report,
            "improvement.scope",
            snapshot.scope == expected,
            format!("scope={}, expected={expected}", snapshot.scope),
        );
    }

    if let Some(min) = check.min_failures {
        push_check(
            report,
            "improvement.min_failures",
            snapshot.failures.len() >= min,
            format!("{} failure bucket(s), min {min}", snapshot.failures.len()),
        );
    }

    for category in &check.expected_failure_categories {
        let found = snapshot
            .failures
            .iter()
            .any(|failure| failure.category == *category);
        push_check(
            report,
            format!("improvement.failure.{category}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!(
                    "failures={:?}",
                    snapshot
                        .failures
                        .iter()
                        .map(|failure| failure.category.as_str())
                        .collect::<Vec<_>>()
                )
            },
        );
    }

    if let Some(min) = check.min_proposals {
        push_check(
            report,
            "improvement.min_proposals",
            snapshot.proposals.len() >= min,
            format!("{} proposal(s), min {min}", snapshot.proposals.len()),
        );
    }

    if let Some(min) = check.min_inserted_proposals {
        let inserted = artifacts
            .improvement_proposals
            .as_ref()
            .map(|result| result.inserted)
            .unwrap_or(0);
        push_check(
            report,
            "improvement.min_inserted_proposals",
            inserted >= min,
            format!("{inserted} inserted proposal(s), min {min}"),
        );
    }

    for kind in &check.expected_proposal_kinds {
        let found = snapshot
            .proposals
            .iter()
            .any(|proposal| proposal.kind == *kind);
        push_check(
            report,
            format!("improvement.proposal_kind.{kind}"),
            found,
            if found {
                "matched".to_string()
            } else {
                format!(
                    "proposalKinds={:?}",
                    snapshot
                        .proposals
                        .iter()
                        .map(|proposal| proposal.kind.as_str())
                        .collect::<Vec<_>>()
                )
            },
        );
    }

    if let Some(expect) = check.expect_draft_only {
        let draft_only = snapshot
            .proposals
            .iter()
            .all(|proposal| proposal.status == "draft");
        push_check(
            report,
            "improvement.draft_only",
            draft_only == expect,
            format!("draftOnly={draft_only}, expected={expect}"),
        );
    }

    if let Some(min) = check.min_eval_runs {
        push_check(
            report,
            "improvement.min_eval_runs",
            snapshot.eval.runs >= min,
            format!("{} eval run(s), min {min}", snapshot.eval.runs),
        );
    }

    if let Some(expected) = check.expect_eval_success_rate {
        let actual = snapshot.eval.success_rate.unwrap_or(-1.0);
        push_check(
            report,
            "improvement.eval_success_rate",
            (actual - expected).abs() <= 0.001,
            format!("{actual:.3}, expected {expected:.3}"),
        );
    }

    if let Some(min) = check.min_repair_loop_blocked {
        push_check(
            report,
            "improvement.repair_loop_blocked",
            snapshot.repair_loop.blocked >= min,
            format!(
                "{} blocked repair loop run(s), min {min}",
                snapshot.repair_loop.blocked
            ),
        );
    }

    if let Some(min) = check.min_retros {
        push_check(
            report,
            "improvement.min_retros",
            snapshot.retros.len() >= min,
            format!("{} retro(s), min {min}", snapshot.retros.len()),
        );
    }
    if let Some(min) = check.min_retro_recommendations {
        push_check(
            report,
            "improvement.min_retro_recommendations",
            snapshot.retro.recommendations >= min,
            format!(
                "{} recommendation(s), min {min}",
                snapshot.retro.recommendations
            ),
        );
    }

    if check.expected_applied_status.is_some()
        || check.expected_applied_kind.is_some()
        || check.min_applied_artifacts.is_some()
        || check.expected_action_target_contains.is_some()
    {
        let Some(result) = artifacts.improvement_apply.as_ref() else {
            push_check(
                report,
                "improvement.apply",
                false,
                "applyFirstProposal did not produce an apply result",
            );
            return;
        };

        if let Some(expected) = check.expected_applied_status.as_deref() {
            push_check(
                report,
                "improvement.applied_status",
                result.proposal.status == expected,
                format!("status={}, expected={expected}", result.proposal.status),
            );
        }
        if let Some(expected) = check.expected_applied_kind.as_deref() {
            push_check(
                report,
                "improvement.applied_kind",
                result.proposal.kind == expected,
                format!("kind={}, expected={expected}", result.proposal.kind),
            );
        }
        if let Some(min) = check.min_applied_artifacts {
            push_check(
                report,
                "improvement.min_applied_artifacts",
                result.artifacts.len() >= min,
                format!("{} artifact(s), min {min}", result.artifacts.len()),
            );
        }
        if let Some(needle) = check.expected_action_target_contains.as_deref() {
            let found = result
                .artifacts
                .iter()
                .any(|artifact| artifact.path.contains(needle))
                || result
                    .plan
                    .steps
                    .iter()
                    .any(|step| step.target_path.contains(needle));
            push_check(
                report,
                "improvement.action_target",
                found,
                if found {
                    "matched".to_string()
                } else {
                    format!(
                        "targets={:?}",
                        result
                            .plan
                            .steps
                            .iter()
                            .map(|step| step.target_path.as_str())
                            .collect::<Vec<_>>()
                    )
                },
            );
        }
    }

    if check.expected_promoted_status.is_some()
        || check.min_promoted_artifacts.is_some()
        || check.expected_promotion_target_contains.is_some()
    {
        let Some(result) = artifacts.improvement_promotion.as_ref() else {
            push_check(
                report,
                "improvement.promotion",
                false,
                "promoteAppliedProposal did not produce a promotion result",
            );
            return;
        };

        if let Some(expected) = check.expected_promoted_status.as_deref() {
            push_check(
                report,
                "improvement.promoted_status",
                result.proposal.status == expected,
                format!("status={}, expected={expected}", result.proposal.status),
            );
        }
        if let Some(min) = check.min_promoted_artifacts {
            push_check(
                report,
                "improvement.min_promoted_artifacts",
                result.artifacts.len() >= min,
                format!("{} artifact(s), min {min}", result.artifacts.len()),
            );
        }
        if let Some(needle) = check.expected_promotion_target_contains.as_deref() {
            let found = result
                .artifacts
                .iter()
                .any(|artifact| artifact.path.contains(needle))
                || result
                    .plan
                    .steps
                    .iter()
                    .any(|step| step.target_path.contains(needle));
            push_check(
                report,
                "improvement.promotion_target",
                found,
                if found {
                    "matched".to_string()
                } else {
                    format!(
                        "targets={:?}",
                        result
                            .plan
                            .steps
                            .iter()
                            .map(|step| step.target_path.as_str())
                            .collect::<Vec<_>>()
                    )
                },
            );
        }
    }
}

fn prepare_repo(base: &Path, fixture: &CodingEvalFixture) -> Result<PathBuf> {
    let repo_root = base.join(sanitize_name(&fixture.name));
    std::fs::create_dir_all(&repo_root)?;
    run_git(&repo_root, &["init"])?;
    run_git(
        &repo_root,
        &["config", "user.email", "eval@example.invalid"],
    )?;
    run_git(&repo_root, &["config", "user.name", "Hope Eval"])?;
    run_git(&repo_root, &["config", "commit.gpgsign", "false"])?;
    for file in &fixture.repo.files {
        write_fixture_file(&repo_root, file)?;
    }
    run_git(&repo_root, &["add", "."])?;
    run_git(&repo_root, &["commit", "-m", "baseline"])?;
    for file in &fixture.repo.changes {
        write_fixture_file(&repo_root, file)?;
    }
    Ok(repo_root)
}

fn seed_tasks(db: &SessionDB, session_id: &str, tasks: &[TaskFixture]) -> Result<()> {
    for task in tasks {
        let row = db.create_task(session_id, &task.content, task.active_form.as_deref())?;
        let status = parse_task_status(&task.status)?;
        if status != TaskStatus::Pending {
            db.update_task(row.id, Some(status), None, None)?;
        }
    }
    Ok(())
}

fn seed_workflow(
    db: &SessionDB,
    session_id: &str,
    goal_id: Option<&str>,
    workflow: &WorkflowFixture,
) -> Result<()> {
    let run = db.create_workflow_run(CreateWorkflowRunInput {
        session_id: session_id.to_string(),
        kind: workflow.kind.clone(),
        execution_mode: workflow.execution_mode.clone(),
        script_source: workflow.script_source.clone(),
        budget: json!({}),
        parent_run_id: None,
        origin: Some("eval".to_string()),
        goal_id: goal_id.map(ToOwned::to_owned),
        worktree_id: None,
    })?;
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("eval_seed"))?;
    for op in &workflow.ops {
        db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run.id.clone(),
            op_key: op.op_key.clone(),
            op_type: op.op_type.clone(),
            effect_class: parse_effect_class(&op.effect_class)?,
            input: op.input.clone(),
            child_handle: None,
        })?;
        match op.state.as_deref() {
            Some("failed") => {
                db.fail_workflow_op(
                    &run.id,
                    &op.op_key,
                    op.error
                        .clone()
                        .unwrap_or_else(|| json!({ "message": "eval seeded failure" })),
                )?;
            }
            Some("completed") => {
                db.complete_workflow_op(
                    &run.id,
                    &op.op_key,
                    op.output.clone().unwrap_or_else(|| json!({ "ok": true })),
                )?;
            }
            Some("started") | None => {}
            Some(other) => bail!("unsupported workflow op state: {other}"),
        }
    }
    Ok(())
}

fn write_fixture_file(root: &Path, file: &FileFixture) -> Result<()> {
    let path = root.join(&file.path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &file.text)
        .with_context(|| format!("writing fixture file {}", path.display()))
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn resolve_focus_paths(repo_root: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            let path = path.trim();
            let resolved = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                repo_root.join(path)
            };
            resolved
                .canonicalize()
                .unwrap_or(resolved)
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

fn candidate_matches(candidate: &ContextCandidate, expected: &CandidateExpectation) -> bool {
    if expected
        .kind
        .as_deref()
        .is_some_and(|kind| candidate_kind(candidate) != kind)
    {
        return false;
    }
    if expected
        .title_contains
        .as_deref()
        .is_some_and(|needle| !contains_ci(&candidate.title, needle))
    {
        return false;
    }
    if expected.path_suffix.as_deref().is_some_and(|suffix| {
        !candidate
            .path
            .as_deref()
            .is_some_and(|path| path_matches_suffix(path, suffix))
    }) {
        return false;
    }
    if expected.status_contains.as_deref().is_some_and(|needle| {
        !candidate
            .status
            .as_deref()
            .is_some_and(|status| contains_ci(status, needle))
    }) {
        return false;
    }
    if expected.source.as_deref().is_some_and(|source| {
        !candidate
            .sources
            .iter()
            .any(|candidate_source| candidate_source == source)
    }) {
        return false;
    }
    true
}

fn candidate_kind(candidate: &ContextCandidate) -> &'static str {
    match &candidate.kind {
        ContextCandidateKind::File => "file",
        ContextCandidateKind::Symbol => "symbol",
        ContextCandidateKind::Diagnostic => "diagnostic",
        ContextCandidateKind::ReviewFinding => "review_finding",
        ContextCandidateKind::VerificationStep => "verification_step",
        ContextCandidateKind::GoalEvidence => "goal_evidence",
        ContextCandidateKind::Task => "task",
        ContextCandidateKind::WorkflowOp => "workflow_op",
        ContextCandidateKind::IdeContext => "ide_context",
        ContextCandidateKind::UrlSource => "url_source",
    }
}

fn focus_paths(candidate: &ContextCandidate) -> Vec<String> {
    candidate
        .metadata
        .get("actions")
        .and_then(|actions| actions.get("focusPaths"))
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn path_matches_suffix(path: &str, suffix: &str) -> bool {
    let path = path.replace('\\', "/");
    let suffix = suffix.replace('\\', "/");
    path == suffix || path.ends_with(&format!("/{suffix}"))
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn summarize_candidates(candidates: &[ContextCandidate]) -> String {
    candidates
        .iter()
        .take(8)
        .map(|candidate| {
            format!(
                "{}:{}:{}",
                candidate_kind(candidate),
                candidate.title,
                candidate.status.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn summarize_findings(findings: &[review::ReviewFinding]) -> String {
    findings
        .iter()
        .take(8)
        .map(|finding| format!("{}:{}:{}", finding.title, finding.category, finding.file))
        .collect::<Vec<_>>()
        .join(", ")
}

fn push_check(
    report: &mut FixtureReport,
    name: impl Into<String>,
    passed: bool,
    detail: impl Into<String>,
) {
    report.outcomes.push(CheckOutcome {
        name: name.into(),
        passed,
        detail: detail.into(),
    });
}

impl CandidateExpectation {
    fn label(&self) -> String {
        [
            self.kind.as_deref().unwrap_or("*"),
            self.title_contains.as_deref().unwrap_or("*"),
            self.path_suffix.as_deref().unwrap_or("*"),
            self.status_contains.as_deref().unwrap_or("*"),
        ]
        .join(":")
    }
}

fn parse_task_status(status: &str) -> Result<TaskStatus> {
    TaskStatus::from_str(status).ok_or_else(|| anyhow!("unsupported task status: {status}"))
}

fn parse_effect_class(value: &str) -> Result<WorkflowEffectClass> {
    WorkflowEffectClass::from_str(value)
        .ok_or_else(|| anyhow!("unsupported workflow effect class: {value}"))
}

fn sanitize_name(name: &str) -> String {
    let out = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        "fixture".to_string()
    } else {
        out
    }
}

fn default_pending_status() -> String {
    "pending".to_string()
}

fn default_workflow_kind() -> String {
    "coding".to_string()
}

fn default_execution_mode() -> String {
    "guarded".to_string()
}

fn default_workflow_script() -> String {
    "await workflow.finish({ summary: 'eval fixture' });".to_string()
}

fn default_effect_class() -> String {
    "idempotent".to_string()
}
