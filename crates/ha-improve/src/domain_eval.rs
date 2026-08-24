//! 领域评测与三道闸的**机器**（阶段 5 第八刀，自 ha-core 迁出）：fixture
//! 跑批（含真实模型执行）、campaign 跑批、质量 / 就绪 / 运营三道闸判定与
//! soak 报表。
//!
//! 台账（`domain_eval_*` 表的读写方法、wire 类型、行映射、内置任务集）留在
//! [`ha_core::domain_eval`]——本模块只经类型化仓储方法触达 `sessions.db`。

use anyhow::{anyhow, bail, Result};
use ha_core::agent_loader::DEFAULT_AGENT_ID;
use ha_core::chat_engine::{ChatSource, NoopEventSink};
use ha_core::domain_quality::RunDomainQualityInput;
use ha_core::domain_workflow::RecordDomainEvidenceInput;
use ha_core::provider::{ActiveModel, ProviderConfig};
use ha_core::session::{MessageRole, NewMessage, SessionDB, SessionKind};
use ha_core::util::now_rfc3339;
use ha_core::workflow::CreateWorkflowRunInput;
use ha_core::workflow_mode::WorkflowMode;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// 同名 kernel 模块留守的契约类型与类型化仓储方法。
use ha_core::domain_eval::*;

const DEFAULT_MIN_EVAL_RUNS: usize = 1;

const DEFAULT_MIN_PASS_RATE: f64 = 1.0;

const DEFAULT_MIN_QUALITY_RUNS: usize = 1;

const DEFAULT_MAX_BLOCKED_QUALITY_RUNS: usize = 0;

const DEFAULT_MIN_DOMAIN_COVERAGE: usize = 1;

const DEFAULT_DOMAIN_READINESS_MIN_CAMPAIGN_ITEMS: usize = 1;

const DEFAULT_DOMAIN_READINESS_MIN_LEADERBOARD_ROWS: usize = 1;

const DEFAULT_DOMAIN_READINESS_MAX_FAILED_CAMPAIGN_ITEMS: usize = 0;

const DEFAULT_DOMAIN_READINESS_MAX_OPEN_LEARNING_PROPOSALS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MIN_WORKFLOW_RUNS: usize = 1;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_WORKFLOW_RUNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_BLOCKED_WORKFLOW_RUNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_CANCELLED_WORKFLOW_RUNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_WORKFLOW_RUNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MIN_LOOP_RUNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_LOOP_RUNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_CAMPAIGNS: usize = 0;

const DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_CAMPAIGN_ITEMS: usize = 0;

const DEFAULT_DOMAIN_SOAK_WINDOW_DAYS: u32 = 7;

const DEFAULT_DOMAIN_SOAK_MAX_ITEMS: usize = 12;

const MAX_DOMAIN_SOAK_MAX_ITEMS: usize = 50;

const DOMAIN_EVAL_SOURCE_CAMPAIGN: &str = "fixture_campaign";

pub async fn run_domain_eval_fixture(
    db: Arc<SessionDB>,
    input: RunDomainEvalFixtureInput,
) -> Result<DomainEvalFixtureReport> {
    let fixture = input.fixture;
    let name = non_empty(&fixture.name)
        .ok_or_else(|| anyhow!("fixture.name is required"))?
        .to_string();
    let execution_mode = non_empty(&fixture.execution_mode)
        .unwrap_or("trace_fixture")
        .to_string();
    let source_type = fixture_source_type(&execution_mode);
    let task_id = non_empty(&fixture.task_id)
        .ok_or_else(|| anyhow!("fixture.task_id is required"))?
        .to_string();
    let task = db
        .resolve_domain_eval_task(&task_id)?
        .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
    let session = db.create_session(DEFAULT_AGENT_ID)?;
    db.set_session_kind(&session.id, SessionKind::EvalFixture)?;
    let mut report = DomainEvalFixtureReport {
        fixture_run_id: None,
        name: name.clone(),
        execution_mode: execution_mode.clone(),
        source_type: source_type.clone(),
        status: "failed".to_string(),
        passed: false,
        session_id: session.id.clone(),
        goal_id: None,
        workflow_run_id: None,
        quality_run_id: None,
        eval_run: None,
        execution: None,
        checks: Vec::new(),
        error: None,
    };

    if !matches!(execution_mode.as_str(), "trace_fixture" | "agent") {
        report.error = Some(format!(
            "unsupported domain eval fixture execution mode {execution_mode:?}; expected trace_fixture or agent"
        ));
        report.checks.push(DomainEvalFixtureCheck {
            name: "execution_mode".to_string(),
            status: "failed".to_string(),
            expected: "trace_fixture or agent".to_string(),
            actual: execution_mode,
            detail: "Domain eval fixtures only support deterministic trace replay or explicit agent-backed execution.".to_string(),
        });
        db.persist_domain_eval_fixture_report(&mut report)?;
        return Ok(report);
    }

    let domain = fixture
        .domain
        .as_deref()
        .and_then(non_empty)
        .map(normalize_domain)
        .unwrap_or_else(|| task.domain.clone());
    let goal = db.create_goal(ha_core::goal::CreateGoalInput {
        session_id: session.id.clone(),
        objective: fixture
            .goal
            .objective
            .as_deref()
            .and_then(non_empty)
            .unwrap_or(&task.input.prompt)
            .to_string(),
        completion_criteria: fixture
            .goal
            .completion_criteria
            .as_deref()
            .and_then(non_empty)
            .unwrap_or_else(|| {
                task.success_criteria
                    .first()
                    .map(String::as_str)
                    .unwrap_or("Domain eval fixture criteria must pass.")
            })
            .to_string(),
        domain: Some(domain.clone()),
        workflow_template_id: fixture.goal.workflow_template_id.clone(),
        workflow_template_version: fixture.goal.workflow_template_version.clone(),
        workflow_task_type: fixture
            .goal
            .workflow_task_type
            .clone()
            .or_else(|| Some(task.task_type.clone())),
        budget_token_limit: None,
        budget_time_limit_secs: None,
        budget_turn_limit: None,
    })?;
    report.goal_id = Some(goal.goal.id.clone());

    if execution_mode == "agent" {
        let execution =
            run_domain_eval_agent_execution(db.clone(), &session.id, &task, &fixture).await?;
        let execution_failed = execution.status != "completed";
        report.execution = Some(execution);
        if execution_failed {
            report.checks = domain_eval_fixture_checks(&fixture.checks, None, &report.execution);
            report.error = report
                .execution
                .as_ref()
                .and_then(|execution| execution.error.clone())
                .or_else(|| Some("agent execution failed".to_string()));
            report.status = "failed".to_string();
            report.passed = false;
            db.persist_domain_eval_fixture_report(&mut report)?;
            return Ok(report);
        }
    }

    if execution_mode == "trace_fixture" {
        for evidence in &fixture.evidence {
            db.record_domain_evidence(RecordDomainEvidenceInput {
                goal_id: Some(goal.goal.id.clone()),
                session_id: Some(session.id.clone()),
                project_id: session.project_id.clone(),
                domain: domain.clone(),
                evidence_type: evidence.evidence_type.clone(),
                title: evidence.title.clone(),
                summary: evidence.summary.clone(),
                source_metadata: evidence.source_metadata.clone(),
                confidence: evidence.confidence.or(Some(0.95)),
                access_scope: Some("fixture".to_string()),
                redaction_status: Some("not_required".to_string()),
            })?;
        }

        if let Some(workflow) = fixture.workflow.clone() {
            let run = db.create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: workflow.kind,
                execution_mode: workflow.execution_mode,
                script_source: workflow.script_source,
                budget: json!({ "fixture": name }),
                parent_run_id: None,
                origin: Some("domain_eval_fixture".to_string()),
                goal_id: Some(goal.goal.id.clone()),
                goal_criterion_id: None,
                worktree_id: None,
            })?;
            report.workflow_run_id = Some(run.id);
        }
    }

    let source_quality_run_id = if fixture
        .quality
        .as_ref()
        .map(|quality| quality.run)
        .unwrap_or(true)
    {
        let quality = fixture.quality.clone().unwrap_or_default();
        let snapshot = crate::domain_quality::run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id.clone(),
                goal_id: Some(goal.goal.id.clone()),
                domain: Some(domain),
                template_id: fixture.goal.workflow_template_id.clone(),
                template_version: fixture.goal.workflow_template_version.clone(),
                profiles: Vec::new(),
                artifact_title: Some(task.title.clone()),
                artifact_kind: Some(task.task_type.clone()),
                source_metadata: fixture_quality_source_metadata(
                    quality.source_metadata,
                    &source_type,
                    &name,
                    &execution_mode,
                ),
                explicit_user_approval: quality.explicit_user_approval,
            },
        )?;
        let quality_run_id = snapshot.run.id;
        report.quality_run_id = Some(quality_run_id.clone());
        Some(quality_run_id)
    } else {
        None
    };

    let eval_run = db.run_domain_eval_task(RunDomainEvalTaskInput {
        session_id: session.id.clone(),
        task_id: task.id,
        label: fixture.label.clone().or_else(|| Some(name.clone())),
        source_quality_run_id,
        source_type: Some(source_type),
    })?;
    report.checks = domain_eval_fixture_checks(&fixture.checks, Some(&eval_run), &report.execution);
    let passed = report.checks.iter().all(|check| check.status == "passed");
    report.status = if passed { "passed" } else { "failed" }.to_string();
    report.passed = passed;
    report.eval_run = Some(eval_run);
    db.persist_domain_eval_fixture_report(&mut report)?;
    Ok(report)
}

pub fn evaluate_domain_quality_gate(
    db: &SessionDB,
    input: DomainQualityGateInput,
) -> Result<DomainQualityGateReport> {
    let thresholds = domain_quality_gate_thresholds(&input);
    let scope = resolve_domain_quality_gate_scope(db, &input)?;
    let summary = domain_quality_gate_summary(db, &scope)?;
    let mut checks = Vec::new();
    push_gate_check(
        &mut checks,
        "domain_eval_runs",
        if summary.eval_runs >= thresholds.min_eval_runs {
            "passed"
        } else {
            "insufficient_data"
        },
        "p1",
        format!("at least {} domain eval run(s)", thresholds.min_eval_runs),
        summary.eval_runs.to_string(),
        "Domain gate requires explicit non-coding eval evidence; coding benchmark runs do not count.",
    );
    push_gate_check(
        &mut checks,
        "domain_eval_pass_rate",
        match summary.pass_rate {
            Some(rate) if rate >= thresholds.min_pass_rate => "passed",
            Some(_) => "failed",
            None => "insufficient_data",
        },
        "p1",
        format!("pass rate >= {:.0}%", thresholds.min_pass_rate * 100.0),
        summary
            .pass_rate
            .map(|rate| format!("{:.0}%", rate * 100.0))
            .unwrap_or_else(|| "n/a".to_string()),
        "Failed or insufficient domain eval runs block the domain quality gate.",
    );
    push_gate_check(
        &mut checks,
        "domain_eval_average_score",
        match summary.average_score {
            Some(score) if score >= thresholds.min_average_score => "passed",
            Some(_) => "failed",
            None => "insufficient_data",
        },
        "p2",
        format!("average score >= {:.2}", thresholds.min_average_score),
        summary
            .average_score
            .map(|score| format!("{score:.2}"))
            .unwrap_or_else(|| "n/a".to_string()),
        "Average score catches partial evidence quality regressions even when status is not failed.",
    );
    push_gate_check(
        &mut checks,
        "domain_quality_runs",
        if summary.quality_runs >= thresholds.min_quality_runs {
            "passed"
        } else {
            "insufficient_data"
        },
        "p1",
        format!(
            "at least {} domain quality run(s)",
            thresholds.min_quality_runs
        ),
        summary.quality_runs.to_string(),
        "Domain Quality run/check history is required beside eval scoring.",
    );
    push_gate_check(
        &mut checks,
        "blocked_domain_quality",
        if summary.blocked_quality_runs
            + summary.failed_quality_runs
            + summary.needs_user_quality_runs
            <= thresholds.max_blocked_quality_runs
        {
            "passed"
        } else {
            "failed"
        },
        "p1",
        format!(
            "blocked/failed/needs_user quality runs <= {}",
            thresholds.max_blocked_quality_runs
        ),
        (summary.blocked_quality_runs
            + summary.failed_quality_runs
            + summary.needs_user_quality_runs)
            .to_string(),
        "Open domain quality blockers mean the non-coding task is not releasable.",
    );
    push_gate_check(
        &mut checks,
        "domain_coverage",
        if summary.domains_covered >= thresholds.min_domain_coverage {
            "passed"
        } else {
            "insufficient_data"
        },
        "p2",
        format!("at least {} domain(s)", thresholds.min_domain_coverage),
        summary.domains_covered.to_string(),
        "General eval must make the covered domains explicit and not masquerade as a global score.",
    );
    if thresholds.require_approval_safety {
        push_gate_check(
            &mut checks,
            "approval_safety",
            if summary.approval_blockers == 0 {
                "passed"
            } else {
                "failed"
            },
            "p1",
            "no approval blockers".to_string(),
            summary.approval_blockers.to_string(),
            "High-risk send/share/external-update actions must have explicit user approval evidence.",
        );
    }
    let status = gate_status(&checks);
    Ok(DomainQualityGateReport {
        generated_at: now_rfc3339(),
        status,
        scope: scope.scope,
        session_id: scope.session_id,
        project_id: scope.project_id,
        domain: scope.domain,
        window_days: scope.window_days,
        since: scope.since,
        thresholds,
        summary,
        checks,
    })
}

pub fn evaluate_domain_readiness_gate(
    db: &SessionDB,
    input: DomainReadinessGateInput,
) -> Result<DomainReadinessGateReport> {
    let thresholds = domain_readiness_gate_thresholds(&input);
    let quality_gate = evaluate_domain_quality_gate(
        db,
        DomainQualityGateInput {
            session_id: input.session_id.clone(),
            project_id: input.project_id.clone(),
            domain: input.domain.clone(),
            window_days: Some(thresholds.window_days),
            min_eval_runs: Some(thresholds.min_eval_runs),
            min_pass_rate: Some(thresholds.min_pass_rate),
            min_average_score: Some(thresholds.min_average_score),
            min_quality_runs: Some(thresholds.min_quality_runs),
            max_blocked_quality_runs: Some(thresholds.max_blocked_quality_runs),
            min_domain_coverage: Some(thresholds.min_domain_coverage),
            require_approval_safety: thresholds.require_approval_safety,
            include_synthetic: thresholds.include_synthetic,
        },
    )?;
    let campaign_leaderboard =
        db.get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
            session_id: quality_gate.session_id.clone(),
            project_id: quality_gate.project_id.clone(),
            domain: quality_gate.domain.clone(),
            window_days: Some(thresholds.window_days),
            limit: Some(MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT),
            campaign_ids: Vec::new(),
        })?;
    let mut summary = db.domain_readiness_campaign_summary(
        quality_gate.session_id.as_deref(),
        quality_gate.project_id.as_deref(),
        quality_gate.domain.as_deref(),
        &quality_gate.since,
    )?;
    summary.eval_runs = quality_gate.summary.eval_runs;
    summary.quality_runs = quality_gate.summary.quality_runs;
    summary.leaderboard_rows = campaign_leaderboard.rows.len();
    summary.quality_status = quality_gate.status.clone();
    summary.leaderboard_status = campaign_leaderboard.status.clone();

    let campaign_failures = summary.failed_campaign_items
        + summary.cancelled_campaign_items
        + summary.interrupted_campaign_items;
    let mut checks = Vec::new();
    push_readiness_check(
        &mut checks,
        "domain_quality_gate",
        &quality_gate.status,
        "blocking",
        "quality gate passed".to_string(),
        quality_gate.status.clone(),
        "Readiness keeps live domain eval and domain quality evidence as the first gate.",
    );
    push_readiness_check(
        &mut checks,
        "campaign_sample",
        if summary.campaign_items >= thresholds.min_campaign_items {
            "passed"
        } else {
            "insufficient_data"
        },
        "blocking",
        format!(
            "at least {} domain campaign item(s)",
            thresholds.min_campaign_items
        ),
        format!("{} item(s)", summary.campaign_items),
        "Durable campaign evidence proves the domain workflow can be replayed and compared.",
    );
    push_readiness_check(
        &mut checks,
        "campaign_completion",
        if summary.campaign_items < thresholds.min_campaign_items {
            "insufficient_data"
        } else if summary.active_campaigns == 0
            && summary.terminal_campaign_items >= summary.campaign_items
        {
            "passed"
        } else {
            "insufficient_data"
        },
        "blocking",
        "no active or queued domain campaigns".to_string(),
        format!(
            "{} active, {}/{} terminal item(s)",
            summary.active_campaigns, summary.terminal_campaign_items, summary.campaign_items
        ),
        "Running campaigns are observable long tasks, but readiness should wait for their final result.",
    );
    let leaderboard_status = if summary.leaderboard_rows < thresholds.min_leaderboard_rows {
        "insufficient_data"
    } else {
        campaign_leaderboard.status.as_str()
    };
    push_readiness_check(
        &mut checks,
        "campaign_leaderboard",
        leaderboard_status,
        "blocking",
        format!(
            "at least {} comparable leaderboard row(s) and no failed rows",
            thresholds.min_leaderboard_rows
        ),
        format!(
            "{} row(s), status {}",
            summary.leaderboard_rows, campaign_leaderboard.status
        ),
        "The same campaign evidence feeds the model/execution comparison view.",
    );
    push_readiness_check(
        &mut checks,
        "campaign_failures",
        if campaign_failures <= thresholds.max_failed_campaign_items {
            "passed"
        } else {
            "failed"
        },
        "blocking",
        format!(
            "<= {} failed/cancelled/interrupted campaign item(s)",
            thresholds.max_failed_campaign_items
        ),
        format!("{campaign_failures} failed/cancelled/interrupted item(s)"),
        "Known domain campaign failures must be retried, fixed, or allowed to age out of the gate window.",
    );
    push_readiness_check(
        &mut checks,
        "learning_closure",
        if summary.pending_learning_campaigns > 0
            || summary.open_learning_proposals > thresholds.max_open_learning_proposals
        {
            "failed"
        } else {
            "passed"
        },
        "blocking",
        format!(
            "no unmaterialized failures and <= {} open learning proposal(s)",
            thresholds.max_open_learning_proposals
        ),
        format!(
            "{} pending campaign(s), {} open proposal(s)",
            summary.pending_learning_campaigns, summary.open_learning_proposals
        ),
        "Failed campaign evidence should become domain eval cases or guidance drafts, then be resolved by the user.",
    );

    let blockers = checks
        .iter()
        .filter(|check| check.status != "passed" && check.severity != "advisory")
        .map(|check| check.name.clone())
        .collect::<Vec<_>>();
    let status = readiness_status(&checks);
    let recommended_next_steps = domain_readiness_recommendations(&checks);

    Ok(DomainReadinessGateReport {
        generated_at: now_rfc3339(),
        status,
        scope: quality_gate.scope.clone(),
        session_id: quality_gate.session_id.clone(),
        project_id: quality_gate.project_id.clone(),
        domain: quality_gate.domain.clone(),
        since: quality_gate.since.clone(),
        thresholds,
        summary,
        checks,
        quality_gate,
        campaign_leaderboard,
        blockers,
        recommended_next_steps,
    })
}

pub fn evaluate_domain_operational_gate(
    db: &SessionDB,
    input: DomainOperationalGateInput,
) -> Result<DomainOperationalGateReport> {
    let thresholds = domain_operational_gate_thresholds(&input);
    let scope = resolve_domain_operational_gate_scope(db, &input, thresholds.window_days)?;
    let summary = domain_operational_summary(db, &scope)?;
    let campaign_failures = summary.failed_campaign_items
        + summary.cancelled_campaign_items
        + summary.interrupted_campaign_items;
    let mut checks = Vec::new();
    push_operational_check(
        &mut checks,
        "workflow_sample",
        if summary.workflow_runs >= thresholds.min_workflow_runs {
            "passed"
        } else {
            "insufficient_data"
        },
        "blocking",
        format!("at least {} workflow run(s)", thresholds.min_workflow_runs),
        format!("{} workflow run(s)", summary.workflow_runs),
        "Operational readiness needs durable workflow evidence, not only eval or final text.",
    );
    push_operational_check(
        &mut checks,
        "workflow_failures",
        if summary.failed_workflow_runs <= thresholds.max_failed_workflow_runs
            && summary.blocked_workflow_runs <= thresholds.max_blocked_workflow_runs
            && summary.cancelled_workflow_runs <= thresholds.max_cancelled_workflow_runs
        {
            "passed"
        } else {
            "failed"
        },
        "blocking",
        format!(
            "<= {} failed, <= {} blocked, <= {} cancelled workflow run(s)",
            thresholds.max_failed_workflow_runs,
            thresholds.max_blocked_workflow_runs,
            thresholds.max_cancelled_workflow_runs
        ),
        format!(
            "{} failed, {} blocked, {} cancelled",
            summary.failed_workflow_runs,
            summary.blocked_workflow_runs,
            summary.cancelled_workflow_runs
        ),
        "Failed or blocked workflow runs must be repaired, retried, or allowed to age out before calling the scope operationally ready.",
    );
    push_operational_check(
        &mut checks,
        "workflow_active_drain",
        if summary.active_workflow_runs <= thresholds.max_active_workflow_runs {
            "passed"
        } else {
            "insufficient_data"
        },
        "blocking",
        format!(
            "<= {} active workflow run(s)",
            thresholds.max_active_workflow_runs
        ),
        format!(
            "{} active ({} paused, {} awaiting approval)",
            summary.active_workflow_runs,
            summary.paused_workflow_runs,
            summary.awaiting_approval_workflow_runs
        ),
        "Active workflow runs are observable, but an operational gate should wait for them to finish or be explicitly paused/cancelled.",
    );
    push_operational_check(
        &mut checks,
        "loop_sample",
        if summary.loop_runs >= thresholds.min_loop_runs {
            "passed"
        } else {
            "insufficient_data"
        },
        "advisory",
        format!("at least {} loop run(s)", thresholds.min_loop_runs),
        format!("{} loop run(s)", summary.loop_runs),
        "Loop evidence is optional by default, but raises confidence for recurring long tasks.",
    );
    push_operational_check(
        &mut checks,
        "loop_failures",
        if summary.failed_loop_runs <= thresholds.max_failed_loop_runs {
            "passed"
        } else {
            "failed"
        },
        "blocking",
        format!("<= {} failed loop run(s)", thresholds.max_failed_loop_runs),
        format!(
            "{} failed, {} active, {} schedule(s) active",
            summary.failed_loop_runs, summary.active_loop_runs, summary.active_loop_schedules
        ),
        "Recurring loops should not accumulate failed ticks without user-visible recovery.",
    );
    push_operational_check(
        &mut checks,
        "campaign_active_drain",
        if summary.active_campaigns <= thresholds.max_active_campaigns {
            "passed"
        } else {
            "insufficient_data"
        },
        "blocking",
        format!("<= {} active campaign(s)", thresholds.max_active_campaigns),
        format!("{} active campaign(s)", summary.active_campaigns),
        "Active campaigns prove observability, but final operational readiness should wait for terminal results.",
    );
    push_operational_check(
        &mut checks,
        "campaign_failures",
        if campaign_failures <= thresholds.max_failed_campaign_items {
            "passed"
        } else {
            "failed"
        },
        "blocking",
        format!(
            "<= {} failed/cancelled/interrupted campaign item(s)",
            thresholds.max_failed_campaign_items
        ),
        format!("{campaign_failures} failed/cancelled/interrupted item(s)"),
        "Campaign failures should be retried or turned into learning evidence before treating long-run behavior as stable.",
    );

    let blockers = checks
        .iter()
        .filter(|check| check.status != "passed" && check.severity != "advisory")
        .map(|check| check.name.clone())
        .collect::<Vec<_>>();
    let status = operational_status(&checks);
    let recommended_next_steps = domain_operational_recommendations(&checks);

    Ok(DomainOperationalGateReport {
        generated_at: now_rfc3339(),
        status,
        scope: scope.scope,
        session_id: scope.session_id,
        project_id: scope.project_id,
        domain: scope.domain,
        since: scope.since,
        thresholds,
        summary,
        checks,
        blockers,
        recommended_next_steps,
    })
}

pub fn generate_domain_soak_report(
    db: &SessionDB,
    input: DomainSoakReportInput,
) -> Result<DomainSoakReport> {
    let window_days = input
        .window_days
        .unwrap_or(DEFAULT_DOMAIN_SOAK_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);
    let max_items = input
        .max_items
        .unwrap_or(DEFAULT_DOMAIN_SOAK_MAX_ITEMS)
        .clamp(1, MAX_DOMAIN_SOAK_MAX_ITEMS);
    let operational_input = DomainOperationalGateInput {
        session_id: input.session_id.clone(),
        project_id: input.project_id.clone(),
        domain: input.domain.clone(),
        window_days: Some(window_days),
        min_workflow_runs: Some(1),
        max_failed_workflow_runs: Some(0),
        max_blocked_workflow_runs: Some(0),
        max_cancelled_workflow_runs: Some(0),
        max_active_workflow_runs: Some(0),
        min_loop_runs: Some(0),
        max_failed_loop_runs: Some(0),
        max_active_campaigns: Some(0),
        max_failed_campaign_items: Some(0),
    };
    let scope = resolve_domain_operational_gate_scope(db, &operational_input, window_days)?;
    let operational_gate = evaluate_domain_operational_gate(db, operational_input)?;
    let until = now_rfc3339();
    let mut summary = DomainSoakReportSummary {
        required_sample_days: if window_days > 1 { 2 } else { 1 },
        ..Default::default()
    };
    let mut sample_days = BTreeSet::new();
    let mut incidents = Vec::new();
    let mut timeline = Vec::new();

    let workflow_rows = db.domain_soak_workflows(&scope)?;
    let mut workflow_durations = Vec::new();
    for row in workflow_rows {
        track_soak_sample_day(&mut sample_days, &row.created_at);
        summary.workflow_runs += 1;
        if row
            .origin
            .as_deref()
            .is_some_and(|origin| origin.contains("repair"))
        {
            summary.repair_workflow_runs += 1;
        }
        let duration = row
            .completed_at
            .as_deref()
            .and_then(|finished| timestamp_delta_secs(&row.created_at, finished));
        if let Some(duration) = duration {
            workflow_durations.push(duration);
        }
        match row.state.as_str() {
            "completed" => summary.completed_workflow_runs += 1,
            "failed" => {
                summary.failed_workflow_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "workflow",
                    &row.id,
                    &row.kind,
                    &row.state,
                    "critical",
                    Some(row.created_at.clone()),
                    row.completed_at.clone().or_else(|| Some(row.updated_at.clone())),
                    duration,
                    row.blocked_reason
                        .clone()
                        .unwrap_or_else(|| "workflow failed".to_string()),
                    "Open the Workflow run detail, inspect failed ops, and retry through a repair workflow.".to_string(),
                );
            }
            "blocked" => {
                summary.blocked_workflow_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "workflow",
                    &row.id,
                    &row.kind,
                    &row.state,
                    "critical",
                    Some(row.created_at.clone()),
                    row.completed_at.clone().or_else(|| Some(row.updated_at.clone())),
                    duration,
                    row.blocked_reason
                        .clone()
                        .unwrap_or_else(|| "workflow blocked".to_string()),
                    "Resolve the blocker or explicitly cancel/restart the workflow before unattended continuation.".to_string(),
                );
            }
            "cancelled" => {
                summary.cancelled_workflow_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "workflow",
                    &row.id,
                    &row.kind,
                    &row.state,
                    "critical",
                    Some(row.created_at.clone()),
                    row.completed_at.clone().or_else(|| Some(row.updated_at.clone())),
                    duration,
                    "workflow was cancelled".to_string(),
                    "Confirm cancellation was intentional or retry with a smaller bounded workflow.".to_string(),
                );
            }
            "awaiting_approval" => {
                summary.active_workflow_runs += 1;
                summary.awaiting_approval_workflow_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "workflow",
                    &row.id,
                    &row.kind,
                    &row.state,
                    "warning",
                    Some(row.created_at.clone()),
                    None,
                    timestamp_delta_secs(&row.created_at, &until),
                    "workflow is waiting for approval".to_string(),
                    "Approve, deny, pause, or cancel the workflow so long-running work can drain."
                        .to_string(),
                );
            }
            "running" | "recovering" | "awaiting_user" | "paused" | "draft" => {
                summary.active_workflow_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "workflow",
                    &row.id,
                    &row.kind,
                    &row.state,
                    "warning",
                    Some(row.created_at.clone()),
                    None,
                    timestamp_delta_secs(&row.created_at, &until),
                    format!("workflow is {}", row.state),
                    "Let the workflow finish or explicitly pause/cancel stale active work."
                        .to_string(),
                );
            }
            _ => {}
        }
        timeline.push(DomainSoakTimelineItem {
            source: "workflow".to_string(),
            id: row.id,
            label: row.kind,
            status: row.state,
            at: row.completed_at.unwrap_or(row.updated_at),
            duration_secs: duration,
        });
        if let Some(item) = timeline.last() {
            track_soak_activity(&mut summary, &mut sample_days, &item.at);
        }
    }
    summary.average_workflow_drain_secs = average_secs(&workflow_durations);
    summary.max_workflow_drain_secs = workflow_durations.iter().copied().max();

    let mut approval_wait_started: BTreeMap<String, String> = BTreeMap::new();
    let mut approval_wait_durations = Vec::new();
    for event in db.domain_soak_workflow_events(&scope)? {
        track_soak_activity(&mut summary, &mut sample_days, &event.created_at);
        if event.event_type == "run_control_action" {
            match json_string_value(&event.payload, "action").as_deref() {
                Some("approve") => {
                    summary.approval_events += 1;
                    summary.workflow_control_intervention_events += 1;
                }
                Some("pause") => {
                    summary.pause_events += 1;
                    summary.workflow_control_intervention_events += 1;
                }
                Some("resume") => {
                    summary.resume_events += 1;
                    summary.workflow_control_intervention_events += 1;
                }
                Some("cancel") => {
                    summary.cancel_events += 1;
                    summary.workflow_control_intervention_events += 1;
                }
                _ => {}
            }
        }
        if event.event_type == "run_recovery_claimed"
            || event
                .payload
                .get("to")
                .and_then(Value::as_str)
                .is_some_and(|state| state == "recovering")
        {
            summary.recovery_events += 1;
        }
        if event.event_type == "budget_usage" {
            summary.workflow_budget_usage_events += 1;
            let spent = event
                .payload
                .get("spentOutputTokens")
                .and_then(Value::as_u64);
            let limit = event.payload.get("maxOutputTokens").and_then(Value::as_u64);
            if event
                .payload
                .get("exhausted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                summary.workflow_budget_exhausted_events += 1;
            }
            if let Some(spent) = spent {
                let replace = summary
                    .max_workflow_output_tokens_spent
                    .map_or(true, |current| spent > current);
                if replace {
                    summary.max_workflow_output_tokens_spent = Some(spent);
                    summary.max_workflow_output_token_budget = limit;
                }
            }
        }
        if event.event_type == "run_state_changed"
            && event
                .payload
                .get("to")
                .and_then(Value::as_str)
                .is_some_and(|state| state == "awaiting_approval")
        {
            summary.approval_events += 1;
            summary.approval_request_events += 1;
            approval_wait_started.insert(event.run_id.clone(), event.created_at.clone());
        }
        if event.event_type == "run_state_changed"
            && event
                .payload
                .get("from")
                .and_then(Value::as_str)
                .is_some_and(|state| state == "awaiting_approval")
            && event
                .payload
                .get("to")
                .and_then(Value::as_str)
                .is_none_or(|state| state != "awaiting_approval")
        {
            summary.approval_decision_events += 1;
            if let Some(started_at) = approval_wait_started.remove(&event.run_id) {
                if let Some(duration) = timestamp_delta_secs(&started_at, &event.created_at) {
                    approval_wait_durations.push(duration);
                }
            }
        }
    }
    summary.average_approval_wait_secs = average_secs(&approval_wait_durations);
    summary.max_approval_wait_secs = approval_wait_durations.iter().copied().max();
    let open_approval_wait_durations: Vec<i64> = approval_wait_started
        .values()
        .filter_map(|started_at| timestamp_delta_secs(started_at, &until))
        .collect();
    summary.open_approval_waits = open_approval_wait_durations.len();
    summary.max_open_approval_wait_secs = open_approval_wait_durations.iter().copied().max();

    let loop_rows = db.domain_soak_loop_runs(&scope)?;
    let mut loop_durations = Vec::new();
    for row in loop_rows {
        track_soak_sample_day(&mut sample_days, &row.started_at);
        summary.loop_runs += 1;
        let duration = row
            .finished_at
            .as_deref()
            .and_then(|finished| timestamp_delta_secs(&row.started_at, finished));
        if let Some(duration) = duration {
            loop_durations.push(duration);
        }
        match row.state.as_str() {
            "succeeded" | "empty" | "skipped" => summary.succeeded_loop_runs += 1,
            "failed" | "cancelled" => {
                summary.failed_loop_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "loop",
                    &row.id,
                    &row.trigger_reason,
                    &row.state,
                    "critical",
                    Some(row.started_at.clone()),
                    row.finished_at.clone(),
                    duration,
                    row.error
                        .clone()
                        .or(row.result_summary.clone())
                        .unwrap_or_else(|| "loop run failed".to_string()),
                    "Inspect the loop schedule and update its strategy before continuing unattended.".to_string(),
                );
            }
            "running" | "queued" | "injected" => {
                summary.active_loop_runs += 1;
                push_soak_incident(
                    &mut incidents,
                    "loop",
                    &row.id,
                    &row.trigger_reason,
                    &row.state,
                    "warning",
                    Some(row.started_at.clone()),
                    None,
                    timestamp_delta_secs(&row.started_at, &until),
                    format!("loop run is {}", row.state),
                    "Wait for the tick to finish or stop the loop if it is stale.".to_string(),
                );
            }
            _ => {}
        }
        timeline.push(DomainSoakTimelineItem {
            source: "loop".to_string(),
            id: row.id,
            label: format!("{}: {}", row.loop_id, row.trigger_reason),
            status: row.state,
            at: row.finished_at.unwrap_or(row.started_at),
            duration_secs: duration,
        });
        if let Some(item) = timeline.last() {
            track_soak_activity(&mut summary, &mut sample_days, &item.at);
        }
    }
    summary.average_loop_duration_secs = average_secs(&loop_durations);
    summary.max_loop_duration_secs = loop_durations.iter().copied().max();

    let campaign_rows = db.domain_soak_campaign_rows(&scope)?;
    let mut campaign_ids = BTreeSet::new();
    let mut item_ids = BTreeSet::new();
    let mut campaign_item_durations = Vec::new();
    for row in campaign_rows {
        if campaign_ids.insert(row.campaign_id.clone()) {
            summary.campaigns += 1;
            if matches!(
                row.campaign_status.as_str(),
                "queued" | "running" | "cancel_requested"
            ) {
                summary.active_campaigns += 1;
            }
            timeline.push(DomainSoakTimelineItem {
                source: "campaign".to_string(),
                id: row.campaign_id.clone(),
                label: row.campaign_name.clone(),
                status: row.campaign_status.clone(),
                at: row.campaign_updated_at.clone(),
                duration_secs: None,
            });
            track_soak_activity(&mut summary, &mut sample_days, &row.campaign_updated_at);
        }
        let Some(item_id) = row.item_id.clone() else {
            continue;
        };
        if !item_ids.insert(item_id.clone()) {
            continue;
        }
        if let Some(started_at) = row.item_started_at.as_deref() {
            track_soak_sample_day(&mut sample_days, started_at);
        }
        summary.campaign_items += 1;
        let item_status = row.item_status.clone().unwrap_or_default();
        let duration = row
            .item_started_at
            .as_deref()
            .zip(row.item_finished_at.as_deref())
            .and_then(|(started, finished)| timestamp_delta_secs(started, finished));
        if let Some(duration) = duration {
            campaign_item_durations.push(duration);
        }
        if row.item_attempt.unwrap_or(0) > 1 {
            summary.retried_campaign_items += 1;
        }
        match item_status.as_str() {
            "passed" => summary.passed_campaign_items += 1,
            "failed" | "partial" => {
                summary.failed_campaign_items += 1;
                push_soak_incident(
                    &mut incidents,
                    "campaign_item",
                    &item_id,
                    row.item_title.as_deref().unwrap_or(&row.campaign_name),
                    &item_status,
                    "critical",
                    row.item_started_at.clone(),
                    row.item_finished_at.clone(),
                    duration,
                    row.item_error
                        .clone()
                        .unwrap_or_else(|| "campaign item failed".to_string()),
                    "Retry the failed campaign item or convert the failure into a learning proposal.".to_string(),
                );
            }
            "cancelled" => {
                summary.cancelled_campaign_items += 1;
                push_soak_incident(
                    &mut incidents,
                    "campaign_item",
                    &item_id,
                    row.item_title.as_deref().unwrap_or(&row.campaign_name),
                    &item_status,
                    "critical",
                    row.item_started_at.clone(),
                    row.item_finished_at.clone(),
                    duration,
                    "campaign item was cancelled".to_string(),
                    "Confirm cancellation was intentional or retry the item when resources are available.".to_string(),
                );
            }
            "interrupted" => {
                summary.interrupted_campaign_items += 1;
                push_soak_incident(
                    &mut incidents,
                    "campaign_item",
                    &item_id,
                    row.item_title.as_deref().unwrap_or(&row.campaign_name),
                    &item_status,
                    "critical",
                    row.item_started_at.clone(),
                    row.item_finished_at.clone(),
                    duration,
                    row.item_error
                        .clone()
                        .unwrap_or_else(|| "campaign item interrupted".to_string()),
                    "Retry interrupted items after checking provider/runtime availability."
                        .to_string(),
                );
            }
            "queued" | "running" => {
                push_soak_incident(
                    &mut incidents,
                    "campaign_item",
                    &item_id,
                    row.item_title.as_deref().unwrap_or(&row.campaign_name),
                    &item_status,
                    "warning",
                    row.item_started_at.clone().or(row.item_updated_at.clone()),
                    None,
                    row.item_started_at
                        .as_deref()
                        .and_then(|started| timestamp_delta_secs(started, &until)),
                    format!("campaign item is {item_status}"),
                    "Wait for active campaign items to finish or cancel stale campaigns."
                        .to_string(),
                );
            }
            _ => {}
        }
        timeline.push(DomainSoakTimelineItem {
            source: "campaign_item".to_string(),
            id: item_id,
            label: row.item_title.unwrap_or(row.campaign_name),
            status: item_status,
            at: row
                .item_finished_at
                .or(row.item_updated_at)
                .or(row.item_started_at)
                .unwrap_or(row.campaign_updated_at),
            duration_secs: duration,
        });
        if let Some(item) = timeline.last() {
            track_soak_activity(&mut summary, &mut sample_days, &item.at);
        }
    }
    summary.average_campaign_item_duration_secs = average_secs(&campaign_item_durations);
    summary.max_campaign_item_duration_secs = campaign_item_durations.iter().copied().max();

    let (
        connector_e2e,
        connector_execution,
        connector_verification,
        connector_latest,
        connector_sample_days,
    ) = db.domain_soak_connector_evidence_counts(&scope)?;
    summary.connector_e2e_evidence = connector_e2e;
    summary.connector_execution_evidence = connector_execution;
    summary.connector_verification_evidence = connector_verification;
    if let Some(connector_latest) = connector_latest.as_deref() {
        track_soak_activity(&mut summary, &mut sample_days, connector_latest);
    }
    sample_days.extend(connector_sample_days);
    if summary.connector_execution_evidence > 0 && summary.connector_verification_evidence == 0 {
        push_soak_incident(
            &mut incidents,
            "connector_e2e",
            "connector_verification_missing",
            "Connector action verification missing",
            "unverified",
            "warning",
            connector_latest,
            None,
            None,
            format!(
                "{} connector execution evidence item(s) have no post-action verification",
                summary.connector_execution_evidence
            ),
            "Record connector_action_verified evidence by reading back the external system state before trusting unattended connector runs.".to_string(),
        );
    }
    summary.sample_days = sample_days.len();
    summary.latest_activity_age_secs = summary
        .latest_activity_at
        .as_deref()
        .and_then(|activity_at| timestamp_delta_secs(activity_at, &until));

    incidents.sort_by(|a, b| {
        incident_rank(&a.severity)
            .cmp(&incident_rank(&b.severity))
            .then_with(|| b.started_at.cmp(&a.started_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    timeline.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
    summary.incidents = incidents.len();
    summary.critical_incidents = incidents
        .iter()
        .filter(|incident| incident.severity == "critical")
        .count();
    summary.warning_incidents = incidents
        .iter()
        .filter(|incident| incident.severity == "warning")
        .count();
    if incidents.len() > max_items {
        incidents.truncate(max_items);
    }
    if timeline.len() > max_items {
        timeline.truncate(max_items);
    }
    summary.total_records = summary.workflow_runs
        + summary.loop_runs
        + summary.campaign_items
        + summary.connector_e2e_evidence;
    let stale_activity = summary
        .latest_activity_age_secs
        .is_some_and(|age| age > 24 * 60 * 60);
    let insufficient_day_coverage =
        summary.sample_days < summary.required_sample_days && summary.total_records > 0;
    let status = if summary.total_records == 0 {
        "insufficient_data"
    } else if summary.critical_incidents > 0 || operational_gate.status == "failed" {
        "failed"
    } else if summary.warning_incidents > 0
        || operational_gate.status == "insufficient_data"
        || stale_activity
        || insufficient_day_coverage
    {
        "insufficient_data"
    } else {
        "passed"
    }
    .to_string();
    let recommended_next_steps =
        domain_soak_recommendations(&summary, &incidents, &operational_gate);
    let markdown = render_domain_soak_markdown(
        &scope,
        &until,
        &status,
        &summary,
        &incidents,
        &recommended_next_steps,
    );

    Ok(DomainSoakReport {
        generated_at: until.clone(),
        status,
        scope: scope.scope,
        session_id: scope.session_id,
        project_id: scope.project_id,
        domain: scope.domain,
        window_days: scope.window_days,
        since: scope.since,
        until,
        summary,
        incidents,
        timeline,
        recommended_next_steps,
        markdown,
        operational_gate,
    })
}

pub fn domain_operational_summary(
    db: &SessionDB,
    scope: &DomainGateScope,
) -> Result<DomainOperationalGateSummary> {
    let mut summary = DomainOperationalGateSummary::default();
    let now = now_rfc3339();
    db.fill_domain_operational_workflows(scope, &mut summary, &now)?;
    db.fill_domain_operational_loops(scope, &mut summary, &now)?;
    db.fill_domain_operational_campaigns(scope, &mut summary, &now)?;
    Ok(summary)
}

pub fn resolve_domain_quality_gate_scope(
    db: &SessionDB,
    input: &DomainQualityGateInput,
) -> Result<DomainGateScope> {
    let window_days = input
        .window_days
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);
    let since = since_timestamp(window_days);
    let domain = input
        .domain
        .as_deref()
        .and_then(non_empty)
        .map(normalize_domain);
    if let Some(session_id) = input.session_id.as_deref().and_then(non_empty) {
        let session = db
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("domain quality gate is disabled for incognito sessions");
        }
        return Ok(DomainGateScope {
            scope: "session".to_string(),
            session_id: Some(session.id),
            project_id: session.project_id,
            domain,
            window_days,
            since,
            include_synthetic: input.include_synthetic,
        });
    }
    if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
        return Ok(DomainGateScope {
            scope: "project".to_string(),
            session_id: None,
            project_id: Some(project_id.to_string()),
            domain,
            window_days,
            since,
            include_synthetic: input.include_synthetic,
        });
    }
    Ok(DomainGateScope {
        scope: "global".to_string(),
        session_id: None,
        project_id: None,
        domain,
        window_days,
        since,
        include_synthetic: input.include_synthetic,
    })
}

pub fn resolve_domain_operational_gate_scope(
    db: &SessionDB,
    input: &DomainOperationalGateInput,
    window_days: u32,
) -> Result<DomainGateScope> {
    let since = since_timestamp(window_days);
    let domain = input
        .domain
        .as_deref()
        .and_then(non_empty)
        .map(normalize_domain);
    if let Some(session_id) = input.session_id.as_deref().and_then(non_empty) {
        let session = db
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if session.incognito {
            bail!("domain operational gate is disabled for incognito sessions");
        }
        return Ok(DomainGateScope {
            scope: "session".to_string(),
            session_id: Some(session.id),
            project_id: session.project_id,
            domain,
            window_days,
            since,
            include_synthetic: false,
        });
    }
    if let Some(project_id) = input.project_id.as_deref().and_then(non_empty) {
        return Ok(DomainGateScope {
            scope: "project".to_string(),
            session_id: None,
            project_id: Some(project_id.to_string()),
            domain,
            window_days,
            since,
            include_synthetic: false,
        });
    }
    Ok(DomainGateScope {
        scope: "global".to_string(),
        session_id: None,
        project_id: None,
        domain,
        window_days,
        since,
        include_synthetic: false,
    })
}

pub fn domain_quality_gate_summary(
    db: &SessionDB,
    scope: &DomainGateScope,
) -> Result<DomainQualityGateSummary> {
    let runs = db.list_domain_eval_runs(ListDomainEvalRunsInput {
        session_id: scope.session_id.clone(),
        project_id: scope.project_id.clone(),
        domain: scope.domain.clone(),
        window_days: Some(scope.window_days),
        limit: Some(MAX_DOMAIN_EVAL_LIMIT),
        include_synthetic: scope.include_synthetic,
        ..Default::default()
    })?;
    let mut summary = DomainQualityGateSummary {
        eval_runs: runs.len(),
        ..Default::default()
    };
    let mut score_sum = 0.0;
    let mut domains = BTreeSet::new();
    for run in runs {
        domains.insert(run.domain);
        score_sum += run.score;
        match run.status.as_str() {
            "passed" => summary.passed_eval_runs += 1,
            "failed" => summary.failed_eval_runs += 1,
            _ => summary.insufficient_eval_runs += 1,
        }
    }
    if summary.eval_runs > 0 {
        summary.pass_rate = Some(summary.passed_eval_runs as f64 / summary.eval_runs as f64);
        summary.average_score = Some(score_sum / summary.eval_runs as f64);
    }
    let quality_rows = db.domain_quality_gate_quality_rows(scope)?;
    for row in &quality_rows {
        domains.insert(row.domain.clone());
        summary.quality_runs += 1;
        match row.state.as_str() {
            "completed" => summary.completed_quality_runs += 1,
            "blocked" => summary.blocked_quality_runs += 1,
            "failed" => summary.failed_quality_runs += 1,
            "needs_user" => summary.needs_user_quality_runs += 1,
            _ => {}
        }
        summary.approval_blockers += row
            .checks
            .iter()
            .filter(|(check_type, status)| {
                check_type == "approval"
                    && matches!(status.as_str(), "needs_user" | "failed" | "blocked")
            })
            .count();
    }
    summary.domains_covered = domains.len();
    let evidence_counts = db.domain_quality_gate_evidence_counts(scope)?;
    summary.evidence_items = evidence_counts.values().sum();
    summary.source_cited = evidence_counts.get("source_cited").copied().unwrap_or(0);
    summary.dated_sources = db.domain_quality_gate_dated_sources(scope)?;
    summary.data_quality_checked = evidence_counts
        .get("data_quality_checked")
        .copied()
        .unwrap_or(0);
    Ok(summary)
}

pub async fn run_domain_eval_campaign(
    db: Arc<SessionDB>,
    input: RunDomainEvalCampaignInput,
) -> Result<DomainEvalCampaign> {
    let campaign_id = input.campaign_id.trim().to_string();
    if campaign_id.is_empty() {
        bail!("domain eval campaign id must not be empty");
    }
    let items = db.prepare_domain_eval_campaign_run(&campaign_id, input.retry_failed_only)?;
    for queued_item in items {
        if db.is_domain_eval_campaign_cancel_requested(&campaign_id)? {
            break;
        }
        let Some(item) = db.mark_domain_eval_campaign_item_running(&queued_item.id)? else {
            continue;
        };
        let campaign = db
            .get_domain_eval_campaign(&campaign_id)?
            .ok_or_else(|| anyhow!("domain eval campaign not found: {campaign_id}"))?;
        let task = db
            .resolve_domain_eval_task(&item.task_id)?
            .ok_or_else(|| anyhow!("domain eval task not found: {}", item.task_id))?;
        let label = domain_eval_campaign_item_label(&campaign, &item);
        let mut fixture = domain_eval_campaign_fixture(&task, &item.execution_mode, &label);
        if item.execution_mode == "agent" {
            let Some(provider_id) = item.provider_id.clone() else {
                db.fail_domain_eval_campaign_item(
                    &item.id,
                    "agent campaign item is missing providerId",
                )?;
                continue;
            };
            let Some(model_id) = item.model_id.clone() else {
                db.fail_domain_eval_campaign_item(
                    &item.id,
                    "agent campaign item is missing modelId",
                )?;
                continue;
            };
            let Some(provider_config) =
                domain_campaign_provider_config(&provider_id, &input.providers)
            else {
                db.fail_domain_eval_campaign_item(
                    &item.id,
                    &format!(
                        "Provider config for {provider_id} was not supplied or is masked; campaign history never stores provider secrets"
                    ),
                )?;
                continue;
            };
            fixture.execution.providers = vec![provider_config];
            fixture.execution.model_chain = vec![ActiveModel {
                provider_id,
                model_id,
            }];
        }

        match run_domain_eval_fixture(db.clone(), RunDomainEvalFixtureInput { fixture }).await {
            Ok(report) => {
                db.finish_domain_eval_campaign_item(&item.id, &report)?;
            }
            Err(err) => {
                db.fail_domain_eval_campaign_item(&item.id, &err.to_string())?;
            }
        }
    }
    db.complete_domain_eval_campaign(&campaign_id)?;
    db.get_domain_eval_campaign(&campaign_id)?
        .ok_or_else(|| anyhow!("domain eval campaign not found after run: {campaign_id}"))
}

fn domain_campaign_provider_config(
    provider_id: &str,
    supplied: &[ProviderConfig],
) -> Option<ProviderConfig> {
    supplied
        .iter()
        .find(|provider| {
            provider.id == provider_id && !ha_core::provider::is_masked_key(&provider.api_key)
        })
        .cloned()
        .or_else(|| {
            ha_core::config::cached_config()
                .providers
                .iter()
                .find(|provider| {
                    provider.id == provider_id
                        && !ha_core::provider::is_masked_key(&provider.api_key)
                })
                .cloned()
        })
}

fn domain_eval_campaign_item_label(
    campaign: &DomainEvalCampaign,
    item: &DomainEvalCampaignItem,
) -> String {
    format!(
        "{} · {} · {}",
        campaign.name,
        item.task_id,
        item.label
            .clone()
            .or_else(|| {
                item.provider_id
                    .as_ref()
                    .zip(item.model_id.as_ref())
                    .map(|(provider_id, model_id)| format!("{provider_id}/{model_id}"))
            })
            .unwrap_or_else(|| item.execution_mode.clone())
    )
}

fn domain_eval_campaign_fixture(
    task: &DomainEvalTask,
    execution_mode: &str,
    label: &str,
) -> DomainEvalFixture {
    let mut evidence = Vec::new();
    for req in &task.required_evidence {
        let count = req.min_count.max(1);
        for index in 0..count {
            evidence.push(DomainEvalFixtureEvidence {
                evidence_type: req.evidence_type.clone(),
                title: format!("{} #{}", req.title, index + 1),
                summary: Some(format!(
                    "Synthetic campaign evidence for {} requirement {}.",
                    task.id, req.evidence_type
                )),
                source_metadata: campaign_evidence_metadata(task, req, index),
                confidence: Some(0.95),
            });
        }
    }
    if matches!(task.domain.as_str(), "research" | "knowledge_curation")
        && !evidence
            .iter()
            .any(|item| item.evidence_type == "source_cited")
    {
        evidence.push(DomainEvalFixtureEvidence {
            evidence_type: "source_cited".to_string(),
            title: "Synthetic cited source".to_string(),
            summary: Some("Synthetic campaign citation with date metadata.".to_string()),
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "url": "https://example.invalid/domain-eval-campaign",
                "retrievedAt": now_rfc3339(),
                "publishedAt": "2026-01-01",
            }),
            confidence: Some(0.95),
        });
    }
    if task.domain == "data_analysis"
        && !evidence
            .iter()
            .any(|item| item.evidence_type == "data_quality_checked")
    {
        evidence.push(DomainEvalFixtureEvidence {
            evidence_type: "data_quality_checked".to_string(),
            title: "Synthetic data quality check".to_string(),
            summary: Some(
                "Synthetic campaign metric definition and sample-size check.".to_string(),
            ),
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "dataset": "domain_eval_campaign_fixture",
                "metric": "quality_score",
                "denominator": "eligible records",
                "sampleSize": 128,
            }),
            confidence: Some(0.95),
        });
    }
    if task.domain == "meeting_prep"
        && !evidence
            .iter()
            .any(|item| item.evidence_type == "artifact_created")
    {
        evidence.push(DomainEvalFixtureEvidence {
            evidence_type: "artifact_created".to_string(),
            title: "Synthetic meeting brief artifact".to_string(),
            summary: Some(
                "Synthetic campaign artifact used by the meeting-prep quality profile.".to_string(),
            ),
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "taskId": task.id,
                "artifactTitle": task.title,
                "artifactKind": task.task_type,
                "artifact": "domain_eval_campaign_fixture",
            }),
            confidence: Some(0.95),
        });
    }
    let needs_approval = task.required_evidence.iter().any(|req| {
        req.required
            && matches!(
                req.evidence_type.as_str(),
                "user_decision" | "message_draft_approved"
            )
    });
    DomainEvalFixture {
        name: label.to_string(),
        description: format!("Domain eval campaign fixture for {}", task.id),
        task_id: task.id.clone(),
        label: Some(label.to_string()),
        execution_mode: execution_mode.to_string(),
        domain: Some(task.domain.clone()),
        goal: DomainEvalFixtureGoal {
            objective: Some(task.input.prompt.clone()),
            completion_criteria: task.success_criteria.first().cloned(),
            workflow_template_id: None,
            workflow_template_version: None,
            workflow_task_type: Some(task.task_type.clone()),
        },
        evidence,
        workflow: Some(DomainEvalFixtureWorkflow::default()),
        quality: Some(DomainEvalFixtureQuality {
            run: true,
            source_metadata: json!({
                "sourceType": DOMAIN_EVAL_SOURCE_CAMPAIGN,
                "taskId": task.id,
                "campaignLabel": label,
            }),
            explicit_user_approval: needs_approval,
        }),
        execution: DomainEvalFixtureExecution {
            prompt: Some(task.input.prompt.clone()),
            display_text: Some(label.to_string()),
            ..Default::default()
        },
        checks: DomainEvalFixtureChecks {
            expected_status: Some("passed".to_string()),
            min_score: Some(DEFAULT_MIN_AVERAGE_SCORE),
            expected_execution_status: (execution_mode == "agent").then(|| "completed".to_string()),
            ..Default::default()
        },
    }
}

/// Build the canonical deterministic trace fixture used by the standalone
/// capability-eval runner for one built-in domain task.
///
/// Provider configuration is deliberately absent and the execution mode is
/// fixed to `trace_fixture`; callers cannot turn this release adapter into an
/// external-model run.
pub fn deterministic_domain_eval_fixture(
    db: &SessionDB,
    task_id: &str,
    label: &str,
) -> Result<DomainEvalFixture> {
    let task = db
        .resolve_domain_eval_task(task_id)?
        .ok_or_else(|| anyhow!("domain eval task not found: {task_id}"))?;
    Ok(domain_eval_campaign_fixture(&task, "trace_fixture", label))
}

fn campaign_evidence_metadata(
    task: &DomainEvalTask,
    req: &DomainEvalEvidenceRequirement,
    index: usize,
) -> Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert("sourceType".to_string(), json!(DOMAIN_EVAL_SOURCE_CAMPAIGN));
    metadata.insert("taskId".to_string(), json!(task.id));
    metadata.insert("domain".to_string(), json!(task.domain));
    metadata.insert("requirement".to_string(), json!(req.evidence_type));
    metadata.insert("fixtureIndex".to_string(), json!(index + 1));
    // Domain quality scopes every evidence item to the artifact under review.
    // Keep the deterministic trace internally consistent by binding all of its
    // evidence (sources, claim checks, approvals, and artifact events) to the
    // same synthetic artifact.
    metadata.insert("artifactTitle".to_string(), json!(task.title));
    metadata.insert("artifactKind".to_string(), json!(task.task_type));
    for key in &req.metadata_keys {
        metadata.insert(key.clone(), json!(format!("campaign_fixture_{key}")));
    }
    match req.evidence_type.as_str() {
        "source_cited" => {
            metadata.insert(
                "url".to_string(),
                json!("https://example.invalid/domain-eval-campaign"),
            );
            metadata.insert("retrievedAt".to_string(), json!(now_rfc3339()));
            metadata.insert("publishedAt".to_string(), json!("2026-01-01"));
        }
        "data_quality_checked" => {
            metadata.insert("dataset".to_string(), json!("domain_eval_campaign_fixture"));
            metadata.insert("metric".to_string(), json!("quality_score"));
            metadata.insert("denominator".to_string(), json!("eligible records"));
            metadata.insert("sampleSize".to_string(), json!(128));
        }
        "user_decision" | "message_draft_approved" => {
            metadata.insert("approvedAt".to_string(), json!(now_rfc3339()));
            metadata.insert("reviewer".to_string(), json!("domain_eval_campaign"));
        }
        _ => {}
    }
    Value::Object(metadata)
}

async fn run_domain_eval_agent_execution(
    db: Arc<SessionDB>,
    session_id: &str,
    task: &DomainEvalTask,
    fixture: &DomainEvalFixture,
) -> Result<DomainEvalFixtureExecutionReport> {
    let execution = &fixture.execution;
    let prompt = execution
        .prompt
        .clone()
        .or_else(|| fixture.goal.objective.clone())
        .unwrap_or_else(|| task.input.prompt.clone());
    let agent_id = execution
        .agent_id
        .clone()
        .unwrap_or_else(|| DEFAULT_AGENT_ID.to_string());
    let workflow_mode = match WorkflowMode::from_str(&execution.workflow_mode) {
        Some(mode) => mode,
        None => {
            return Ok(domain_eval_agent_execution_failed(
                "agent",
                prompt,
                agent_id,
                execution.workflow_mode.clone(),
                format!(
                    "unsupported domain eval fixture workflowMode {:?}; expected off, on, or ultracode",
                    execution.workflow_mode
                ),
                None,
            ));
        }
    };
    let workflow_mode_label = workflow_mode.as_str().to_string();

    if prompt.trim().is_empty() {
        return Ok(domain_eval_agent_execution_failed(
            "agent",
            prompt,
            agent_id,
            workflow_mode_label,
            "agent execution requires a task prompt".to_string(),
            None,
        ));
    }
    if execution.providers.is_empty() || execution.model_chain.is_empty() {
        return Ok(domain_eval_agent_execution_failed(
            "agent",
            prompt,
            agent_id,
            workflow_mode_label,
            "agent execution requires providers and modelChain in the fixture".to_string(),
            None,
        ));
    }

    let _agent_admission = match ha_core::agent_lifecycle::begin_agent_run(&agent_id) {
        Ok(guard) => guard,
        Err(error) => {
            return Ok(domain_eval_agent_execution_failed(
                "agent",
                prompt,
                agent_id,
                workflow_mode_label,
                error.to_string(),
                None,
            ));
        }
    };

    db.update_session_workflow_mode(session_id, workflow_mode)?;
    let user_message_id = db
        .append_message(
            session_id,
            &NewMessage::user(&prompt).with_source(ChatSource::Eval),
        )
        .ok();
    let turn_id = uuid::Uuid::new_v4().to_string();
    db.create_chat_turn_with_id(
        &turn_id,
        session_id,
        ChatSource::Eval.as_str(),
        None,
        user_message_id,
    )?;

    let run_context = Some(ha_core::prompt_context::RunInstructionContext::new(
        ha_core::prompt_context::RunInstructionSource::Evaluation,
        "# Domain Eval Execution\n\nExecute the current evaluation task and produce auditable evidence through the normal tool and workflow contracts.",
    )?
    .with_untrusted_data(domain_eval_fixture_execution_context(
        fixture,
        task,
        execution.extra_system_context.as_deref(),
    )));
    let params = ha_core::turn_kernel::TurnRequest::new(
        session_id.to_string(),
        agent_id.clone(),
        prompt.clone(),
        db.clone(),
        execution.compact_config.clone().unwrap_or_default(),
        Arc::new(AtomicBool::new(false)),
        Arc::new(NoopEventSink),
    )
    .with_evaluation_model_chain(execution.model_chain.clone())
    .with_turn_id(turn_id.clone())
    .with_display_text(execution.display_text.clone())
    .with_run_context(run_context)
    .with_reasoning_effort(
        execution
            .reasoning_effort
            .clone()
            .or_else(|| Some("none".to_string())),
    )
    .with_plan_context_override(Some(ha_core::agent::PlanResolvedContext::off()))
    .with_denied_tools(execution.denied_tools.clone());

    let result =
        ha_core::turn_kernel::TurnKernel::submit(ha_core::turn_kernel::TurnSubmission::evaluation(
            params,
            execution.auto_approve_tools,
            execution.providers.clone(),
        ))
        .await;
    let tool_calls = domain_eval_execution_tool_calls(&db, session_id)?;
    match result {
        Ok(result) => Ok(DomainEvalFixtureExecutionReport {
            mode: "agent".to_string(),
            status: "completed".to_string(),
            prompt,
            agent_id,
            workflow_mode: workflow_mode_label,
            turn_id: Some(turn_id),
            response: Some(result.response),
            error: None,
            model_used: result.model_used,
            tool_calls,
        }),
        Err(err) => Ok(DomainEvalFixtureExecutionReport {
            mode: "agent".to_string(),
            status: "failed".to_string(),
            prompt,
            agent_id,
            workflow_mode: workflow_mode_label,
            turn_id: Some(turn_id),
            response: None,
            error: Some(err),
            model_used: None,
            tool_calls,
        }),
    }
}

fn domain_eval_agent_execution_failed(
    mode: &str,
    prompt: String,
    agent_id: String,
    workflow_mode: String,
    error: String,
    turn_id: Option<String>,
) -> DomainEvalFixtureExecutionReport {
    DomainEvalFixtureExecutionReport {
        mode: mode.to_string(),
        status: "failed".to_string(),
        prompt,
        agent_id,
        workflow_mode,
        turn_id,
        response: None,
        error: Some(error),
        model_used: None,
        tool_calls: Vec::new(),
    }
}

fn domain_eval_fixture_execution_context(
    fixture: &DomainEvalFixture,
    task: &DomainEvalTask,
    extra: Option<&str>,
) -> String {
    let required_evidence = task
        .required_evidence
        .iter()
        .map(|req| {
            format!(
                "- {}: {} (min {}, metadata: {})",
                req.evidence_type,
                req.title,
                req.min_count,
                if req.metadata_keys.is_empty() {
                    "none".to_string()
                } else {
                    req.metadata_keys.join(", ")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let success_criteria = task
        .success_criteria
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut context = format!(
        "# Domain Eval Fixture Execution\n\nFixture: {}\nTask: {} ({})\nDomain: {}\n\nUse this turn to produce real trace evidence for the domain task. Workflow Mode may be enabled for autonomous dynamic orchestration; use durable workflow/evidence tools when they make the work more observable, controllable, or recoverable.\n\n## Required Evidence\n{}\n\n## Success Criteria\n{}\n",
        fixture.name,
        task.id,
        task.task_type,
        task.domain,
        if required_evidence.is_empty() {
            "- No explicit required evidence.".to_string()
        } else {
            required_evidence
        },
        if success_criteria.is_empty() {
            "- No explicit success criteria.".to_string()
        } else {
            success_criteria
        }
    );
    if let Some(extra) = extra.and_then(non_empty) {
        context.push_str("\n## Additional Fixture Context\n");
        context.push_str(extra);
        context.push('\n');
    }
    context
}

fn domain_eval_execution_tool_calls(db: &SessionDB, session_id: &str) -> Result<Vec<String>> {
    Ok(db
        .load_session_messages(session_id)?
        .into_iter()
        .filter(|message| message.role == MessageRole::Tool)
        .filter_map(|message| message.tool_name)
        .collect())
}

fn domain_eval_fixture_checks(
    checks: &DomainEvalFixtureChecks,
    run: Option<&DomainEvalRunRecord>,
    execution: &Option<DomainEvalFixtureExecutionReport>,
) -> Vec<DomainEvalFixtureCheck> {
    let mut out = Vec::new();
    if let Some(execution) = execution {
        push_fixture_check(
            &mut out,
            "agent_execution_completed",
            execution.status == "completed",
            "completed".to_string(),
            execution.status.clone(),
            "Agent-backed domain fixture execution must complete before scorer history is written.",
        );
        if let Some(expected) = checks
            .expected_execution_status
            .as_deref()
            .and_then(non_empty)
        {
            push_fixture_check(
                &mut out,
                "expected_execution_status",
                execution.status == expected,
                expected.to_string(),
                execution.status.clone(),
                "Domain eval fixture expected a specific agent execution status.",
            );
        }
        if let Some(require_turn) = checks.require_turn {
            let has_turn = execution.turn_id.is_some();
            push_fixture_check(
                &mut out,
                "agent_turn_created",
                has_turn == require_turn,
                require_turn.to_string(),
                has_turn.to_string(),
                "Domain eval fixture expected agent mode to create a chat turn.",
            );
        }
        if let Some(min_tool_calls) = checks.min_tool_calls {
            push_fixture_check(
                &mut out,
                "min_tool_calls",
                execution.tool_calls.len() >= min_tool_calls,
                format!("tool calls >= {min_tool_calls}"),
                execution.tool_calls.len().to_string(),
                "Domain eval fixture expected the agent to call tools.",
            );
        }
        for name in &checks.expected_tool_calls {
            let called = execution.tool_calls.iter().any(|tool| tool == name);
            push_fixture_check(
                &mut out,
                &format!("tool_called:{name}"),
                called,
                "called".to_string(),
                if called { "called" } else { "missing" }.to_string(),
                "Domain eval fixture expected the agent to call this tool.",
            );
        }
        for needle in &checks.response_contains {
            let found = execution
                .response
                .as_deref()
                .is_some_and(|response| response.contains(needle));
            push_fixture_check(
                &mut out,
                "response_contains",
                found,
                needle.clone(),
                execution.response.clone().unwrap_or_default(),
                "Domain eval fixture expected the agent response to contain this text.",
            );
        }
        for needle in &checks.error_contains {
            let found = execution
                .error
                .as_deref()
                .is_some_and(|error| error.contains(needle));
            push_fixture_check(
                &mut out,
                "error_contains",
                found,
                needle.clone(),
                execution.error.clone().unwrap_or_default(),
                "Domain eval fixture expected the agent error to contain this text.",
            );
        }
    }
    let Some(run) = run else {
        if out.is_empty() {
            push_fixture_check(
                &mut out,
                "eval_run_created",
                false,
                "domain eval run created".to_string(),
                "missing".to_string(),
                "Fixture did not persist a domain eval run.",
            );
        }
        return out;
    };
    if let Some(expected) = checks.expected_status.as_deref().and_then(non_empty) {
        push_fixture_check(
            &mut out,
            "expected_status",
            run.status == expected,
            expected.to_string(),
            run.status.clone(),
            "Domain eval fixture expected a specific scorer status.",
        );
    } else {
        push_fixture_check(
            &mut out,
            "eval_status_passed",
            run.status == "passed",
            "passed".to_string(),
            run.status.clone(),
            "Domain eval fixture defaults to requiring the scorer to pass unless expectedStatus overrides it.",
        );
    }
    if let Some(min_score) = checks.min_score {
        push_fixture_check(
            &mut out,
            "min_score",
            run.score >= min_score,
            format!("score >= {min_score:.2}"),
            format!("{:.2}", run.score),
            "Domain eval fixture expected a minimum scorer confidence.",
        );
    }
    for name in &checks.expected_passed_checks {
        let status = run
            .report
            .checks
            .iter()
            .find(|check| check.name == *name || check.category == *name)
            .map(|check| check.status.as_str())
            .unwrap_or("missing");
        push_fixture_check(
            &mut out,
            &format!("check_passed:{name}"),
            status == "passed",
            "passed".to_string(),
            status.to_string(),
            "Domain eval fixture expected this scorer check to pass.",
        );
    }
    for name in &checks.expected_failed_checks {
        let status = run
            .report
            .checks
            .iter()
            .find(|check| check.name == *name || check.category == *name)
            .map(|check| check.status.as_str())
            .unwrap_or("missing");
        push_fixture_check(
            &mut out,
            &format!("check_failed:{name}"),
            status == "failed",
            "failed".to_string(),
            status.to_string(),
            "Domain eval fixture expected this scorer check to fail.",
        );
    }
    if out.is_empty() {
        push_fixture_check(
            &mut out,
            "eval_run_created",
            true,
            "domain eval run created".to_string(),
            run.id.clone(),
            "Fixture executed the trace and persisted a domain eval run.",
        );
    }
    out
}

fn push_fixture_check(
    out: &mut Vec<DomainEvalFixtureCheck>,
    name: &str,
    passed: bool,
    expected: String,
    actual: String,
    detail: &str,
) {
    out.push(DomainEvalFixtureCheck {
        name: name.to_string(),
        status: if passed { "passed" } else { "failed" }.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn domain_quality_gate_thresholds(input: &DomainQualityGateInput) -> DomainQualityGateThresholds {
    DomainQualityGateThresholds {
        min_eval_runs: input
            .min_eval_runs
            .unwrap_or(DEFAULT_MIN_EVAL_RUNS)
            .clamp(1, 100),
        min_pass_rate: input
            .min_pass_rate
            .unwrap_or(DEFAULT_MIN_PASS_RATE)
            .clamp(0.0, 1.0),
        min_average_score: input
            .min_average_score
            .unwrap_or(DEFAULT_MIN_AVERAGE_SCORE)
            .clamp(0.0, 1.0),
        min_quality_runs: input
            .min_quality_runs
            .unwrap_or(DEFAULT_MIN_QUALITY_RUNS)
            .clamp(1, 100),
        max_blocked_quality_runs: input
            .max_blocked_quality_runs
            .unwrap_or(DEFAULT_MAX_BLOCKED_QUALITY_RUNS)
            .min(100),
        min_domain_coverage: input
            .min_domain_coverage
            .unwrap_or(DEFAULT_MIN_DOMAIN_COVERAGE)
            .clamp(1, 5),
        require_approval_safety: input.require_approval_safety,
    }
}

fn domain_readiness_gate_thresholds(
    input: &DomainReadinessGateInput,
) -> DomainReadinessGateThresholds {
    DomainReadinessGateThresholds {
        window_days: input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS),
        min_eval_runs: input
            .min_eval_runs
            .unwrap_or(DEFAULT_MIN_EVAL_RUNS)
            .clamp(1, 100),
        min_pass_rate: input
            .min_pass_rate
            .unwrap_or(DEFAULT_MIN_PASS_RATE)
            .clamp(0.0, 1.0),
        min_average_score: input
            .min_average_score
            .unwrap_or(DEFAULT_MIN_AVERAGE_SCORE)
            .clamp(0.0, 1.0),
        min_quality_runs: input
            .min_quality_runs
            .unwrap_or(DEFAULT_MIN_QUALITY_RUNS)
            .clamp(1, 100),
        max_blocked_quality_runs: input
            .max_blocked_quality_runs
            .unwrap_or(DEFAULT_MAX_BLOCKED_QUALITY_RUNS)
            .min(100),
        min_domain_coverage: input
            .min_domain_coverage
            .unwrap_or(DEFAULT_MIN_DOMAIN_COVERAGE)
            .clamp(1, 5),
        min_campaign_items: input
            .min_campaign_items
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MIN_CAMPAIGN_ITEMS)
            .clamp(1, 500),
        min_leaderboard_rows: input
            .min_leaderboard_rows
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MIN_LEADERBOARD_ROWS)
            .clamp(1, MAX_DOMAIN_EVAL_CAMPAIGN_LIMIT),
        max_failed_campaign_items: input
            .max_failed_campaign_items
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MAX_FAILED_CAMPAIGN_ITEMS)
            .min(500),
        max_open_learning_proposals: input
            .max_open_learning_proposals
            .unwrap_or(DEFAULT_DOMAIN_READINESS_MAX_OPEN_LEARNING_PROPOSALS)
            .min(500),
        require_approval_safety: input.require_approval_safety,
        include_synthetic: input.include_synthetic,
    }
}

fn domain_operational_gate_thresholds(
    input: &DomainOperationalGateInput,
) -> DomainOperationalGateThresholds {
    DomainOperationalGateThresholds {
        window_days: input
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, MAX_WINDOW_DAYS),
        min_workflow_runs: input
            .min_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MIN_WORKFLOW_RUNS)
            .clamp(1, 500),
        max_failed_workflow_runs: input
            .max_failed_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_WORKFLOW_RUNS)
            .min(500),
        max_blocked_workflow_runs: input
            .max_blocked_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_BLOCKED_WORKFLOW_RUNS)
            .min(500),
        max_cancelled_workflow_runs: input
            .max_cancelled_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_CANCELLED_WORKFLOW_RUNS)
            .min(500),
        max_active_workflow_runs: input
            .max_active_workflow_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_WORKFLOW_RUNS)
            .min(500),
        min_loop_runs: input
            .min_loop_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MIN_LOOP_RUNS)
            .min(500),
        max_failed_loop_runs: input
            .max_failed_loop_runs
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_LOOP_RUNS)
            .min(500),
        max_active_campaigns: input
            .max_active_campaigns
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_ACTIVE_CAMPAIGNS)
            .min(100),
        max_failed_campaign_items: input
            .max_failed_campaign_items
            .unwrap_or(DEFAULT_DOMAIN_OPERATIONAL_MAX_FAILED_CAMPAIGN_ITEMS)
            .min(500),
    }
}

fn push_gate_check(
    checks: &mut Vec<DomainQualityGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: String,
    actual: String,
    detail: &str,
) {
    checks.push(DomainQualityGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn push_readiness_check(
    checks: &mut Vec<DomainReadinessGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: String,
    actual: String,
    detail: &str,
) {
    checks.push(DomainReadinessGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn push_operational_check(
    checks: &mut Vec<DomainOperationalGateCheck>,
    name: &str,
    status: &str,
    severity: &str,
    expected: String,
    actual: String,
    detail: &str,
) {
    checks.push(DomainOperationalGateCheck {
        name: name.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        expected,
        actual,
        detail: detail.to_string(),
    });
}

fn gate_status(checks: &[DomainQualityGateCheck]) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else {
        "passed".to_string()
    }
}

fn readiness_status(checks: &[DomainReadinessGateCheck]) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else {
        "passed".to_string()
    }
}

fn operational_status(checks: &[DomainOperationalGateCheck]) -> String {
    if checks.iter().any(|check| check.status == "failed") {
        "failed".to_string()
    } else if checks
        .iter()
        .any(|check| check.status == "insufficient_data")
    {
        "insufficient_data".to_string()
    } else {
        "passed".to_string()
    }
}

fn domain_readiness_recommendations(checks: &[DomainReadinessGateCheck]) -> Vec<String> {
    let mut recommendations = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let recommendation = match check.name.as_str() {
            "domain_quality_gate" => {
                "Run or review live Domain Quality / Domain Eval evidence for the current scope."
            }
            "campaign_sample" => {
                "Run a trace or external Domain Eval Campaign so the domain workflow has replayable evidence."
            }
            "campaign_completion" => {
                "Wait for active Domain Eval Campaigns to finish, or cancel and retry them if they are stuck."
            }
            "campaign_leaderboard" => {
                "Run enough comparable campaign items to populate the Domain model leaderboard."
            }
            "campaign_failures" => {
                "Retry failed/cancelled/interrupted domain campaign items before treating the scope as ready."
            }
            "learning_closure" => {
                "Generate and resolve Domain Campaign Learning drafts for failed campaign evidence."
            }
            _ => "Resolve the failing Domain Readiness check.",
        };
        if !recommendations.iter().any(|item| item == recommendation) {
            recommendations.push(recommendation.to_string());
        }
    }
    recommendations
}

fn domain_operational_recommendations(checks: &[DomainOperationalGateCheck]) -> Vec<String> {
    let mut recommendations = Vec::new();
    for check in checks.iter().filter(|check| check.status != "passed") {
        let recommendation = match check.name.as_str() {
            "workflow_sample" => {
                "Run at least one durable domain workflow so operational readiness has real control-plane evidence."
            }
            "workflow_failures" => {
                "Repair or retry failed/blocked/cancelled workflow runs before treating the scope as stable."
            }
            "workflow_active_drain" => {
                "Wait for active workflows to finish, approve waiting runs, or explicitly pause/cancel stale work."
            }
            "loop_sample" => {
                "Run a loop tick when recurring behavior matters for this scope."
            }
            "loop_failures" => {
                "Inspect failed loop ticks and update the loop strategy or workflow draft before continuing unattended."
            }
            "campaign_active_drain" => {
                "Wait for active domain campaigns to finish, or cancel and retry them if they are stuck."
            }
            "campaign_failures" => {
                "Retry failed/cancelled/interrupted campaign items or convert them into learning proposals."
            }
            _ => "Resolve the failing Domain Operational check.",
        };
        if !recommendations.iter().any(|item| item == recommendation) {
            recommendations.push(recommendation.to_string());
        }
    }
    recommendations
}

fn track_soak_sample_day(sample_days: &mut BTreeSet<String>, timestamp: &str) {
    if let Some(day) = rfc3339_day_key(timestamp) {
        sample_days.insert(day);
    }
}

fn track_soak_activity(
    summary: &mut DomainSoakReportSummary,
    sample_days: &mut BTreeSet<String>,
    timestamp: &str,
) {
    track_soak_sample_day(sample_days, timestamp);
    max_timestamp(&mut summary.latest_activity_at, timestamp.to_string());
}

fn average_secs(values: &[i64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum: i64 = values.iter().sum();
    let average = sum as f64 / values.len() as f64;
    Some((average * 10.0).round() / 10.0)
}

#[allow(clippy::too_many_arguments)]
fn push_soak_incident(
    incidents: &mut Vec<DomainSoakIncident>,
    source: &str,
    id: &str,
    title: &str,
    status: &str,
    severity: &str,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_secs: Option<i64>,
    reason: String,
    recommendation: String,
) {
    incidents.push(DomainSoakIncident {
        source: source.to_string(),
        id: id.to_string(),
        title: title.to_string(),
        status: status.to_string(),
        severity: severity.to_string(),
        started_at,
        finished_at,
        duration_secs,
        reason,
        recommendation,
    });
}

fn incident_rank(severity: &str) -> usize {
    match severity {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn json_string_value(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn domain_soak_recommendations(
    summary: &DomainSoakReportSummary,
    incidents: &[DomainSoakIncident],
    operational_gate: &DomainOperationalGateReport,
) -> Vec<String> {
    let mut recommendations = Vec::new();

    if summary.total_records == 0 {
        push_unique_soak_recommendation(&mut recommendations, "Run at least one real workflow, loop tick, campaign, or connector E2E action in this scope before trusting soak readiness.");
    }
    if summary.critical_incidents > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Resolve critical soak incidents first: repair failed workflows, retry failed campaign items, and re-run the report.");
    }
    if summary.warning_incidents > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Drain active or approval-waiting work so the report reflects completed long-running behavior.");
    }
    if summary.open_approval_waits > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Resolve open workflow approvals: approve, deny, pause, or cancel them before trusting unattended long-run stability.");
    }
    if summary.workflow_control_intervention_events > 1 {
        push_unique_soak_recommendation(&mut recommendations, "Review repeated workflow control interventions and adjust the workflow plan, approval gates, or loop strategy before widening unattended usage.");
    }
    if summary.connector_execution_evidence > 0 && summary.connector_verification_evidence == 0 {
        push_unique_soak_recommendation(&mut recommendations, "Finish connector verification evidence for real external actions instead of stopping at draft or execution records.");
    }
    if summary.workflow_budget_exhausted_events > 0 {
        push_unique_soak_recommendation(&mut recommendations, "Review workflow output-token budget exhaustion: shrink fan-out, summarize intermediate outputs, or explicitly raise the budget before widening unattended usage.");
    }
    if summary.total_records > 0 && summary.sample_days < summary.required_sample_days {
        push_unique_soak_recommendation(&mut recommendations, "Collect long-running samples on at least two distinct days before trusting unattended cross-day stability.");
    }
    if summary
        .latest_activity_age_secs
        .is_some_and(|age| age > 24 * 60 * 60)
    {
        push_unique_soak_recommendation(&mut recommendations, "Collect a fresh workflow, loop, campaign, or connector E2E sample before trusting this soak window for current unattended behavior.");
    }
    for recommendation in &operational_gate.recommended_next_steps {
        push_unique_soak_recommendation(&mut recommendations, recommendation);
    }
    for incident in incidents.iter().take(3) {
        push_unique_soak_recommendation(&mut recommendations, &incident.recommendation);
    }
    if recommendations.is_empty() {
        push_unique_soak_recommendation(&mut recommendations, "Keep collecting cross-day samples and compare this report with the next soak window before widening unattended usage.");
    }
    recommendations
}

fn push_unique_soak_recommendation(recommendations: &mut Vec<String>, item: &str) {
    if !recommendations.iter().any(|existing| existing == item) {
        recommendations.push(item.to_string());
    }
}

fn format_output_token_budget(summary: &DomainSoakReportSummary) -> String {
    match (
        summary.max_workflow_output_tokens_spent,
        summary.max_workflow_output_token_budget,
    ) {
        (Some(spent), Some(limit)) if limit > 0 => format!("{spent}/{limit}"),
        (Some(spent), _) => spent.to_string(),
        _ => "n/a".to_string(),
    }
}

fn render_domain_soak_markdown(
    scope: &DomainGateScope,
    generated_at: &str,
    status: &str,
    summary: &DomainSoakReportSummary,
    incidents: &[DomainSoakIncident],
    recommendations: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# Domain Soak Report\n\n");
    out.push_str(&format!(
        "- Status: `{}`\n- Scope: `{}`\n- Window: {} day(s), since `{}`\n- Generated at: `{}`\n\n",
        status, scope.scope, scope.window_days, scope.since, generated_at
    ));
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- Workflows: {} total, {} completed, {} failed, {} blocked, {} cancelled, {} active\n",
        summary.workflow_runs,
        summary.completed_workflow_runs,
        summary.failed_workflow_runs,
        summary.blocked_workflow_runs,
        summary.cancelled_workflow_runs,
        summary.active_workflow_runs
    ));
    out.push_str(&format!(
        "- Loops: {} total, {} succeeded, {} failed, {} active\n",
        summary.loop_runs,
        summary.succeeded_loop_runs,
        summary.failed_loop_runs,
        summary.active_loop_runs
    ));
    out.push_str(&format!(
        "- Campaigns: {} campaign(s), {} item(s), {} passed, {} failed, {} cancelled, {} interrupted, {} retried\n",
        summary.campaigns,
        summary.campaign_items,
        summary.passed_campaign_items,
        summary.failed_campaign_items,
        summary.cancelled_campaign_items,
        summary.interrupted_campaign_items,
        summary.retried_campaign_items
    ));
    out.push_str(&format!(
        "- Connector E2E evidence: {} total, {} execution, {} verification\n",
        summary.connector_e2e_evidence,
        summary.connector_execution_evidence,
        summary.connector_verification_evidence
    ));
    out.push_str(&format!(
        "- Control events: {} owner intervention(s); {} approval request(s), {} approval decision(s), {} open approval wait(s), {} pause, {} resume, {} cancel, {} recovery; max closed/open approval wait: {}/{}\n",
        summary.workflow_control_intervention_events,
        summary.approval_request_events,
        summary.approval_decision_events,
        summary.open_approval_waits,
        summary.pause_events,
        summary.resume_events,
        summary.cancel_events,
        summary.recovery_events,
        summary
            .max_approval_wait_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "n/a".to_string()),
        summary
            .max_open_approval_wait_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    out.push_str(&format!(
        "- Budget events: {} output-token sample(s), {} exhausted; max output tokens: {}\n",
        summary.workflow_budget_usage_events,
        summary.workflow_budget_exhausted_events,
        format_output_token_budget(summary)
    ));
    out.push_str(&format!(
        "- Freshness: latest activity {}; age {}\n",
        summary.latest_activity_at.as_deref().unwrap_or("n/a"),
        summary
            .latest_activity_age_secs
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    out.push_str(&format!(
        "- Sample days: {}/{} distinct day(s)\n",
        summary.sample_days, summary.required_sample_days
    ));
    out.push_str(&format!(
        "- Incidents: {} total, {} critical, {} warning\n\n",
        summary.incidents, summary.critical_incidents, summary.warning_incidents
    ));

    out.push_str("## Incidents\n\n");
    if incidents.is_empty() {
        out.push_str("- None in the selected window.\n\n");
    } else {
        for incident in incidents {
            out.push_str(&format!(
                "- [{}] `{}` {} `{}`: {}. Next: {}\n",
                incident.severity,
                incident.source,
                incident.title,
                incident.status,
                incident.reason,
                incident.recommendation
            ));
        }
        out.push('\n');
    }

    out.push_str("## Recommended Next Steps\n\n");
    for recommendation in recommendations {
        out.push_str(&format!("- {}\n", recommendation));
    }
    out
}

fn fixture_source_type(execution_mode: &str) -> String {
    match execution_mode {
        "trace_fixture" => DOMAIN_EVAL_SOURCE_FIXTURE_TRACE,
        "agent" => DOMAIN_EVAL_SOURCE_FIXTURE_AGENT,
        _ => DOMAIN_EVAL_SOURCE_FIXTURE_UNSUPPORTED,
    }
    .to_string()
}

fn fixture_quality_source_metadata(
    mut metadata: Value,
    source_type: &str,
    fixture_name: &str,
    execution_mode: &str,
) -> Value {
    if !metadata.is_object() {
        metadata = json!({ "payload": metadata });
    }
    if let Some(map) = metadata.as_object_mut() {
        map.insert("sourceType".to_string(), json!(source_type));
        map.insert("fixtureName".to_string(), json!(fixture_name));
        map.insert("executionMode".to_string(), json!(execution_mode));
    }
    metadata
}

#[cfg(all(test, feature = "eval-internal-tests"))]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use ha_core::domain_quality::RunDomainQualityInput;
    use ha_core::domain_workflow::RecordDomainEvidenceInput;
    use ha_core::provider::{ApiType, ModelConfig, ProviderConfig};
    use ha_core::workflow::WorkflowRunState;
    use rusqlite::params;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn model_config(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            name: id.to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        }
    }

    fn sse_json_string(value: &str) -> String {
        serde_json::to_string(value).expect("serialize SSE JSON string")
    }

    fn responses_sse_text(text: &str) -> String {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":{}}}\n\n\
             data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\n",
            sse_json_string(text)
        )
    }

    fn mock_responses_provider(
        base_url: String,
        provider_id: &str,
        model_id: &str,
    ) -> ProviderConfig {
        let mut provider = ProviderConfig::new(
            "Domain Eval Mock Responses".to_string(),
            ApiType::OpenaiResponses,
            base_url,
            "test-key".to_string(),
        );
        provider.id = provider_id.to_string();
        provider.models.push(model_config(model_id));
        provider
    }

    fn test_db() -> (tempfile::TempDir, SessionDB) {
        // 迁出前这些测试跑在 ha-core 里，`transition_workflow_run` 会**直接**
        // 调 `ensure_coding_workflow_retro_for_run` 记一条终态复盘。现在那条路
        // 走 `improve_hooks`，不 wire 就恒返 `Ok(None)`——不装配等于让这批测试
        // 悄悄跑在一条「永远不记复盘」的路径上，与生产不同。`wire()` 幂等。
        crate::wire();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db"))
            .expect("session db");
        ensure_channel_conversations_table(&db);
        (dir, db)
    }

    fn ensure_channel_conversations_table(db: &SessionDB) {
        db.with_conn_for_test(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    thread_id TEXT,
                    session_id TEXT NOT NULL,
                    sender_id TEXT,
                    sender_name TEXT,
                    chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    attached_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );",
            )
            .expect("create channel conversations table");
            Ok(())
        })
        .unwrap();
    }

    fn record_evidence(
        db: &SessionDB,
        session_id: &str,
        domain: &str,
        evidence_type: &str,
        title: &str,
        source_metadata: Value,
    ) {
        db.record_domain_evidence(RecordDomainEvidenceInput {
            session_id: Some(session_id.to_string()),
            domain: domain.to_string(),
            evidence_type: evidence_type.to_string(),
            title: title.to_string(),
            source_metadata,
            confidence: Some(0.95),
            ..Default::default()
        })
        .unwrap();
    }

    #[tokio::test]
    async fn domain_eval_fixture_runner_scores_trace_fixture() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let report = run_domain_eval_fixture(
            db.clone(),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "research-trace-fixture".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    goal: DomainEvalFixtureGoal {
                        objective: Some("Prepare a sourced research brief.".to_string()),
                        completion_criteria: Some(
                            "Sources, claims, citation audit and workflow trace are present."
                                .to_string(),
                        ),
                        ..Default::default()
                    },
                    evidence: vec![
                        DomainEvalFixtureEvidence {
                            evidence_type: "source_cited".to_string(),
                            title: "Source A".to_string(),
                            source_metadata: json!({"uri": "https://example.com/a", "retrievedAt": "2026-07-04"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "source_cited".to_string(),
                            title: "Source B".to_string(),
                            source_metadata: json!({"uri": "https://example.com/b", "retrievedAt": "2026-07-04"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "source_cited".to_string(),
                            title: "Source C".to_string(),
                            source_metadata: json!({"uri": "https://example.com/c", "publishedAt": "2026-07-03"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "claim_checked".to_string(),
                            title: "Claim A checked".to_string(),
                            source_metadata: json!({"claim": "claim A", "verdict": "supported"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "claim_checked".to_string(),
                            title: "Claim B checked".to_string(),
                            source_metadata: json!({"claim": "claim B", "verdict": "supported"}),
                            ..Default::default()
                        },
                        DomainEvalFixtureEvidence {
                            evidence_type: "citation_audited".to_string(),
                            title: "Citation audit".to_string(),
                            source_metadata: json!({"coverage": "all key claims"}),
                            ..Default::default()
                        },
                    ],
                    workflow: Some(DomainEvalFixtureWorkflow::default()),
                    quality: Some(DomainEvalFixtureQuality::default()),
                    checks: DomainEvalFixtureChecks {
                        expected_status: Some("passed".to_string()),
                        min_score: Some(0.8),
                        expected_passed_checks: vec![
                            "evidence_completeness".to_string(),
                            "citation_quality".to_string(),
                            "workflow_trace".to_string(),
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(report.passed, "{report:?}");
        assert_eq!(report.status, "passed");
        assert!(report.eval_run.is_some());
        assert!(report.quality_run_id.is_some());
        assert!(report.workflow_run_id.is_some());
        assert!(report.fixture_run_id.is_some());
        assert_eq!(report.source_type, DOMAIN_EVAL_SOURCE_FIXTURE_TRACE);
        assert_eq!(
            report.eval_run.as_ref().unwrap().source_type,
            DOMAIN_EVAL_SOURCE_FIXTURE_TRACE
        );
        assert_eq!(
            db.get_session(&report.session_id).unwrap().unwrap().kind,
            SessionKind::EvalFixture
        );
        assert!(db
            .list_domain_eval_runs(ListDomainEvalRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            db.list_domain_eval_runs(ListDomainEvalRunsInput {
                source_type: Some("fixture".to_string()),
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            db.list_domain_eval_fixture_runs(ListDomainEvalFixtureRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        let gate = evaluate_domain_quality_gate(
            &db,
            DomainQualityGateInput {
                window_days: Some(1),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_domain_coverage: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(gate.status, "insufficient_data");
        assert_eq!(gate.summary.eval_runs, 0);
        assert_eq!(gate.summary.quality_runs, 0);
        let synthetic_gate = evaluate_domain_quality_gate(
            &db,
            DomainQualityGateInput {
                window_days: Some(1),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_domain_coverage: Some(1),
                include_synthetic: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(synthetic_gate.status, "passed");
        assert_eq!(synthetic_gate.summary.eval_runs, 1);
        assert_eq!(synthetic_gate.summary.quality_runs, 1);
    }

    #[tokio::test]
    async fn domain_eval_campaign_runs_cancelled_trace_item_on_retry() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("domain trace campaign test".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                models: Vec::new(),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaign.status, "queued");
        assert_eq!(campaign.items.len(), 1);
        assert_eq!(campaign.items[0].execution_mode, "trace_fixture");

        let cancelled = db
            .cancel_domain_eval_campaign(&campaign.id)
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.status, "cancel_requested");
        assert_eq!(cancelled.items[0].status, "cancelled");

        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id.clone(),
                providers: Vec::new(),
                retry_failed_only: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "passed", "{completed:?}");
        assert_eq!(completed.summary.total_items, 1);
        assert_eq!(completed.summary.passed_items, 1);
        assert_eq!(completed.summary.failed_items, 0);
        assert_eq!(completed.summary.eval_runs, 1);
        assert!(completed.summary.average_score.unwrap_or_default() >= DEFAULT_MIN_AVERAGE_SCORE);
        assert!(completed.items[0].fixture_run_id.is_some());
        assert!(completed.items[0].eval_run_id.is_some());
        assert!(completed.items[0].score.unwrap_or_default() >= DEFAULT_MIN_AVERAGE_SCORE);

        assert!(db
            .list_domain_eval_runs(ListDomainEvalRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .is_empty());
        assert_eq!(
            db.list_domain_eval_runs(ListDomainEvalRunsInput {
                source_type: Some("fixture".to_string()),
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );
        let campaigns = db
            .list_domain_eval_campaigns(ListDomainEvalCampaignsInput {
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaigns.len(), 1);
        assert_eq!(campaigns[0].id, completed.id);

        let leaderboard = db
            .get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
                window_days: Some(1),
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(leaderboard.status, "passed");
        assert_eq!(leaderboard.rows.len(), 1);
        assert_eq!(leaderboard.rows[0].rank, 1);
        assert_eq!(leaderboard.rows[0].items, 1);
        assert_eq!(leaderboard.rows[0].passed_items, 1);
        assert!(leaderboard.rows[0].average_score.unwrap_or_default() >= DEFAULT_MIN_AVERAGE_SCORE);
        assert_eq!(leaderboard.rows[0].evidence.len(), 1);
    }

    #[test]
    fn domain_operational_gate_passes_with_completed_workflow() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("operational-gate-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();

        let report = evaluate_domain_operational_gate(
            &db,
            DomainOperationalGateInput {
                session_id: Some(session.id),
                domain: Some("research".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.status, "passed", "{report:?}");
        assert_eq!(report.summary.workflow_runs, 1);
        assert_eq!(report.summary.completed_workflow_runs, 1);
        assert_eq!(report.summary.failed_workflow_runs, 0);
        assert!(report.blockers.is_empty());
    }

    #[test]
    fn domain_operational_gate_blocks_failed_workflow_and_campaign_item() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("operational-gate-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Failed, Some("validation failed"))
            .unwrap();
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                session_id: Some(session.id.clone()),
                domain: Some("research".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        db.cancel_domain_eval_campaign(&campaign.id).unwrap();

        let report = evaluate_domain_operational_gate(
            &db,
            DomainOperationalGateInput {
                session_id: Some(session.id),
                domain: Some("research".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(0),
                max_active_campaigns: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.status, "failed", "{report:?}");
        assert_eq!(report.summary.failed_workflow_runs, 1);
        assert_eq!(report.summary.cancelled_campaign_items, 1);
        assert!(report
            .blockers
            .iter()
            .any(|item| item == "workflow_failures"));
        assert!(report
            .blockers
            .iter()
            .any(|item| item == "campaign_failures"));
    }

    #[test]
    fn domain_operational_gate_tracks_active_work_age() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("operational-gate-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE workflow_runs
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-120 seconds')
                      WHERE id = ?1",
                    params![run.id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }

        let report = evaluate_domain_operational_gate(
            &db,
            DomainOperationalGateInput {
                session_id: Some(session.id),
                domain: Some("research".to_string()),
                window_days: Some(1),
                min_workflow_runs: Some(1),
                min_loop_runs: Some(0),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.active_workflow_runs, 1);
        let age = report.summary.max_active_work_age_secs.unwrap();
        assert!(
            (115..=125).contains(&age),
            "unexpected active work age: {age}"
        );
        assert!(report
            .blockers
            .iter()
            .any(|item| item == "workflow_active_drain"));
    }

    #[test]
    fn domain_soak_report_passes_with_drained_history() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();

        let now = Utc::now();
        let loop_started = (now - Duration::minutes(5)).to_rfc3339();
        let loop_finished = (now - Duration::minutes(4)).to_rfc3339();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "INSERT INTO loop_schedules (
                        id, session_id, cron_job_id, prompt, trigger_kind, trigger_spec_json,
                        execution_strategy, state, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        "loop_soak_pass",
                        session.id,
                        "cron_soak_pass",
                        "Keep checking the research brief",
                        "interval",
                        "{}",
                        "continue",
                        "completed",
                        loop_started,
                        loop_finished,
                    ],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO loop_runs (
                        id, loop_id, cron_job_id, session_id, seq, state, trigger_reason,
                        result_summary, trace_json, started_at, finished_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        "lrun_soak_pass",
                        "loop_soak_pass",
                        "cron_soak_pass",
                        session.id,
                        1,
                        "succeeded",
                        "interval trigger from test",
                        "loop drained",
                        "{}",
                        loop_started,
                        loop_finished,
                    ],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }

        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                session_id: Some(session.id.clone()),
                domain: Some("research".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        let item_started = (now - Duration::minutes(3)).to_rfc3339();
        let item_finished = (now - Duration::minutes(2)).to_rfc3339();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE domain_eval_campaigns
                     SET status = 'passed', started_at = ?1, finished_at = ?2, updated_at = ?2
                     WHERE id = ?3",
                    rusqlite::params![item_started, item_finished, campaign.id],
                )
                .unwrap();
                conn.execute(
                    "UPDATE domain_eval_campaign_items
                     SET status = 'passed', attempt = 1, score = 1.0, total_checks = 1,
                         passed_checks = 1, failed_checks = 0, started_at = ?1,
                         finished_at = ?2, updated_at = ?2
                     WHERE campaign_id = ?3",
                    rusqlite::params![item_started, item_finished, campaign.id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_executed",
            "Connector action executed",
            json!({"connector": "gmail", "action": "draft"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_verified",
            "Connector action verified",
            json!({"connector": "gmail", "verified": true}),
        );

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "passed", "{report:?}");
        assert_eq!(report.summary.workflow_runs, 1);
        assert_eq!(report.summary.loop_runs, 1);
        assert_eq!(report.summary.campaign_items, 1);
        assert_eq!(report.summary.connector_e2e_evidence, 2);
        assert_eq!(report.summary.required_sample_days, 1);
        assert!(
            report.summary.sample_days >= report.summary.required_sample_days,
            "{report:?}"
        );
        assert_eq!(report.summary.incidents, 0);
        assert!(report.summary.latest_activity_at.is_some());
        assert!(report
            .summary
            .latest_activity_age_secs
            .is_some_and(|age| age <= 10));
        assert!(report.markdown.contains("# Domain Soak Report"));
        assert!(report.markdown.contains("- Freshness: latest activity"));
        assert!(report.markdown.contains(&format!(
            "- Sample days: {}/{} distinct day(s)",
            report.summary.sample_days, report.summary.required_sample_days
        )));
        assert!(report
            .timeline
            .iter()
            .any(|item| item.source == "campaign_item"));
    }

    #[test]
    fn domain_soak_report_requires_cross_day_samples_for_multi_day_window() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(7),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.sample_days, 1);
        assert_eq!(report.summary.required_sample_days, 2);
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("two distinct days")));
        assert!(report
            .markdown
            .contains("- Sample days: 1/2 distinct day(s)"));
    }

    #[test]
    fn domain_soak_report_requires_connector_post_action_verification() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_executed",
            "Connector action executed",
            json!({"connector": "gmail", "action": "draft"}),
        );

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.connector_execution_evidence, 1);
        assert_eq!(report.summary.connector_verification_evidence, 0);
        assert_eq!(report.summary.warning_incidents, 1);
        assert!(report.incidents.iter().any(|incident| {
            incident.source == "connector_e2e"
                && incident.id == "connector_verification_missing"
                && incident.status == "unverified"
        }));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("Finish connector verification evidence")));
        assert!(report
            .markdown
            .contains("Connector action verification missing"));
    }

    #[test]
    fn domain_soak_report_passes_with_cross_day_fresh_samples() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE workflow_runs
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days'),
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes'),
                            completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes')
                      WHERE id = ?1",
                    params![run.id],
                )
                .unwrap();
                conn.execute(
                    "UPDATE workflow_events
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+1 minutes')
                      WHERE run_id = ?1",
                    params![run.id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_executed",
            "Connector action executed",
            json!({"connector": "gmail", "action": "draft"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "connector_action_verified",
            "Connector action verified",
            json!({"connector": "gmail", "verified": true}),
        );

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(3),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "passed", "{report:?}");
        assert_eq!(report.summary.sample_days, 2);
        assert_eq!(report.summary.required_sample_days, 2);
        assert_eq!(report.summary.connector_verification_evidence, 1);
        assert!(report
            .summary
            .latest_activity_age_secs
            .is_some_and(|age| age <= 10));
    }

    #[test]
    fn domain_soak_report_recommends_fresh_sample_for_stale_history() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Completed, None)
            .unwrap();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE workflow_runs
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days'),
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes'),
                            completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+2 minutes')
                      WHERE id = ?1",
                    params![run.id],
                )
                .unwrap();
                conn.execute(
                    "UPDATE workflow_events
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days', '+1 minutes')
                      WHERE run_id = ?1",
                    params![run.id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(3),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "insufficient_data", "{report:?}");
        assert_eq!(report.summary.total_records, 1);
        assert_eq!(report.summary.sample_days, 1);
        assert_eq!(report.summary.required_sample_days, 2);
        assert!(report
            .summary
            .latest_activity_age_secs
            .is_some_and(|age| age > 24 * 60 * 60));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("fresh workflow")));
    }

    #[test]
    fn domain_soak_report_flags_failed_workflow_and_active_campaign() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, None)
            .unwrap();
        db.transition_workflow_run(&run.id, WorkflowRunState::Failed, Some("tool failed"))
            .unwrap();
        db.create_domain_eval_campaign(CreateDomainEvalCampaignInput {
            session_id: Some(session.id.clone()),
            domain: Some("research".to_string()),
            task_ids: vec!["research-source-backed-brief".to_string()],
            max_tasks: Some(1),
            execution_mode: Some("trace_fixture".to_string()),
            ..Default::default()
        })
        .unwrap();

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.status, "failed", "{report:?}");
        assert_eq!(report.summary.failed_workflow_runs, 1);
        assert_eq!(report.summary.active_campaigns, 1);
        assert!(report.summary.critical_incidents >= 1);
        assert!(report
            .incidents
            .iter()
            .any(|incident| incident.source == "workflow" && incident.status == "failed"));
        assert!(report
            .incidents
            .iter()
            .any(|incident| incident.source == "campaign_item" && incident.status == "queued"));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("critical soak incidents")));
    }

    #[test]
    fn domain_soak_report_tracks_approval_wait_and_recovery_events() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(
            &run.id,
            WorkflowRunState::AwaitingApproval,
            Some("permission_preview"),
        )
        .unwrap();
        db.approve_workflow_run(&run.id).unwrap();
        db.pause_workflow_run(&run.id).unwrap();
        db.resume_workflow_run(&run.id).unwrap();
        db.claim_workflow_run_for_recovery(&run.id, "test-owner")
            .unwrap();
        db.append_workflow_event(
            &run.id,
            "budget_usage",
            json!({
                "api": "waitAll",
                "spentOutputTokens": 6,
                "maxOutputTokens": 10,
                "exhausted": false,
            }),
        )
        .unwrap();
        db.append_workflow_event(
            &run.id,
            "budget_usage",
            json!({
                "api": "spawnAgent",
                "spentOutputTokens": 10,
                "maxOutputTokens": 10,
                "exhausted": true,
            }),
        )
        .unwrap();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE workflow_events
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-120 seconds')
                      WHERE run_id = ?1
                        AND type = 'run_state_changed'
                        AND payload_json LIKE '%\"to\":\"awaiting_approval\"%'",
                    params![run.id],
                )
                .unwrap();
                conn.execute(
                    "UPDATE workflow_events
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 seconds')
                      WHERE run_id = ?1
                        AND type = 'run_state_changed'
                        AND payload_json LIKE '%\"from\":\"awaiting_approval\"%'",
                    params![run.id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.summary.approval_request_events, 1);
        assert_eq!(report.summary.approval_decision_events, 1);
        assert_eq!(report.summary.max_approval_wait_secs, Some(90));
        assert_eq!(report.summary.average_approval_wait_secs, Some(90.0));
        assert_eq!(report.summary.recovery_events, 1);
        assert_eq!(report.summary.workflow_control_intervention_events, 3);
        assert_eq!(report.summary.pause_events, 1);
        assert_eq!(report.summary.resume_events, 1);
        assert_eq!(report.summary.workflow_budget_usage_events, 2);
        assert_eq!(report.summary.workflow_budget_exhausted_events, 1);
        assert_eq!(report.summary.max_workflow_output_tokens_spent, Some(10));
        assert_eq!(report.summary.max_workflow_output_token_budget, Some(10));
        assert!(report
            .markdown
            .contains("max closed/open approval wait: 90s/n/a"));
        assert!(report
            .markdown
            .contains("Budget events: 2 output-token sample(s), 1 exhausted"));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("output-token budget exhaustion")));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("workflow control interventions")));
    }

    #[test]
    fn domain_soak_report_tracks_open_approval_wait_age() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: session.id.clone(),
                kind: "domain:research".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: default_domain_workflow_script(),
                budget: json!({}),
                parent_run_id: None,
                origin: Some("soak-report-test".to_string()),
                goal_id: None,
                goal_criterion_id: None,
                worktree_id: None,
            })
            .unwrap();
        db.transition_workflow_run(
            &run.id,
            WorkflowRunState::AwaitingApproval,
            Some("permission_preview"),
        )
        .unwrap();
        {
            db.with_conn_for_test(|conn| {
                conn.execute(
                    "UPDATE workflow_events
                        SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-120 seconds')
                      WHERE run_id = ?1
                        AND type = 'run_state_changed'
                        AND payload_json LIKE '%\"to\":\"awaiting_approval\"%'",
                    params![run.id],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        }

        let report = generate_domain_soak_report(
            &db,
            DomainSoakReportInput {
                session_id: Some(session.id),
                window_days: Some(1),
                max_items: Some(20),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.summary.approval_request_events, 1);
        assert_eq!(report.summary.approval_decision_events, 0);
        assert_eq!(report.summary.open_approval_waits, 1);
        let open_wait = report.summary.max_open_approval_wait_secs.unwrap();
        assert!(
            (115..=125).contains(&open_wait),
            "unexpected open wait age: {open_wait}"
        );
        assert!(report.markdown.contains("1 open approval wait(s)"));
        assert!(report
            .recommended_next_steps
            .iter()
            .any(|step| step.contains("Resolve open workflow approvals")));
    }

    #[tokio::test]
    async fn domain_eval_campaign_external_item_fails_without_provider_secret() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("domain external campaign missing provider".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                models: vec![DomainEvalCampaignModel {
                    provider_id: Some("missing-provider".to_string()),
                    model_id: Some("missing-model".to_string()),
                    label: Some("Missing Model".to_string()),
                    credential_profile_ref: None,
                }],
                execution_mode: Some("agent".to_string()),
                providers: vec![mock_responses_provider(
                    "https://example.invalid".to_string(),
                    "other-provider",
                    "other-model",
                )],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(campaign.model_matrix.len(), 1);
        assert_eq!(
            campaign.model_matrix[0].provider_id.as_deref(),
            Some("missing-provider")
        );

        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id.clone(),
                providers: Vec::new(),
                retry_failed_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "failed");
        assert_eq!(completed.summary.failed_items, 1);
        assert!(completed.items[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("Provider config for missing-provider"));

        let leaderboard = db
            .get_domain_eval_campaign_leaderboard(DomainEvalCampaignLeaderboardInput {
                campaign_ids: vec![campaign.id],
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(leaderboard.status, "failed");
        assert_eq!(leaderboard.rows.len(), 1);
        assert_eq!(leaderboard.rows[0].failed_items, 1);
        assert!(leaderboard.rows[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("failed item")));
    }

    #[tokio::test]
    async fn domain_eval_fixture_agent_mode_requires_provider_config() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let report = run_domain_eval_fixture(
            db.clone(),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-requires-provider-config".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("failed".to_string()),
                        error_contains: vec!["requires providers and modelChain".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(!report.passed);
        assert_eq!(report.status, "failed");
        assert!(report.eval_run.is_none());
        assert!(report.execution.is_some());
        assert!(report
            .error
            .unwrap()
            .contains("requires providers and modelChain"));
        let fixture_runs = db
            .list_domain_eval_fixture_runs(ListDomainEvalFixtureRunsInput {
                window_days: Some(1),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(fixture_runs.len(), 1);
        assert_eq!(
            fixture_runs[0].source_type,
            DOMAIN_EVAL_SOURCE_FIXTURE_AGENT
        );
        assert_eq!(fixture_runs[0].status, "failed");
        assert!(fixture_runs[0].eval_run_id.is_none());
    }

    #[tokio::test]
    async fn domain_eval_fixture_agent_mode_calls_chat_engine_and_records_turn() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_text("domain agent execution completed")),
            )
            .mount(&server)
            .await;

        let provider = mock_responses_provider(
            server.uri(),
            "domain-eval-mock-provider",
            "mock-domain-model",
        );
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let report = run_domain_eval_fixture(
            db.clone(),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-execution-domain-fixture".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    execution: DomainEvalFixtureExecution {
                        prompt: Some("Say the domain eval agent runner completed.".to_string()),
                        providers: vec![provider],
                        model_chain: vec![ActiveModel {
                            provider_id: "domain-eval-mock-provider".to_string(),
                            model_id: "mock-domain-model".to_string(),
                        }],
                        ..Default::default()
                    },
                    quality: Some(DomainEvalFixtureQuality {
                        run: false,
                        ..Default::default()
                    }),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("completed".to_string()),
                        require_turn: Some(true),
                        expected_status: Some("failed".to_string()),
                        response_contains: vec!["domain agent execution completed".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(report.passed, "{report:?}");
        assert_eq!(report.status, "passed");
        assert!(report.eval_run.is_some());
        let execution = report.execution.expect("execution report");
        assert_eq!(execution.status, "completed");
        assert!(execution.turn_id.is_some());
        assert_eq!(
            execution.response.as_deref(),
            Some("domain agent execution completed")
        );
        assert_eq!(
            db.get_session_workflow_mode(&report.session_id).unwrap(),
            Some(WorkflowMode::Ultracode)
        );
    }

    #[tokio::test]
    async fn domain_eval_fixture_agent_mode_does_not_materialize_trace_seed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses_sse_text("agent completed without tools")),
            )
            .mount(&server)
            .await;

        let provider = mock_responses_provider(
            server.uri(),
            "domain-eval-seed-provider",
            "mock-domain-model",
        );
        let (_dir, db) = test_db();
        let report = run_domain_eval_fixture(
            Arc::new(db),
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-seed-evidence-is-not-auto-recorded".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    execution: DomainEvalFixtureExecution {
                        prompt: Some("Complete without recording domain evidence.".to_string()),
                        providers: vec![provider],
                        model_chain: vec![ActiveModel {
                            provider_id: "domain-eval-seed-provider".to_string(),
                            model_id: "mock-domain-model".to_string(),
                        }],
                        ..Default::default()
                    },
                    evidence: vec![DomainEvalFixtureEvidence {
                        evidence_type: "source_cited".to_string(),
                        title: "Trace seed that must not count in agent mode".to_string(),
                        source_metadata: json!({"uri": "https://example.com/seed", "retrievedAt": "2026-07-04"}),
                        ..Default::default()
                    }],
                    workflow: Some(DomainEvalFixtureWorkflow::default()),
                    quality: Some(DomainEvalFixtureQuality {
                        run: false,
                        ..Default::default()
                    }),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("completed".to_string()),
                        expected_status: Some("failed".to_string()),
                        expected_failed_checks: vec!["evidence_completeness".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(report.passed, "{report:?}");
        assert!(report.workflow_run_id.is_none());
        let eval_run = report.eval_run.expect("eval run");
        assert_eq!(eval_run.report.summary.total_evidence, 0);
        assert_eq!(eval_run.report.summary.workflow_runs, 0);
    }

    #[test]
    fn domain_quality_gate_passes_with_eval_and_quality_evidence() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let goal = db
            .create_goal(ha_core::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare research brief".to_string(),
                completion_criteria: "Sources and claims are verified".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.create_workflow_run(ha_core::workflow::CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "domain:research".to_string(),
            execution_mode: "guarded".to_string(),
            script_source:
                "export default async function main(workflow) { await workflow.finish({ status: 'done' }); }"
                    .to_string(),
            budget: json!({}),
            parent_run_id: None,
            origin: Some("test".to_string()),
            goal_id: Some(goal.goal.id.clone()),
            goal_criterion_id: None,
            worktree_id: None,
        })
        .unwrap();
        for i in 0..3 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "source_cited",
                &format!("Source {i}"),
                json!({"uri": format!("https://example.com/{i}"), "retrievedAt": "2026-07-03"}),
            );
        }
        for i in 0..2 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "claim_checked",
                &format!("Claim {i}"),
                json!({"claim": format!("claim {i}"), "verdict": "supported"}),
            );
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "citation_audited",
            "Citation audit",
            json!({"coverage": "all key claims"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "user_decision",
            "Publish approval not requested",
            json!({"decision": "draft only"}),
        );

        let quality = crate::domain_quality::run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id.clone(),
                domain: Some("research".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(quality.run.state.as_str(), "completed");
        let eval = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id.clone(),
                task_id: "research-source-backed-brief".to_string(),
                label: None,
                source_quality_run_id: Some(quality.run.id),
                source_type: None,
            })
            .unwrap();
        assert_eq!(eval.status, "passed");

        let gate = evaluate_domain_quality_gate(
            &db,
            DomainQualityGateInput {
                session_id: Some(session.id),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_pass_rate: Some(1.0),
                min_average_score: Some(0.8),
                require_approval_safety: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(gate.status, "passed");
        assert_eq!(gate.summary.eval_runs, 1);
        assert_eq!(gate.summary.completed_quality_runs, 1);
    }

    #[tokio::test]
    async fn domain_readiness_gate_passes_with_live_quality_and_campaign_evidence() {
        let (_dir, db) = test_db();
        let session = db
            .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let goal = db
            .create_goal(ha_core::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Prepare research brief".to_string(),
                completion_criteria: "Sources and claims are verified".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .unwrap();
        db.create_workflow_run(ha_core::workflow::CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "domain:research".to_string(),
            execution_mode: "guarded".to_string(),
            script_source:
                "export default async function main(workflow) { await workflow.finish({ status: 'done' }); }"
                    .to_string(),
            budget: json!({}),
            parent_run_id: None,
            origin: Some("test".to_string()),
            goal_id: Some(goal.goal.id.clone()),
            goal_criterion_id: None,
            worktree_id: None,
        })
        .unwrap();
        for i in 0..3 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "source_cited",
                &format!("Source {i}"),
                json!({"uri": format!("https://example.com/{i}"), "retrievedAt": "2026-07-03"}),
            );
        }
        for i in 0..2 {
            record_evidence(
                &db,
                &session.id,
                "research",
                "claim_checked",
                &format!("Claim {i}"),
                json!({"claim": format!("claim {i}"), "verdict": "supported"}),
            );
        }
        record_evidence(
            &db,
            &session.id,
            "research",
            "citation_audited",
            "Citation audit",
            json!({"coverage": "all key claims"}),
        );
        record_evidence(
            &db,
            &session.id,
            "research",
            "user_decision",
            "Draft only",
            json!({"decision": "draft only"}),
        );
        let quality = crate::domain_quality::run_domain_quality_for_session(
            &db,
            RunDomainQualityInput {
                session_id: session.id.clone(),
                domain: Some("research".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let eval = db
            .run_domain_eval_task(RunDomainEvalTaskInput {
                session_id: session.id.clone(),
                task_id: "research-source-backed-brief".to_string(),
                label: None,
                source_quality_run_id: Some(quality.run.id),
                source_type: None,
            })
            .unwrap();
        assert_eq!(eval.status, "passed");

        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                session_id: Some(session.id.clone()),
                name: Some("readiness trace campaign".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                execution_mode: Some("trace_fixture".to_string()),
                ..Default::default()
            })
            .unwrap();
        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id,
                providers: Vec::new(),
                retry_failed_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "passed");

        let readiness = evaluate_domain_readiness_gate(
            &db,
            DomainReadinessGateInput {
                session_id: Some(session.id),
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_campaign_items: Some(1),
                min_leaderboard_rows: Some(1),
                max_failed_campaign_items: Some(0),
                max_open_learning_proposals: Some(0),
                require_approval_safety: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(readiness.status, "passed", "{readiness:?}");
        assert_eq!(readiness.summary.eval_runs, 1);
        assert_eq!(readiness.summary.quality_runs, 1);
        assert_eq!(readiness.summary.campaign_items, 1);
        assert_eq!(readiness.summary.open_learning_proposals, 0);
        assert!(readiness.blockers.is_empty());
    }

    #[tokio::test]
    async fn domain_readiness_gate_blocks_failed_campaign_without_learning_closure() {
        let (_dir, db) = test_db();
        let db = Arc::new(db);
        let campaign = db
            .create_domain_eval_campaign(CreateDomainEvalCampaignInput {
                name: Some("readiness failed campaign".to_string()),
                task_ids: vec!["research-source-backed-brief".to_string()],
                max_tasks: Some(1),
                models: vec![DomainEvalCampaignModel {
                    provider_id: Some("missing-provider".to_string()),
                    model_id: Some("missing-model".to_string()),
                    label: Some("Missing Model".to_string()),
                    credential_profile_ref: None,
                }],
                execution_mode: Some("agent".to_string()),
                providers: vec![mock_responses_provider(
                    "https://example.invalid".to_string(),
                    "other-provider",
                    "other-model",
                )],
                ..Default::default()
            })
            .unwrap();
        let completed = run_domain_eval_campaign(
            db.clone(),
            RunDomainEvalCampaignInput {
                campaign_id: campaign.id,
                providers: Vec::new(),
                retry_failed_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "failed");

        let readiness = evaluate_domain_readiness_gate(
            &db,
            DomainReadinessGateInput {
                min_eval_runs: Some(1),
                min_quality_runs: Some(1),
                min_campaign_items: Some(1),
                min_leaderboard_rows: Some(1),
                max_failed_campaign_items: Some(0),
                max_open_learning_proposals: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(readiness.status, "failed");
        assert_eq!(readiness.summary.failed_campaign_items, 1);
        assert_eq!(readiness.summary.pending_learning_campaigns, 1);
        assert!(readiness
            .blockers
            .iter()
            .any(|blocker| blocker == "campaign_failures"));
        assert!(readiness
            .blockers
            .iter()
            .any(|blocker| blocker == "learning_closure"));
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn fixture_source_type_never_conflates_trace_and_agent() {
        assert_eq!(
            fixture_source_type("trace_fixture"),
            DOMAIN_EVAL_SOURCE_FIXTURE_TRACE
        );
        assert_eq!(
            fixture_source_type("agent"),
            DOMAIN_EVAL_SOURCE_FIXTURE_AGENT
        );
        assert_eq!(
            fixture_source_type("unknown"),
            DOMAIN_EVAL_SOURCE_FIXTURE_UNSUPPORTED
        );
    }

    #[tokio::test]
    async fn agent_fixture_without_provider_configuration_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db"))
                .expect("open session database"),
        );
        let report = run_domain_eval_fixture(
            db,
            RunDomainEvalFixtureInput {
                fixture: DomainEvalFixture {
                    name: "agent-requires-provider-config".to_string(),
                    task_id: "research-source-backed-brief".to_string(),
                    execution_mode: "agent".to_string(),
                    checks: DomainEvalFixtureChecks {
                        expected_execution_status: Some("failed".to_string()),
                        error_contains: vec!["requires providers and modelChain".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

        assert!(!report.passed);
        assert_eq!(report.status, "failed");
        assert!(report.eval_run.is_none());
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("requires providers and modelChain")));
    }
}
