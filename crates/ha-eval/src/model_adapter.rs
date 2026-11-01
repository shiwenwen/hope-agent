//! Registered real-model adapters.
//!
//! Assets select one of these Rust implementations by enum. They never carry
//! a command, executable, or shell fragment. `hope_core_scenario` drives a
//! separately provisioned Hope server through its production HTTP chat path;
//! Provider credentials remain in that server process and are not copied into
//! the runner or task environment.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use ha_eval_spec::model::{
    causal_critical_path_ms, AttributionCompleteness, CampaignBudget, CostMetrics, FaultKind,
    FaultProfile, InvariantKind, LiveAgentScenario, ModelCampaignAdapter, ModelCampaignCheck,
    ModelCampaignEvent, ModelCampaignOutcome, ModelProfile, ModelTrialResult, OrchestrationMetrics,
    PlannedModelCase, PlannedModelTrial, TimingMetrics, TokenMetrics, ToolMetrics, TraceSummary,
    UserSimulatorKind, VerifierKind, TRIAL_SCHEMA_VERSION,
};
use ha_eval_spec::{digest_file, read_json, resolve_contained, sha256_bytes, ArtifactDigest};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const SERVER_URL_ENV: &str = "HA_MODEL_EVAL_SERVER_URL";
const SERVER_TOKEN_ENV: &str = "HA_MODEL_EVAL_SERVER_TOKEN";
const SUPERVISOR_URL_ENV: &str = "HA_MODEL_EVAL_SUPERVISOR_URL";
const SUPERVISOR_TOKEN_ENV: &str = "HA_MODEL_EVAL_SUPERVISOR_TOKEN";
const TRIAL_CLEANUP_GRACE_SECONDS: u64 = 10;
// This is an infrastructure deadlock fuse, not a campaign budget. Unlimited
// evaluations bypass user budget dimensions but must still terminate if the
// isolated Hope process cannot converge after explicit cancellation.
const TRIAL_CLEANUP_SAFETY_FUSE_SECONDS: u64 = 75;
pub(crate) const CLEANUP_INCOMPLETE_FAILURE_CLASS: &str = "cleanup_incomplete_after_evidence";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatResponse {
    session_id: String,
    response: String,
    turn_id: String,
    #[serde(default)]
    blocked_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScriptedUserFlowV1 {
    schema_version: String,
    #[serde(default)]
    turns: Vec<ScriptedUserTurn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScriptedUserTurn {
    message: String,
    #[serde(default)]
    delay_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScriptedUserFlowV2 {
    schema_version: String,
    #[serde(default)]
    steps: Vec<ScriptedUserStep>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ScriptedUserStep {
    Message {
        message: String,
        #[serde(default, rename = "delayMs")]
        delay_ms: u64,
    },
    UpdateGoal {
        objective: String,
        #[serde(rename = "completionCriteria")]
        completion_criteria: String,
    },
    CompactContext {
        #[serde(rename = "expectedTier")]
        expected_tier: u8,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompactContextResponse {
    tier_applied: u8,
    tokens_before: u32,
    tokens_after: u32,
    messages_affected: usize,
    description: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TelemetrySnapshot {
    attribution: AttributionCompleteness,
    #[serde(default)]
    #[allow(dead_code)]
    active_children: u32,
    #[serde(default)]
    background_model_work: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pending_work: u32,
    #[serde(default)]
    budget_exhausted: bool,
    #[serde(default)]
    budget_exhaustion_reasons: Vec<String>,
    timings: TimingMetrics,
    tokens: TokenMetrics,
    cost: CostMetrics,
    tools: ToolMetrics,
    orchestration: OrchestrationMetrics,
    trace: TraceSummary,
    #[serde(default)]
    milestones: Vec<ModelCampaignCheck>,
    #[serde(default)]
    invariants: Vec<ModelCampaignCheck>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    observed_signals: Vec<String>,
    #[serde(default)]
    events: Vec<ModelCampaignEvent>,
}

pub async fn run_registered_trial(
    root: &Path,
    adapter: ModelCampaignAdapter,
    scenario_path: &Path,
    scenario: &LiveAgentScenario,
    planned_case: &PlannedModelCase,
    trial: &PlannedModelTrial,
    model: &ModelProfile,
    attempt: u8,
    unlimited_budget: bool,
) -> ModelTrialResult {
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let started = Instant::now();
    if single_agent_fault_not_applicable(scenario, trial) {
        let mut result = empty_failure_result(
            trial, attempt, started_at, Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            elapsed_ms(started), ModelCampaignOutcome::BenchmarkDefect,
            "comparison_arm_not_applicable",
            "registered fault targets a child executor; the single-Agent arm forbids children, so no model call was dispatched",
        );
        result.attribution = AttributionCompleteness::Complete;
        result.tokens.input = Some(0);
        result.tokens.output = Some(0);
        result.tokens.cache_read = Some(0);
        result.tokens.cache_write = Some(0);
        result.tokens.usage_source = Some("not_dispatched".to_string());
        result.cost.total_usd = Some(0.0);
        result.cost.agent_usd = Some(0.0);
        result.warnings =
            vec!["not applicable: proven pre-dispatch rejection, not a model failure".to_string()];
        result.invariants.push(ModelCampaignCheck {
            id: "model_binding".to_string(),
            passed: true,
            blocking: true,
            detail:
                "no Provider dispatch: unsupported comparison arm rejected before product setup"
                    .to_string(),
            metric: Some(0.0),
            artifact_hashes: Vec::new(),
        });
        return result;
    }
    if scenario.user_simulator.kind == UserSimulatorKind::Llm {
        let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        return empty_failure_result(
            trial,
            attempt,
            started_at,
            completed_at,
            elapsed_ms(started),
            ModelCampaignOutcome::BenchmarkDefect,
            "user_simulator_not_installed",
            "scenario requires an LLM user simulator but the registered v1 Hope Harness only supports scripted_fsm and replay",
        );
    }
    let needs_restart_supervisor = trial.fault_profile == FaultProfile::Chaos
        && scenario
            .faults
            .iter()
            .any(|fault| fault.kind == FaultKind::ProcessRestart);
    if needs_restart_supervisor
        && (std::env::var(SUPERVISOR_URL_ENV).is_err()
            || std::env::var(SUPERVISOR_TOKEN_ENV).is_err())
    {
        let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        return empty_failure_result(
            trial,
            attempt,
            started_at,
            completed_at,
            elapsed_ms(started),
            ModelCampaignOutcome::InfraError,
            "process_restart_supervisor_unavailable",
            "scenario requires the authenticated model-eval process supervisor",
        );
    }
    let execution = match adapter {
        ModelCampaignAdapter::HopeCoreScenario => {
            run_hope_core_scenario(
                root,
                scenario_path,
                scenario,
                planned_case,
                trial,
                model,
                attempt,
                unlimited_budget,
            )
            .await
        }
        other => Err(anyhow!(
            "registered adapter {other:?} has no installed harness implementation"
        )),
    };
    let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let wall_ms = elapsed_ms(started);

    let mut result = match execution {
        Ok(mut observed) => {
            // The full trial includes setup, grading and cleanup. Agent-only
            // telemetry can otherwise disagree with terminal timestamps and
            // turn valid paid evidence into a subprocess failure/retry.
            observed.telemetry.timings.wall_ms = wall_ms;
            if observed.telemetry.timings.critical_path_ms == 0 {
                observed.telemetry.timings.critical_path_ms = wall_ms;
            }
            let exceeded = exceeded_budgets(planned_case, &observed.telemetry);
            if observed.failure_class.as_deref() != Some(CLEANUP_INCOMPLETE_FAILURE_CLASS)
                && observed.telemetry.budget_exhausted
                && !matches!(
                    observed.outcome,
                    ModelCampaignOutcome::PolicyFailed
                        | ModelCampaignOutcome::BenchmarkDefect
                        | ModelCampaignOutcome::SimulatorError
                        | ModelCampaignOutcome::Cancelled
                )
            {
                observed.outcome = ModelCampaignOutcome::BudgetExhausted;
                observed.failure_class = Some("trial_budget_exhausted".to_string());
                observed.telemetry.warnings.push(format!(
                    "runtime stopped at immutable trial budget: {}",
                    observed.telemetry.budget_exhaustion_reasons.join(", ")
                ));
            }
            if !exceeded.is_empty()
                && matches!(
                    observed.outcome,
                    ModelCampaignOutcome::Passed | ModelCampaignOutcome::TaskFailed
                )
            {
                observed.outcome = ModelCampaignOutcome::BudgetExhausted;
                observed.failure_class = Some("trial_budget_exhausted".to_string());
                observed
                    .telemetry
                    .warnings
                    .push(format!("exceeded trial budgets: {}", exceeded.join(", ")));
            }
            ModelTrialResult {
                schema_version: TRIAL_SCHEMA_VERSION.to_string(),
                trial_id: trial.id.clone(),
                campaign_id: trial.campaign_id.clone(),
                plan_digest: trial.plan_digest.clone(),
                commit_sha: trial.commit_sha.clone(),
                app_version: trial.app_version.clone(),
                suite_id: trial.suite_id.clone(),
                suite_version: trial.suite_version.clone(),
                suite_digest: trial.suite_digest.clone(),
                case_id: trial.case_id.clone(),
                case_digest: trial.case_digest.clone(),
                trial_index: trial.trial_index,
                seed: trial.seed,
                model_role: trial.model_role.clone(),
                arm: trial.arm.clone(),
                fault_profile: trial.fault_profile,
                orchestration_profile: trial.orchestration_profile.clone(),
                model_digest: trial.model_digest.clone(),
                scenario_digest: trial.scenario_digest.clone(),
                environment_digest: trial.environment_digest.clone(),
                verifier_digest: trial.verifier_digest.clone(),
                prompt_digest: trial.prompt_digest.clone(),
                tool_schema_digest: trial.tool_schema_digest.clone(),
                runtime_config_digest: crate::model::current_runtime_config_digest(),
                attempt,
                prior_attempts: Vec::new(),
                started_at,
                completed_at,
                outcome: observed.outcome,
                failure_class: observed.failure_class,
                attribution: observed.telemetry.attribution,
                timings: observed.telemetry.timings,
                tokens: observed.telemetry.tokens,
                cost: observed.telemetry.cost,
                tools: observed.telemetry.tools,
                orchestration: observed.telemetry.orchestration,
                milestones: observed.telemetry.milestones,
                invariants: observed.telemetry.invariants,
                judge_checks: Vec::new(),
                trace: observed.telemetry.trace,
                trace_events: observed.telemetry.events,
                artifacts: observed.artifacts,
                warnings: observed.telemetry.warnings,
                error: observed.error,
            }
        }
        Err(error) => {
            let mut result = empty_failure_result(
                trial,
                attempt,
                started_at,
                completed_at,
                wall_ms,
                if adapter == ModelCampaignAdapter::HopeCoreScenario {
                    ModelCampaignOutcome::InfraError
                } else {
                    ModelCampaignOutcome::BenchmarkDefect
                },
                if adapter == ModelCampaignAdapter::HopeCoreScenario {
                    "harness_unavailable"
                } else {
                    "adapter_not_installed"
                },
                &format!("{error:#}"),
            );
            if adapter == ModelCampaignAdapter::HopeCoreScenario {
                recover_failed_trial_usage(trial, attempt, &mut result).await;
                result.completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
                result.timings.wall_ms = elapsed_ms(started);
                result.timings.critical_path_ms =
                    result.timings.critical_path_ms.min(result.timings.wall_ms);
            }
            result
        }
    };
    let binding = model_binding_check(model, &result.trace_events);
    if !binding.passed {
        result.outcome = ModelCampaignOutcome::InfraError;
        result.failure_class = Some("model_binding_mismatch".to_string());
        result.warnings.push(binding.detail.clone());
    }
    result.invariants = merge_checks(result.invariants, vec![binding]);
    result
}

fn model_binding_check(model: &ModelProfile, events: &[ModelCampaignEvent]) -> ModelCampaignCheck {
    let calls: Vec<_> = events
        .iter()
        .filter(|event| event.event == "model.call")
        .collect();
    let mismatches = calls
        .iter()
        .filter(|event| {
            event.attributes.get("providerId").and_then(Value::as_str)
                != Some(model.provider_id.as_str())
                || event.attributes.get("modelId").and_then(Value::as_str)
                    != Some(model.model_id.as_str())
        })
        .count();
    ModelCampaignCheck {
        id: "model_binding".to_string(),
        passed: mismatches == 0,
        blocking: true,
        detail: format!(
            "{} actual model calls checked against {}::{}; {} mismatched or unverified",
            calls.len(),
            model.provider_id,
            model.model_id,
            mismatches
        ),
        metric: Some(mismatches as f64),
        artifact_hashes: Vec::new(),
    }
}

/// Recover real usage from the authenticated product registry when a worker
/// or harness fails. This only observes/cancels the existing trial; it never
/// dispatches a model request or reruns the task.
pub(crate) async fn recover_failed_trial_usage(
    trial: &PlannedModelTrial,
    attempt: u8,
    result: &mut ModelTrialResult,
) {
    let recovery = async {
        let server_url = std::env::var(SERVER_URL_ENV).ok()?;
        let token = std::env::var(SERVER_TOKEN_ENV).ok();
        let id = crate::model::product_trial_id(&trial.campaign_id, &trial.id, attempt);
        recover_trial_snapshot(&server_url, token.as_deref(), &id).await
    };
    match tokio::time::timeout(Duration::from_secs(110), recovery).await {
        Ok(Some((telemetry, cleanup_error))) => {
            apply_recovered_telemetry(result, telemetry, cleanup_error);
        }
        _ => result
            .warnings
            .push("failed-worker telemetry unavailable; model usage remains unknown".to_string()),
    }
}

async fn recover_trial_snapshot(
    server_url: &str,
    token: Option<&str>,
    id: &str,
) -> Option<(TelemetrySnapshot, Option<String>)> {
    let url = reqwest::Url::parse(server_url).ok()?;
    if url.scheme() != "http"
        || url.port().is_none()
        || !url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
    {
        return None;
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let before = fetch_telemetry(&client, server_url, token, id).await?;
    match cleanup_and_wait_for_trial(&client, server_url, token, id).await {
        Ok(after) => Some((after, None)),
        Err(error) => {
            let last = fetch_telemetry(&client, server_url, token, id)
                .await
                .unwrap_or(before);
            Some((last, Some(sanitize_error(&format!("{error:#}")))))
        }
    }
}

fn apply_recovered_telemetry(
    result: &mut ModelTrialResult,
    telemetry: TelemetrySnapshot,
    cleanup_error: Option<String>,
) {
    if cleanup_error.is_some() || !telemetry.trace.closed || telemetry.trace.orphan_span_count > 0 {
        result.outcome = ModelCampaignOutcome::InfraError;
        result.failure_class = Some(CLEANUP_INCOMPLETE_FAILURE_CLASS.to_string());
        result.error = Some(format_failure_with_cleanup(
            result
                .error
                .take()
                .unwrap_or_else(|| "trial worker failed".to_string()),
            cleanup_error,
        ));
    }
    result.attribution = telemetry.attribution;
    result.tokens = telemetry.tokens;
    result.cost = telemetry.cost;
    result.tools = telemetry.tools;
    result.orchestration = telemetry.orchestration;
    result.trace = telemetry.trace;
    result.trace_events = telemetry.events;
    result.milestones = telemetry.milestones;
    result.invariants = telemetry.invariants;
    result.timings.model_active_ms = telemetry.timings.model_active_ms;
    result.timings.tool_active_ms = telemetry.timings.tool_active_ms;
    result.timings.ttft_ms = telemetry.timings.ttft_ms;
    result.warnings.extend(telemetry.warnings);
    result.warnings.push(
        "actual product telemetry preserved after worker failure; task was not rerun".to_string(),
    );
}

fn exceeded_budgets(
    planned_case: &PlannedModelCase,
    telemetry: &TelemetrySnapshot,
) -> Vec<&'static str> {
    let budget = &planned_case.budget;
    let mut exceeded = Vec::new();
    if budget
        .max_wall_seconds
        .is_some_and(|limit| telemetry.timings.wall_ms > limit.saturating_mul(1_000))
    {
        exceeded.push("wall");
    }
    if budget
        .max_model_calls
        .is_some_and(|limit| telemetry.orchestration.model_calls > limit)
    {
        exceeded.push("model_calls");
    }
    if budget
        .max_input_tokens
        .zip(telemetry.tokens.input)
        .is_some_and(|(limit, actual)| actual > limit)
    {
        exceeded.push("input_tokens");
    }
    if budget
        .max_output_tokens
        .zip(telemetry.tokens.output)
        .is_some_and(|(limit, actual)| actual > limit)
    {
        exceeded.push("output_tokens");
    }
    if budget
        .max_cost_usd
        .zip(telemetry.cost.total_usd)
        .is_some_and(|(limit, actual)| actual > limit)
    {
        exceeded.push("cost");
    }
    if budget
        .max_tool_calls
        .is_some_and(|limit| telemetry.tools.attempted > limit)
    {
        exceeded.push("tool_calls");
    }
    if budget
        .max_agents
        .is_some_and(|limit| telemetry.orchestration.spawned_agents > u64::from(limit))
    {
        exceeded.push("agents");
    }
    if budget
        .max_concurrency
        .is_some_and(|limit| telemetry.orchestration.max_concurrency > limit)
    {
        exceeded.push("concurrency");
    }
    exceeded
}

struct ObservedTrial {
    outcome: ModelCampaignOutcome,
    failure_class: Option<String>,
    telemetry: TelemetrySnapshot,
    artifacts: Vec<ArtifactDigest>,
    error: Option<String>,
}

fn compact_context_contract_check(
    response: &CompactContextResponse,
    expected_tier: u8,
    action_index: usize,
    response_hash: String,
) -> ModelCampaignCheck {
    let passed = response.tier_applied == expected_tier
        && response.description == "summarized"
        && response.tokens_after < response.tokens_before
        && response.messages_affected > 0;
    ModelCampaignCheck {
        id: format!("scripted_compact_context_{action_index}"),
        passed,
        blocking: true,
        detail: format!(
            "manual compaction tier={}, description={}, tokens {}→{}, messagesAffected={}",
            response.tier_applied,
            response.description,
            response.tokens_before,
            response.tokens_after,
            response.messages_affected,
        ),
        metric: Some(f64::from(
            response.tokens_before.saturating_sub(response.tokens_after),
        )),
        artifact_hashes: vec![response_hash],
    }
}

async fn timed_out_chat_result(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    trial: &PlannedModelTrial,
    product_trial_id: &str,
    stage: &str,
) -> ObservedTrial {
    let telemetry = match tokio::time::timeout(
        Duration::from_secs(TRIAL_CLEANUP_GRACE_SECONDS.saturating_sub(2).max(1)),
        cleanup_after_failed_chat(client, server_url, token, trial, product_trial_id),
    )
    .await
    {
        Ok((telemetry, _)) => telemetry,
        _ => fallback_telemetry(trial, 0),
    };
    ObservedTrial {
        outcome: ModelCampaignOutcome::BudgetExhausted,
        failure_class: Some("trial_wall_timeout".to_string()),
        telemetry,
        artifacts: Vec::new(),
        error: Some(format!(
            "{stage} exceeded the immutable trial wall-clock budget"
        )),
    }
}

async fn run_hope_core_scenario(
    root: &Path,
    scenario_path: &Path,
    scenario: &LiveAgentScenario,
    planned_case: &PlannedModelCase,
    trial: &PlannedModelTrial,
    model: &ModelProfile,
    attempt: u8,
    unlimited_budget: bool,
) -> Result<ObservedTrial> {
    let server_url = std::env::var(SERVER_URL_ENV)
        .with_context(|| format!("{SERVER_URL_ENV} is required for hope_core_scenario"))?;
    let server_url = server_url.trim_end_matches('/');
    // Paired models share a logical trial ID. Runtime identities must also
    // include the campaign so neither another model nor a retry can reuse a
    // previous root Session or still-closing work.
    let product_trial_id = crate::model::product_trial_id(&trial.campaign_id, &trial.id, attempt);
    if !server_url.starts_with("http://127.0.0.1:")
        && !server_url.starts_with("http://localhost:")
        && !server_url.starts_with("https://")
    {
        bail!("model eval server URL must use loopback HTTP or HTTPS");
    }
    let token = std::env::var(SERVER_TOKEN_ENV).ok();
    if dedicated_source() && token.as_deref().is_none_or(str::is_empty) {
        bail!("dedicated model evaluation requires {SERVER_TOKEN_ENV}");
    }
    let scenario_dir = scenario_path
        .parent()
        .ok_or_else(|| anyhow!("scenario path has no parent"))?;
    validate_scripted_user_flow(scenario_dir, scenario)?;
    let setup_started = Instant::now();
    let workspace = provision_workspace(scenario_dir, scenario)?;
    let environment_setup_ms = elapsed_ms(setup_started);
    let prompt_path = resolve_contained(scenario_dir, trial_prompt_path(scenario, trial)?)?;
    let mut prompt = std::fs::read_to_string(&prompt_path)
        .with_context(|| format!("reading scenario prompt {}", prompt_path.display()))?;
    if prompt.trim().is_empty() {
        bail!("scenario prompt is empty");
    }
    if scenario_has_ready_control_plane(scenario) {
        prompt.push_str(match trial.fault_profile {
            FaultProfile::Clean => "\n\n本轮是 clean/control 正常路径：场景名称描述的是专项能力，并不表示本轮已注入故障。不要主动制造失败、取消、重启或审批拒绝；完成公开任务并验证正常终态。",
            FaultProfile::Chaos => "\n\n本轮是 chaos/faulted 分支：只响应实际注入的故障或明确收到的用户事件，不能自行伪造故障、审批答案或恢复证据。",
        });
    }
    match trial.orchestration_profile.as_deref() {
        Some(profile) if profile.starts_with("single_agent") => {
            prompt.push_str(
                "\n\n评测实验约束：这是计算量配平的单 Agent 对照臂。不得创建 Subagent 或 Team；在相同总 Token、工具、费用和时间预算内由当前 Agent 独立完成。",
            );
        }
        Some("team_full") => {
            prompt.push_str(
                "\n\n评测实验约束：这是完整多 Agent 臂。请把可并行且边界清晰的工作分派给真实 Subagent/Team，消费每个结果并在统一终态验证后再完成。",
            );
        }
        _ => {}
    }

    let mut client_builder = Client::builder();
    if !unlimited_budget {
        let request_timeout_seconds = planned_case
            .timeout_seconds
            .saturating_sub(TRIAL_CLEANUP_GRACE_SECONDS)
            .max(1);
        client_builder = client_builder.timeout(Duration::from_secs(request_timeout_seconds));
    }
    let client = client_builder
        .build()
        .context("building model eval HTTP client")?;
    let mut required_signals = vec!["model".to_string()];
    for capability in &scenario.capabilities {
        let normalized = capability.to_ascii_lowercase().replace('-', "_");
        if matches!(
            normalized.as_str(),
            "goal" | "loop" | "workflow" | "async_jobs" | "subagent" | "team" | "tool"
        ) && !required_signals.contains(&normalized)
        {
            required_signals.push(normalized);
        }
    }
    let solo_profile = trial
        .orchestration_profile
        .as_deref()
        .is_some_and(|profile| profile.starts_with("single_agent"));
    if solo_profile {
        required_signals.retain(|signal| !matches!(signal.as_str(), "subagent" | "team"));
    }
    if trial.fault_profile == FaultProfile::Chaos
        && !scenario.faults.is_empty()
        && !required_signals.iter().any(|signal| signal == "fault")
    {
        required_signals.push("fault".to_string());
    }
    let active_faults = if trial.fault_profile == FaultProfile::Chaos {
        scenario.faults.clone()
    } else {
        Vec::new()
    };
    let process_restart_fault = trial.fault_profile == FaultProfile::Chaos
        && active_faults
            .iter()
            .any(|fault| fault.kind == FaultKind::ProcessRestart);
    let process_restart_trigger = active_faults
        .iter()
        .find(|fault| fault.kind == FaultKind::ProcessRestart)
        .map(|fault| fault.trigger.as_str());
    if let Some(trigger) = process_restart_trigger {
        prompt.push_str(process_restart_phase_instruction(trigger));
    }
    let user_event_trigger = active_faults
        .iter()
        .find(|fault| fault.kind == FaultKind::UserEvent)
        .map(|fault| fault.trigger.as_str());
    if let Some(trigger) = user_event_trigger {
        prompt.push_str(user_event_phase_instruction(
            trigger,
            &scenario.subject.config_profile,
        ));
    }
    let eval_context = json!({
        "evidenceKind": "model_campaign",
        "campaignId": trial.campaign_id,
        "planDigest": trial.plan_digest,
        "suiteId": trial.suite_id,
        "suiteVersion": trial.suite_version,
        "suiteDigest": trial.suite_digest,
        "caseId": trial.case_id,
        "caseDigest": trial.case_digest,
        "trialId": product_trial_id.clone(),
        "trialIndex": trial.trial_index,
        "traceId": format!("trace_{product_trial_id}"),
        "rootSpanId": format!(
            "span_{}",
            &product_trial_id[product_trial_id.len().saturating_sub(16)..]
        ),
        "modelRole": trial.model_role,
        "arm": trial.arm,
        "faultProfile": trial.fault_profile,
        "orchestrationProfile": trial.orchestration_profile,
        "seed": trial.seed,
        "source": evaluation_source(),
        "commitSha": trial.commit_sha,
        "dirty": worktree_is_dirty(root),
        "appVersion": trial.app_version,
        "requiredSignals": required_signals,
        "faults": active_faults.clone(),
        "budget": {
            "maxWallMs": planned_case.budget.max_wall_seconds.map(|value| value.saturating_mul(1_000)),
            "maxModelCalls": planned_case.budget.max_model_calls,
            "maxInputTokens": planned_case.budget.max_input_tokens,
            "maxOutputTokens": planned_case.budget.max_output_tokens,
            "maxCostMicros": planned_case.budget.max_cost_usd.map(|value| (value * 1_000_000.0).ceil() as u64),
            "maxToolCalls": planned_case.budget.max_tool_calls,
            "maxAgents": planned_case.budget.max_agents,
            "maxConcurrency": planned_case.budget.max_concurrency,
        },
    });
    let mut initial_eval_context = eval_context.clone();
    let mut continuation_eval_context = eval_context.clone();
    if process_restart_fault {
        initial_eval_context["faults"] = serde_json::to_value(
            active_faults
                .iter()
                .filter(|fault| fault.kind != FaultKind::ProcessRestart)
                .collect::<Vec<_>>(),
        )?;
    }
    let initial_goal = scenario_initial_goal(scenario, trial.fault_profile);
    let body = json!({
        "message": prompt,
        "incognito": scenario.id == "HA-AJ-005",
        "modelOverride": format!("{}::{}", model.provider_id, model.model_id),
        "temperatureOverride": model.temperature,
        "reasoningEffort": model.reasoning_effort,
        "workingDir": workspace,
        "initialGoal": initial_goal,
        "workflowMode": scenario_workflow_mode(scenario),
        "evalContext": initial_eval_context,
    });
    let mut request = client
        .post(format!("{server_url}/api/chat"))
        .header("x-hope-eval-trial", &product_trial_id)
        .header("x-hope-eval-case", &trial.case_id)
        .json(&body);
    if let Some(token) = token.as_deref().filter(|value| !value.is_empty()) {
        request = request.bearer_auth(token);
    }
    let agent_started = Instant::now();
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            return Ok(timed_out_chat_result(
                &client,
                server_url,
                token.as_deref(),
                trial,
                &product_trial_id,
                "Hope chat",
            )
            .await);
        }
        Err(error) => return Err(error).context("calling Hope server chat"),
    };
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("reading Hope server chat response")?;
    if !status.is_success() {
        let (outcome, classification) =
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                (ModelCampaignOutcome::InfraError, "provider_or_server_error")
            } else {
                (
                    ModelCampaignOutcome::BenchmarkDefect,
                    "invalid_harness_request",
                )
            };
        let (telemetry, cleanup_error) = cleanup_after_failed_chat(
            &client,
            server_url,
            token.as_deref(),
            trial,
            &product_trial_id,
        )
        .await;
        return Ok(ObservedTrial {
            outcome,
            failure_class: Some(classification.to_string()),
            telemetry,
            artifacts: Vec::new(),
            error: Some(format_failure_with_cleanup(
                format!(
                    "Hope server returned HTTP {}; reason: {}; body sha256:{}",
                    status,
                    http_error_hint(&bytes),
                    sha256_bytes(&bytes)
                ),
                cleanup_error,
            )),
        });
    }
    let mut chat: ChatResponse =
        serde_json::from_slice(&bytes).context("parsing Hope chat response")?;
    let mut response_hashes = vec![sha256_bytes(chat.response.as_bytes())];
    let mut request_blocked = chat.blocked_reason.is_some();
    let mut pre_restart_telemetry = None;
    let mut pre_restart_trigger_check = None;
    let mut user_event_precondition_check = None;
    let mut restart_wait_ms = 0;
    let mut scripted_wait_ms = 0u64;
    let mut scripted_action_checks = Vec::new();
    if !request_blocked {
        let user_event_is_fault = scenario
            .faults
            .iter()
            .any(|fault| fault.kind == FaultKind::UserEvent);
        let mut scripted_steps =
            if trial.fault_profile == FaultProfile::Chaos || !user_event_is_fault {
                load_scripted_user_steps(scenario_dir, scenario)?
            } else {
                Vec::new()
            };
        if let Some(trigger) = user_event_trigger {
            let mut before =
                fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id)
                    .await
                    .ok_or_else(|| anyhow!("pre-user-event Hope telemetry is unavailable"))?;
            if scripted_revision_requires_settlement(&scenario.subject.config_profile, trigger) {
                let started_wait = Instant::now();
                let remaining = (!unlimited_budget).then(|| {
                    Duration::from_secs(planned_case.timeout_seconds)
                        .saturating_sub(agent_started.elapsed())
                });
                before = observe_trial_completion_for_phase(
                    before,
                    remaining,
                    Duration::from_millis(250),
                    true,
                    || fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id),
                )
                .await;
                scripted_wait_ms = scripted_wait_ms.saturating_add(elapsed_ms(started_wait));
                if !prior_scripted_work_settled(&before) {
                    return Ok(timed_out_chat_result(
                        &client,
                        server_url,
                        token.as_deref(),
                        trial,
                        &product_trial_id,
                        "settling prior work before scripted revision",
                    )
                    .await);
                }
                scripted_action_checks.push(ModelCampaignCheck {
                    id: "scripted_revision_prior_work_settled".to_string(),
                    passed: prior_scripted_work_settled(&before), blocking: true,
                    detail: "draft revision waits for admitted prior work and parent notifications; cancellation and restart stages retain their active-work precondition".to_string(),
                    metric: None, artifact_hashes: Vec::new(),
                });
            }
            user_event_precondition_check = Some(user_event_precondition_observed(
                trigger, &before, &workspace,
            ));
        }
        if process_restart_fault {
            let before = fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id)
                .await
                .ok_or_else(|| anyhow!("pre-restart Hope telemetry is unavailable"))?;
            let trigger = process_restart_trigger
                .ok_or_else(|| anyhow!("process restart fault has no registered trigger"))?;
            pre_restart_trigger_check = Some(process_restart_trigger_observed(trigger, &before));
            pre_restart_telemetry = Some(before);
            let restart_started = Instant::now();
            restart_supervised_server(&client).await?;
            restart_wait_ms = elapsed_ms(restart_started);
            apply_remaining_restart_budget(
                &mut continuation_eval_context,
                &planned_case.budget,
                pre_restart_telemetry
                    .as_ref()
                    .expect("pre-restart telemetry was just recorded"),
                restart_wait_ms,
            );
            scripted_steps.insert(
                0,
                ScriptedUserStep::Message {
                    message: "Hope 已按评测计划重启。请从持久化状态恢复，不要重复已经提交的副作用；检查 Goal/Workflow checkpoint，只继续尚未完成的工作并重新验证终态。对于 Team，先查询原团队状态和成员结果；只有 paused 状态可 resume，若仍为 active 则先 pause 并确认旧成员执行已终止，再 resume；不得重复已完成工作。".to_string(),
                    delay_ms: 0,
                },
            );
        }
        let mut compact_action_index = 0usize;
        for step in scripted_steps {
            match step {
                ScriptedUserStep::Message { message, delay_ms } => {
                    if delay_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        scripted_wait_ms = scripted_wait_ms.saturating_add(delay_ms);
                    }
                    let body = json!({
                        "message": message,
                        "sessionId": chat.session_id.clone(),
                        "modelOverride": format!("{}::{}", model.provider_id, model.model_id),
                        "temperatureOverride": model.temperature,
                        "reasoningEffort": model.reasoning_effort,
                        "evalContext": continuation_eval_context.clone(),
                    });
                    let mut request = client
                        .post(format!("{server_url}/api/chat"))
                        .header("x-hope-eval-trial", &product_trial_id)
                        .header("x-hope-eval-case", &trial.case_id)
                        .json(&body);
                    if let Some(token) = token.as_deref().filter(|value| !value.is_empty()) {
                        request = request.bearer_auth(token);
                    }
                    let response = match request.send().await {
                        Ok(response) => response,
                        Err(error) if error.is_timeout() => {
                            return Ok(timed_out_chat_result(
                                &client,
                                server_url,
                                token.as_deref(),
                                trial,
                                &product_trial_id,
                                "Hope scripted user turn",
                            )
                            .await);
                        }
                        Err(error) => return Err(error).context("calling Hope scripted user turn"),
                    };
                    let status = response.status();
                    let bytes = response
                        .bytes()
                        .await
                        .context("reading Hope scripted user turn response")?;
                    if expected_scripted_cancellation(
                        status,
                        &bytes,
                        user_event_trigger,
                        user_event_precondition_check
                            .as_ref()
                            .is_some_and(|(passed, _)| *passed),
                    ) {
                        response_hashes.push(sha256_bytes(&bytes));
                        scripted_action_checks.push(ModelCampaignCheck {
                            id: "scripted_cancellation_transport".to_string(), passed: true, blocking: true,
                            detail: "registered cancellation reached the product; independent terminal-state and absent-artifact verifiers remain mandatory".to_string(),
                            metric: None, artifact_hashes: vec![sha256_bytes(&bytes)],
                        });
                        break;
                    }
                    if !status.is_success() {
                        let (telemetry, cleanup_error) = cleanup_after_failed_chat(
                            &client,
                            server_url,
                            token.as_deref(),
                            trial,
                            &product_trial_id,
                        )
                        .await;
                        return Ok(ObservedTrial {
                            outcome: if status == StatusCode::TOO_MANY_REQUESTS
                                || status.is_server_error()
                            {
                                ModelCampaignOutcome::InfraError
                            } else {
                                ModelCampaignOutcome::BenchmarkDefect
                            },
                            failure_class: Some(if status.is_server_error() {
                                "provider_or_server_error".to_string()
                            } else {
                                "scripted_turn_rejected".to_string()
                            }),
                            telemetry,
                            artifacts: Vec::new(),
                            error: Some(format_failure_with_cleanup(
                                format!(
                                    "Hope scripted turn returned HTTP {status}; reason: {}; body sha256:{}",
                                    http_error_hint(&bytes), sha256_bytes(&bytes)
                                ),
                                cleanup_error,
                            )),
                        });
                    }
                    chat = serde_json::from_slice(&bytes)
                        .context("parsing Hope scripted user turn response")?;
                    response_hashes.push(sha256_bytes(chat.response.as_bytes()));
                    if chat.blocked_reason.is_some() {
                        request_blocked = true;
                        break;
                    }
                }
                ScriptedUserStep::UpdateGoal {
                    objective,
                    completion_criteria,
                } => {
                    let check = apply_scripted_goal_revision(
                        &client,
                        server_url,
                        token.as_deref(),
                        &chat.session_id,
                        &objective,
                        &completion_criteria,
                    )
                    .await?;
                    let passed = check.passed;
                    scripted_action_checks.push(check);
                    if !passed {
                        break;
                    }
                }
                ScriptedUserStep::CompactContext { expected_tier } => {
                    compact_action_index += 1;
                    let mut request = client
                        .post(format!(
                            "{server_url}/api/sessions/{}/compact",
                            chat.session_id
                        ))
                        .header("x-hope-eval-trial", &product_trial_id)
                        .header("x-hope-eval-case", &trial.case_id);
                    if let Some(token) = token.as_deref().filter(|value| !value.is_empty()) {
                        request = request.bearer_auth(token);
                    }
                    let response = match request.send().await {
                        Ok(response) => response,
                        Err(error) if error.is_timeout() => {
                            return Ok(timed_out_chat_result(
                                &client,
                                server_url,
                                token.as_deref(),
                                trial,
                                &product_trial_id,
                                "Hope scripted compact_context action",
                            )
                            .await);
                        }
                        Err(error) => {
                            return Err(error)
                                .context("calling Hope scripted compact_context action")
                        }
                    };
                    let status = response.status();
                    let bytes = response
                        .bytes()
                        .await
                        .context("reading Hope scripted compact_context response")?;
                    let response_hash = sha256_bytes(&bytes);
                    let check = if status.is_success() {
                        match serde_json::from_slice::<CompactContextResponse>(&bytes) {
                            Ok(result) => compact_context_contract_check(
                                &result,
                                expected_tier,
                                compact_action_index,
                                response_hash,
                            ),
                            Err(_) => ModelCampaignCheck {
                                id: format!(
                                    "scripted_compact_context_{compact_action_index}"
                                ),
                                passed: false,
                                blocking: true,
                                detail: "manual compaction response is not the registered camelCase contract"
                                    .to_string(),
                                metric: None,
                                artifact_hashes: vec![response_hash],
                            },
                        }
                    } else {
                        ModelCampaignCheck {
                            id: format!("scripted_compact_context_{compact_action_index}"),
                            passed: false,
                            blocking: true,
                            detail: format!(
                                "manual compaction returned HTTP {status}; body sha256:{response_hash}"
                            ),
                            metric: None,
                            artifact_hashes: vec![response_hash],
                        }
                    };
                    let passed = check.passed;
                    scripted_action_checks.push(check);
                    if !passed {
                        break;
                    }
                }
            }
        }
    }
    let completion_wait_started = Instant::now();
    if observes_trial_completion(&scenario.subject.config_profile, unlimited_budget) {
        // A durable job/Workflow may finish before its queued parent-result
        // turn starts. Keep the root identity open and observe real follow-up
        // work within the same immutable case budget. Cleanup must not cut
        // that turn off merely because the foreground HTTP reply returned.
        let remaining = (!unlimited_budget).then(|| {
            Duration::from_secs(
                planned_case
                    .timeout_seconds
                    .saturating_sub(TRIAL_CLEANUP_GRACE_SECONDS),
            )
            .saturating_sub(setup_started.elapsed())
        });
        let initial = fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id)
            .await
            .ok_or_else(|| anyhow!("Hope model-eval telemetry endpoint is unavailable"))?;
        let _ = observe_trial_completion(initial, remaining, Duration::from_millis(250), || {
            fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id)
        })
        .await;
    }
    let completion_wait_ms = elapsed_ms(completion_wait_started);
    finish_trial_for_scoring(&client, server_url, token.as_deref(), &product_trial_id).await?;
    let agent_wall_ms = elapsed_ms(agent_started);
    let response_hash = response_hashes
        .last()
        .cloned()
        .unwrap_or_else(|| sha256_bytes(b""));
    let mut telemetry = fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id)
        .await
        .ok_or_else(|| anyhow!("Hope model-eval telemetry endpoint is unavailable"))?;
    if scenario.subject.config_profile == "eval-control-plane-ready-v4" {
        // Post-turn runtime finalizers can hold a span for a few milliseconds
        // after the HTTP response. Observe their natural teardown before
        // grading; do not cancel work or invent terminal evidence here.
        telemetry = settle_response_snapshot(telemetry, Duration::from_millis(250), || {
            fetch_telemetry(&client, server_url, token.as_deref(), &product_trial_id)
        })
        .await;
    }
    let active_work_at_response =
        telemetry.trace.orphan_span_count > u64::from(telemetry.background_model_work);
    let mut checks = scripted_action_checks;
    if let Some((passed, detail)) = pre_restart_trigger_check {
        checks.push(ModelCampaignCheck {
            id: "process_restart_trigger_observed".to_string(),
            passed,
            blocking: true,
            detail,
            metric: Some(if passed { 1.0 } else { 0.0 }),
            artifact_hashes: Vec::new(),
        });
    }
    if let Some((passed, detail)) = user_event_precondition_check {
        checks.push(ModelCampaignCheck {
            id: "user_event_precondition_observed".to_string(),
            passed,
            blocking: true,
            detail,
            metric: Some(if passed { 1.0 } else { 0.0 }),
            artifact_hashes: Vec::new(),
        });
    }
    let mut benchmark_defect = false;
    for verifier in &scenario.verifiers {
        match evaluate_registered_verifier(
            verifier,
            scenario_dir,
            &workspace,
            &client,
            server_url,
            token.as_deref(),
            &chat,
            &telemetry,
            &response_hash,
            trial,
        )
        .await
        {
            Ok(check) => checks.push(check),
            Err(error) => {
                benchmark_defect = true;
                checks.push(ModelCampaignCheck {
                    id: verifier.id.clone(),
                    passed: false,
                    blocking: verifier.blocking,
                    detail: format!("verifier configuration error: {error:#}"),
                    metric: None,
                    artifact_hashes: vec![response_hash.clone()],
                });
            }
        }
    }
    let cleanup_started = Instant::now();
    let (final_telemetry, cleanup_error) =
        match cleanup_and_wait_for_trial(&client, server_url, token.as_deref(), &product_trial_id)
            .await
        {
            Ok(final_telemetry) => (final_telemetry, None),
            Err(error) => {
                let detail = format!("{error:#}");
                telemetry
                    .warnings
                    .push(format!("trial cleanup remained incomplete: {detail}"));
                (telemetry, Some(detail))
            }
        };
    let environment_cleanup_ms = elapsed_ms(cleanup_started);
    // Use the post-cleanup snapshot for complete usage/terminal metrics, while
    // retaining checks observed at the assistant response boundary. Active
    // descendants at that boundary are a false-completion signal even if the
    // Harness subsequently cancels them successfully during cleanup.
    telemetry = final_telemetry;
    telemetry.timings.environment_setup_ms = environment_setup_ms;
    telemetry.timings.environment_cleanup_ms = environment_cleanup_ms;
    telemetry.timings.environment_wait_ms = telemetry
        .timings
        .environment_wait_ms
        .saturating_add(completion_wait_ms);
    telemetry.timings.wall_ms = agent_wall_ms;
    if let Some(pre_restart) = pre_restart_telemetry {
        merge_restart_telemetry(pre_restart, &mut telemetry, restart_wait_ms);
    }
    telemetry.timings.environment_wait_ms = telemetry
        .timings
        .environment_wait_ms
        .saturating_add(scripted_wait_ms);
    let causal_critical_path = causal_critical_path_ms(&telemetry.trace, &telemetry.events);
    telemetry.timings.critical_path_ms = if causal_critical_path == 0 {
        agent_wall_ms
    } else {
        causal_critical_path.min(agent_wall_ms)
    };
    telemetry.invariants = evaluate_invariants(scenario, &telemetry);
    if active_work_at_response {
        checks.push(ModelCampaignCheck {
            id: "harness_completion_quiescent".to_string(),
            passed: false,
            blocking: true,
            detail: "assistant returned while descendant work was still active".to_string(),
            metric: None,
            artifact_hashes: Vec::new(),
        });
    }
    let fault_observed = telemetry
        .observed_signals
        .iter()
        .any(|signal| signal == "fault");
    checks.push(ModelCampaignCheck {
        id: "experiment_arm_integrity".to_string(),
        passed: if trial.fault_profile == FaultProfile::Chaos {
            fault_observed
        } else {
            !fault_observed
        },
        blocking: true,
        detail: if trial.fault_profile == FaultProfile::Chaos {
            "faulted arm must activate at least one registered fault"
        } else {
            "control arm must not activate a registered fault"
        }
        .to_string(),
        metric: Some(if fault_observed { 1.0 } else { 0.0 }),
        artifact_hashes: Vec::new(),
    });
    if let Some(profile) = trial.orchestration_profile.as_deref() {
        let passed = if profile.starts_with("single_agent") {
            telemetry.orchestration.spawned_agents == 0
        } else if profile == "team_full" {
            telemetry.orchestration.spawned_agents > 0
        } else {
            true
        };
        checks.push(ModelCampaignCheck {
            id: "comparison_arm_integrity".to_string(),
            passed,
            blocking: true,
            detail: format!(
                "orchestration profile {profile} observed {} spawned Agent(s)",
                telemetry.orchestration.spawned_agents
            ),
            metric: Some(telemetry.orchestration.spawned_agents as f64),
            artifact_hashes: Vec::new(),
        });
    }
    telemetry.milestones = merge_checks(telemetry.milestones, checks);
    telemetry.warnings.push(format!(
        "Hope session {} final turn {} response sha256:{} ({} turn(s))",
        chat.session_id,
        chat.turn_id,
        response_hash,
        response_hashes.len()
    ));
    let blocking_failed = telemetry
        .milestones
        .iter()
        .chain(&telemetry.invariants)
        .any(|check| check.blocking && !check.passed);
    let (mut outcome, mut failure_class) = if cleanup_error.is_some() {
        (
            ModelCampaignOutcome::InfraError,
            Some(CLEANUP_INCOMPLETE_FAILURE_CLASS.to_string()),
        )
    } else if request_blocked {
        (
            ModelCampaignOutcome::PolicyFailed,
            Some("request_blocked".to_string()),
        )
    } else if let Some(reason) = experiment_arm_failure(trial.fault_profile, fault_observed) {
        (
            ModelCampaignOutcome::BenchmarkDefect,
            Some(reason.to_string()),
        )
    } else if benchmark_defect {
        (
            ModelCampaignOutcome::BenchmarkDefect,
            Some("verifier_defect".to_string()),
        )
    } else if blocking_failed {
        (
            ModelCampaignOutcome::TaskFailed,
            Some("hard_verifier_failed".to_string()),
        )
    } else {
        (ModelCampaignOutcome::Passed, None)
    };
    let artifacts = match collect_declared_artifacts(&workspace, &scenario.artifacts) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            outcome = ModelCampaignOutcome::PolicyFailed;
            failure_class = Some("unsafe_artifact".to_string());
            telemetry.warnings.push(format!(
                "declared artifact rejected: {}",
                sanitize_error(&format!("{error:#}"))
            ));
            Vec::new()
        }
    };
    Ok(ObservedTrial {
        outcome,
        failure_class,
        telemetry,
        artifacts,
        error: cleanup_error,
    })
}

async fn apply_scripted_goal_revision(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    session_id: &str,
    objective: &str,
    completion_criteria: &str,
) -> Result<ModelCampaignCheck> {
    let token = token
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("scripted owner Goal revision requires owner authentication"))?;
    let safe_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    if !safe_id(session_id) {
        bail!("scripted owner action has an invalid Session identity");
    }
    let mut request = client.get(format!(
        "{server_url}/api/sessions/{session_id}/goal/latest"
    ));
    request = request.bearer_auth(token);
    let response = request
        .send()
        .await
        .context("reading current Session Goal for scripted revision")?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        bail!(
            "scripted Goal lookup returned HTTP {status}; {}",
            http_error_hint(&bytes)
        );
    }
    let before: Value = serde_json::from_slice(&bytes)?;
    let goal = before
        .get("goal")
        .ok_or_else(|| anyhow!("scripted owner revision has no current Goal"))?;
    if goal.get("sessionId").and_then(Value::as_str) != Some(session_id) {
        bail!("scripted owner revision cannot target another Session");
    }
    let goal_id = goal
        .get("id")
        .and_then(Value::as_str)
        .filter(|v| safe_id(v))
        .ok_or_else(|| anyhow!("scripted owner revision has an invalid Goal identity"))?;
    let previous = goal
        .get("revision")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("Goal revision is missing"))?;
    let expected_revision = previous
        .checked_add(1)
        .ok_or_else(|| anyhow!("Goal revision overflow"))?;
    let mut request = client
        .patch(format!("{server_url}/api/goals/{goal_id}"))
        .json(&json!({"objective":objective,"completionCriteria":completion_criteria}));
    request = request.bearer_auth(token);
    let response = request
        .send()
        .await
        .context("applying declared user Goal revision through owner API")?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        bail!(
            "scripted owner Goal revision returned HTTP {status}; {}",
            http_error_hint(&bytes)
        );
    }
    let after: Value = serde_json::from_slice(&bytes)?;
    let revision = after.pointer("/goal/revision").and_then(Value::as_i64);
    let passed = after.pointer("/goal/id").and_then(Value::as_str) == Some(goal_id)
        && after.pointer("/goal/sessionId").and_then(Value::as_str) == Some(session_id)
        && revision == Some(expected_revision)
        && after.pointer("/goal/state").and_then(Value::as_str) == Some("active");
    Ok(ModelCampaignCheck { id:"scripted_owner_goal_revision".to_string(), passed, blocking:true,
        detail:format!("owner revised current Session Goal {goal_id}: revision {previous} -> {:?}; model permissions unchanged", revision),
        metric: revision.map(|v| v as f64), artifact_hashes:vec![sha256_bytes(&bytes)] })
}

fn process_restart_phase_instruction(trigger: &str) -> &'static str {
    match trigger {
        "after_goal_checkpoint" => {
            "\n\n评测故障阶段：本轮只建立并更新真实 Goal，记录至少一条可恢复证据/检查点；不要请求 Goal 完成，也不要写最终交付物。确认状态已持久化后回复 `READY_FOR_RESTART`，Runner 将重启 Hope，随后再继续完成。"
        }
        "before_third_operation" => {
            "\n\n评测故障阶段：先创建并运行真实 Workflow，最多完成前两个 operation，写入真实 checkpoint 后暂停；不要执行第三个 operation，也不要写最终交付物。确认 checkpoint 已持久化后回复 `READY_FOR_RESTART`，Runner 将重启 Hope，随后再恢复。"
        }
        "executor_crash_after_checkpoint" => {
            "\n\n评测故障阶段：先启动真实 Subagent/Team 子任务并形成持久化父子执行记录；在至少一个子执行已开始但最终结果尚未消费时回复 `READY_FOR_RESTART`，不要写最终交付物。Runner 将立即重启 Hope，用于验证崩溃恢复和结果只消费一次。"
        }
        _ => {
            "\n\n评测故障阶段：先建立真实、可恢复的控制面状态并持久化检查点，暂不完成任务或写最终交付物；回复 `READY_FOR_RESTART` 后由 Runner 重启 Hope，再继续完成。"
        }
    }
}

fn user_event_phase_instruction(trigger: &str, profile: &str) -> &'static str {
    if profile == "eval-control-plane-ready-v9"
        && matches!(
            trigger,
            "after_first_draft" | "narrow_time_range_after_draft"
        )
    {
        return "\n\n评测用户事件阶段：本轮只创建草稿（如 drafts/research.json），不要创建最终 deliverables/result.json，也不要请求完成 Goal。若创建 Workflow，只让它真实读取并验证草稿，验证成功后正常 finish；不要为了等待用户而制造 blocked/等待审批的 Workflow 运行。Goal 保持未完成并登记草稿 checkpoint。回复 WAITING_FOR_USER_EVENT，用户将通过控制面修改 Goal 契约，随后按新的真实 revision 交付。";
    }
    match trigger {
        "after_first_draft" => {
            "\n\n评测用户事件阶段：本轮只完成第一版草稿并把当前 Goal 保持在未完成状态；不要写最终 deliverables/result.json。回复 `WAITING_FOR_USER_EVENT` 后等待用户修订，再按新目标交付。"
        }
        "pause_resume_cancel_sequence" => {
            "\n\n评测用户事件阶段：先创建并启动真实 Workflow，使至少一个节点进入运行/排队状态；不要完成 Workflow 或写最终交付物。回复 `WAITING_FOR_USER_EVENT`，随后严格处理用户的 pause、resume、cancel 指令。"
        }
        "reject_delete_approval" => {
            "\n\n评测用户事件阶段：先建立真实 Workflow 并形成需要删除审批的候选步骤，但不得执行删除，也不要完成 Workflow 或写最终交付物。回复 `WAITING_FOR_USER_EVENT`，等待用户审批决定。"
        }
        "purge_before_completion" => {
            "\n\n评测用户事件阶段：先启动一个真实后台任务并保持未完成/未注入状态；不要写最终交付物。回复 `WAITING_FOR_USER_EVENT`，等待用户发出 Incognito purge。"
        }
        "cancel_parent_during_child_tool" => {
            "\n\n评测用户事件阶段：先启动真实 Subagent/Team 子任务，使至少一个子 Agent 正在执行工具且结果尚未消费；不要写最终交付物。回复 `WAITING_FOR_USER_EVENT`，等待父任务取消。"
        }
        "narrow_time_range_after_draft" => {
            "\n\n评测用户事件阶段：先基于冻结语料形成第一版研究草稿并保持 Goal/Workflow 未完成；不要写最终 deliverables/result.json。回复 `WAITING_FOR_USER_EVENT`，等待用户收窄时间范围。"
        }
        _ => {
            "\n\n评测用户事件阶段：先建立真实且未完成的中间状态，不要写最终交付物；回复 `WAITING_FOR_USER_EVENT` 后等待用户事件再继续。"
        }
    }
}

fn process_restart_trigger_observed(
    trigger: &str,
    telemetry: &TelemetrySnapshot,
) -> (bool, String) {
    let has_signal = |signal: &str| {
        telemetry
            .observed_signals
            .iter()
            .any(|observed| observed == signal)
    };
    let has_event = |prefix: &str| {
        telemetry
            .events
            .iter()
            .any(|event| event.event.starts_with(prefix))
    };
    let passed = match trigger {
        "after_goal_checkpoint" => has_signal("goal") && has_event("goal."),
        "before_third_operation" => {
            has_signal("workflow")
                && (telemetry.orchestration.checkpoints > 0 || has_event("workflow.checkpoint"))
        }
        "executor_crash_after_checkpoint" => {
            telemetry.orchestration.spawned_agents > 0
                && (has_signal("subagent") || has_signal("team"))
                && has_event("subagent.run")
        }
        _ => false,
    };
    (
        passed,
        format!(
            "restart trigger {trigger}: signals={:?}, checkpoints={}, spawnedAgents={}",
            telemetry.observed_signals,
            telemetry.orchestration.checkpoints,
            telemetry.orchestration.spawned_agents
        ),
    )
}

fn user_event_precondition_observed(
    trigger: &str,
    telemetry: &TelemetrySnapshot,
    workspace: &Path,
) -> (bool, String) {
    let has_signal = |signal: &str| {
        telemetry
            .observed_signals
            .iter()
            .any(|observed| observed == signal)
    };
    let final_artifact_absent = !workspace.join("deliverables/result.json").exists();
    let passed = match trigger {
        "after_first_draft" => has_signal("goal") && final_artifact_absent,
        "pause_resume_cancel_sequence" | "reject_delete_approval" => {
            has_signal("workflow") && final_artifact_absent
        }
        "purge_before_completion" => {
            has_signal("async_jobs")
                && telemetry.orchestration.async_jobs > 0
                && final_artifact_absent
        }
        "cancel_parent_during_child_tool" => {
            telemetry.orchestration.spawned_agents > 0
                && (has_signal("subagent") || has_signal("team"))
                && final_artifact_absent
        }
        "narrow_time_range_after_draft" => {
            (has_signal("workflow") || has_signal("subagent")) && final_artifact_absent
        }
        _ => false,
    };
    (
        passed,
        format!(
            "user event {trigger}: signals={:?}, asyncJobs={}, spawnedAgents={}, finalArtifactAbsent={final_artifact_absent}",
            telemetry.observed_signals,
            telemetry.orchestration.async_jobs,
            telemetry.orchestration.spawned_agents,
        ),
    )
}

fn apply_remaining_restart_budget(
    context: &mut Value,
    budget: &CampaignBudget,
    before: &TelemetrySnapshot,
    restart_wait_ms: u64,
) {
    let exact_remaining = |limit: Option<u64>, used: Option<u64>, calls: u64| {
        limit.map(|limit| {
            let used = match used {
                Some(value) => value,
                None if calls == 0 => 0,
                None => limit,
            };
            limit.saturating_sub(used)
        })
    };
    let cost_remaining = budget.max_cost_usd.map(|limit| {
        let limit = (limit * 1_000_000.0).ceil() as u64;
        let used = before
            .cost
            .total_usd
            .map(|value| (value * 1_000_000.0).ceil() as u64)
            .unwrap_or_else(|| {
                if before.orchestration.model_calls == 0 {
                    0
                } else {
                    limit
                }
            });
        limit.saturating_sub(used)
    });
    context["budget"] = json!({
        "maxWallMs": budget.max_wall_seconds.map(|value| value.saturating_mul(1_000).saturating_sub(before.timings.wall_ms).saturating_sub(restart_wait_ms)),
        "maxModelCalls": budget.max_model_calls.map(|value| value.saturating_sub(before.orchestration.model_calls)),
        "maxInputTokens": exact_remaining(budget.max_input_tokens, before.tokens.input, before.orchestration.model_calls),
        "maxOutputTokens": exact_remaining(budget.max_output_tokens, before.tokens.output, before.orchestration.model_calls),
        "maxCostMicros": cost_remaining,
        "maxToolCalls": budget.max_tool_calls.map(|value| value.saturating_sub(before.tools.attempted)),
        "maxAgents": budget.max_agents.map(|value| value.saturating_sub(before.orchestration.spawned_agents.min(u64::from(u32::MAX)) as u32)),
        "maxConcurrency": budget.max_concurrency,
    });
}

fn evaluate_invariants(
    scenario: &LiveAgentScenario,
    telemetry: &TelemetrySnapshot,
) -> Vec<ModelCampaignCheck> {
    scenario
        .invariants
        .iter()
        .map(|invariant| {
            let events = &telemetry.events;
            let matching = |name: &str| {
                events
                    .iter()
                    .filter(|event| event.event == name)
                    .collect::<Vec<_>>()
            };
            let (passed, detail) = match invariant.kind {
                InvariantKind::Never => {
                    let name = invariant.event.as_deref().unwrap_or_default();
                    let count = matching(name).len();
                    (count == 0, format!("event {name} occurred {count} time(s)"))
                }
                InvariantKind::AtMostOnce | InvariantKind::ExactlyOnce => {
                    let name = invariant.event.as_deref().unwrap_or_default();
                    let selected = matching(name);
                    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
                    for event in &selected {
                        *counts.entry(event.key.as_deref().unwrap_or("__global__")).or_default() += 1;
                    }
                    let maximum = counts.values().copied().max().unwrap_or(0);
                    let passed = if invariant.kind == InvariantKind::ExactlyOnce {
                        !counts.is_empty() && counts.values().all(|count| *count == 1)
                    } else {
                        maximum <= 1
                    };
                    (
                        passed,
                        format!(
                            "event {name}: {} key(s), max multiplicity {maximum}",
                            counts.len()
                        ),
                    )
                }
                InvariantKind::Eventually => {
                    let name = invariant.event.as_deref().unwrap_or_default();
                    let count = matching(name).len();
                    (count > 0, format!("event {name} observed {count} time(s)"))
                }
                InvariantKind::EventuallyWithin => {
                    let name = invariant.event.as_deref().unwrap_or_default();
                    let within = invariant.within_ms.unwrap_or(0);
                    let selected = matching(name);
                    let passed = selected
                        .iter()
                        .any(|event| event.duration_ms <= within);
                    (
                        passed,
                        format!("event {name} must complete within {within}ms"),
                    )
                }
                InvariantKind::Before | InvariantKind::After => {
                    let first_name = invariant.first.as_deref().unwrap_or_default();
                    let then_name = invariant.then.as_deref().unwrap_or_default();
                    let first = matching(first_name)
                        .iter()
                        .map(|event| event.seq)
                        .min();
                    let then = matching(then_name)
                        .iter()
                        .map(|event| event.seq)
                        .min();
                    let passed = match (invariant.kind, first, then) {
                        (InvariantKind::Before, Some(first), Some(then)) => first < then,
                        (InvariantKind::After, Some(first), Some(then)) => first > then,
                        _ => false,
                    };
                    (
                        passed,
                        format!("event order {first_name}={first:?}, {then_name}={then:?}"),
                    )
                }
                InvariantKind::MaxConcurrent => {
                    let name = invariant.event.as_deref().unwrap_or_default();
                    let maximum = invariant.maximum.unwrap_or(0);
                    let mut active = 0u32;
                    let mut peak = 0u32;
                    for event in events.iter().filter(|event| event.event == name) {
                        if event.status == "started" {
                            active = active.saturating_add(1);
                            peak = peak.max(active);
                        } else if matches!(event.status.as_str(), "completed" | "failed" | "cancelled") {
                            active = active.saturating_sub(1);
                        }
                    }
                    (peak <= maximum, format!("event {name} peak concurrency {peak}/{maximum}"))
                }
                InvariantKind::NoOverlap => {
                    let first_name = invariant.first.as_deref().unwrap_or_default();
                    let then_name = invariant.then.as_deref().unwrap_or_default();
                    let intervals = |name: &str| {
                        let mut grouped = std::collections::BTreeMap::<
                            String,
                            (Option<u64>, Option<u64>),
                        >::new();
                        for event in events.iter().filter(|event| event.event == name) {
                            let entry = grouped
                                .entry(event.key.clone().unwrap_or_else(|| "__global__".to_string()))
                                .or_default();
                            if event.status == "started" {
                                entry.0 = Some(entry.0.map_or(event.seq, |seq| seq.min(event.seq)));
                            } else if matches!(
                                event.status.as_str(),
                                "completed" | "succeeded" | "failed" | "cancelled"
                            ) {
                                entry.1 = Some(entry.1.map_or(event.seq, |seq| seq.max(event.seq)));
                            }
                        }
                        grouped
                            .into_values()
                            .filter_map(|(start, end)| Some((start?, end?)))
                            .collect::<Vec<_>>()
                    };
                    let first = intervals(first_name);
                    let then = intervals(then_name);
                    let complete = !first.is_empty() && !then.is_empty();
                    let overlap = first.iter().any(|(left_start, left_end)| {
                        then.iter().any(|(right_start, right_end)| {
                            left_start <= right_end && right_start <= left_end
                        })
                    });
                    (
                        complete && !overlap,
                        format!(
                            "{first_name} and {then_name}: complete intervals={complete}, overlap={overlap}"
                        ),
                    )
                }
                InvariantKind::ParentChildClosed => (
                    telemetry.trace.closed && telemetry.trace.orphan_span_count == 0,
                    format!(
                        "trace closed={}, orphan spans={}",
                        telemetry.trace.closed, telemetry.trace.orphan_span_count
                    ),
                ),
            };
            ModelCampaignCheck {
                id: invariant.id.clone(),
                passed,
                blocking: invariant.blocking,
                detail,
                metric: None,
                artifact_hashes: Vec::new(),
            }
        })
        .collect()
}

async fn fetch_telemetry(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    product_trial_id: &str,
) -> Option<TelemetrySnapshot> {
    let mut request = client.get(format!(
        "{server_url}/api/eval/model/trials/{product_trial_id}"
    ));
    if let Some(token) = token.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("missing")
        .to_string();
    let bytes = response.bytes().await.ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            eprintln!(
                "Hope model-eval telemetry schema mismatch: {error}; content-type={content_type}; bytes={}; body sha256:{}",
                bytes.len(),
                sha256_bytes(&bytes)
            );
            None
        }
    }
}

async fn restart_supervised_server(client: &Client) -> Result<()> {
    let base = std::env::var(SUPERVISOR_URL_ENV)
        .with_context(|| format!("{SUPERVISOR_URL_ENV} is required"))?;
    let parsed = reqwest::Url::parse(&base).context("parsing model-eval supervisor URL")?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if parsed.scheme() != "http" || !loopback || parsed.port().is_none() {
        bail!("model-eval supervisor URL must be loopback HTTP with an explicit port");
    }
    let token = std::env::var(SUPERVISOR_TOKEN_ENV)
        .with_context(|| format!("{SUPERVISOR_TOKEN_ENV} is required"))?;
    if token.len() < 24 || token.contains(['\r', '\n']) {
        bail!("model-eval supervisor token has an invalid encoding");
    }
    let response = client
        .post(format!("{}/restart", base.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .context("requesting registered Hope process restart")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("reading model-eval supervisor response")?;
    if !status.is_success() {
        bail!(
            "model-eval supervisor returned HTTP {status}; body sha256:{}",
            sha256_bytes(&bytes)
        );
    }
    Ok(())
}

fn merge_restart_telemetry(
    before: TelemetrySnapshot,
    after: &mut TelemetrySnapshot,
    restart_wait_ms: u64,
) {
    let interrupted_work = before.active_children > 0 || before.background_model_work > 0;
    let restart_attributes = std::collections::BTreeMap::from([
        (
            "interruptedWorkAtRestart".to_string(),
            json!(interrupted_work),
        ),
        (
            "preRestartActiveChildren".to_string(),
            json!(before.active_children),
        ),
        (
            "preRestartBackgroundModelWork".to_string(),
            json!(before.background_model_work),
        ),
    ]);
    fn add_exact(target: &mut Option<u64>, previous: Option<u64>) {
        *target = match (*target, previous) {
            (Some(current), Some(previous)) => Some(current.saturating_add(previous)),
            _ => None,
        };
    }
    fn add_cost(target: &mut Option<f64>, previous: Option<f64>) {
        *target = match (*target, previous) {
            (Some(current), Some(previous)) => Some(current + previous),
            _ => None,
        };
    }
    after.budget_exhausted |= before.budget_exhausted;
    after
        .budget_exhaustion_reasons
        .extend(before.budget_exhaustion_reasons);
    after.budget_exhaustion_reasons.sort();
    after.budget_exhaustion_reasons.dedup();
    after.timings.model_active_ms = after
        .timings
        .model_active_ms
        .saturating_add(before.timings.model_active_ms);
    after.timings.tool_active_ms = after
        .timings
        .tool_active_ms
        .saturating_add(before.timings.tool_active_ms);
    after.timings.queue_wait_ms = after
        .timings
        .queue_wait_ms
        .saturating_add(before.timings.queue_wait_ms);
    after.timings.approval_wait_ms = after
        .timings
        .approval_wait_ms
        .saturating_add(before.timings.approval_wait_ms);
    after.timings.environment_wait_ms = after
        .timings
        .environment_wait_ms
        .saturating_add(before.timings.environment_wait_ms)
        .saturating_add(restart_wait_ms);
    after.timings.ttft_ms = match (before.timings.ttft_ms, after.timings.ttft_ms) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    add_exact(&mut after.tokens.input, before.tokens.input);
    add_exact(&mut after.tokens.output, before.tokens.output);
    add_exact(&mut after.tokens.cache_read, before.tokens.cache_read);
    add_exact(&mut after.tokens.cache_write, before.tokens.cache_write);
    add_exact(&mut after.tokens.reasoning, before.tokens.reasoning);
    if before.tokens.usage_source != after.tokens.usage_source {
        after.tokens.usage_source = Some("merged_process_restart".to_string());
    }
    add_cost(&mut after.cost.total_usd, before.cost.total_usd);
    add_cost(&mut after.cost.agent_usd, before.cost.agent_usd);
    add_cost(&mut after.cost.simulator_usd, before.cost.simulator_usd);
    add_cost(&mut after.cost.judge_usd, before.cost.judge_usd);
    after.cost.price_snapshot_digest = match (
        before.cost.price_snapshot_digest.as_deref(),
        after.cost.price_snapshot_digest.as_deref(),
    ) {
        (Some(left), Some(right)) if left == right => Some(right.to_string()),
        (Some(left), Some(right)) => {
            let mut values = [left, right];
            values.sort_unstable();
            Some(sha256_bytes(
                format!("{0}\0{1}", values[0], values[1]).as_bytes(),
            ))
        }
        _ => None,
    };
    after.tools.attempted = after.tools.attempted.saturating_add(before.tools.attempted);
    after.tools.logical_calls = after
        .tools
        .logical_calls
        .saturating_add(before.tools.logical_calls);
    after.tools.succeeded = after.tools.succeeded.saturating_add(before.tools.succeeded);
    after.tools.failed = after.tools.failed.saturating_add(before.tools.failed);
    after.tools.cancelled = after.tools.cancelled.saturating_add(before.tools.cancelled);
    after.tools.retries = after.tools.retries.saturating_add(before.tools.retries);
    after.tools.parse_errors = after
        .tools
        .parse_errors
        .saturating_add(before.tools.parse_errors);
    after.tools.invalid = after.tools.invalid.saturating_add(before.tools.invalid);
    after.tools.duplicate = after.tools.duplicate.saturating_add(before.tools.duplicate);
    after.tools.unused_results = after
        .tools
        .unused_results
        .saturating_add(before.tools.unused_results);
    after.tools.effective = after.tools.effective.saturating_add(before.tools.effective);
    after.orchestration.model_calls = after
        .orchestration
        .model_calls
        .saturating_add(before.orchestration.model_calls);
    after.orchestration.model_retries = after
        .orchestration
        .model_retries
        .saturating_add(before.orchestration.model_retries);
    after.orchestration.failovers = after
        .orchestration
        .failovers
        .saturating_add(before.orchestration.failovers);
    after.orchestration.loop_iterations = after
        .orchestration
        .loop_iterations
        .saturating_add(before.orchestration.loop_iterations);
    after.orchestration.replans = after
        .orchestration
        .replans
        .saturating_add(before.orchestration.replans);
    after.orchestration.checkpoints = after
        .orchestration
        .checkpoints
        .saturating_add(before.orchestration.checkpoints);
    after.orchestration.resumes = after
        .orchestration
        .resumes
        .saturating_add(before.orchestration.resumes);
    after.orchestration.spawned_agents = after
        .orchestration
        .spawned_agents
        .saturating_add(before.orchestration.spawned_agents);
    after.orchestration.max_agent_depth = after
        .orchestration
        .max_agent_depth
        .max(before.orchestration.max_agent_depth);
    after.orchestration.max_concurrency = after
        .orchestration
        .max_concurrency
        .max(before.orchestration.max_concurrency);
    after.orchestration.handoffs = after
        .orchestration
        .handoffs
        .saturating_add(before.orchestration.handoffs);
    add_exact(
        &mut after.orchestration.coordination_tokens,
        before.orchestration.coordination_tokens,
    );
    after.orchestration.child_active_ms = after
        .orchestration
        .child_active_ms
        .saturating_add(before.orchestration.child_active_ms);
    after.orchestration.async_jobs = after
        .orchestration
        .async_jobs
        .saturating_add(before.orchestration.async_jobs);
    after.orchestration.duplicate_injections = after
        .orchestration
        .duplicate_injections
        .saturating_add(before.orchestration.duplicate_injections);
    after.orchestration.orphaned_children = after
        .orchestration
        .orphaned_children
        .saturating_add(before.orchestration.orphaned_children);

    let mut events = before.events;
    let interruption_timestamp = events
        .iter()
        .map(|event| event.timestamp_ms)
        .max()
        .unwrap_or(0)
        .max(before.timings.wall_ms);
    close_restart_interrupted_spans(&mut events, interruption_timestamp);
    let timestamp_offset = interruption_timestamp.saturating_add(restart_wait_ms);
    events.push(ModelCampaignEvent {
        seq: 0,
        event: "process.restart".to_string(),
        timestamp_ms: timestamp_offset,
        span_id: format!(
            "span_{}",
            &sha256_bytes(format!("{}:restart", after.trace.trace_id).as_bytes())[..24]
        ),
        parent_span_id: after.trace.root_span_id.clone(),
        key: Some("hope-agent-server".to_string()),
        status: "completed".to_string(),
        duration_ms: restart_wait_ms,
        attributes: restart_attributes,
    });
    let post_restart_events = std::mem::take(&mut after.events);
    let renamed_spans = post_restart_events
        .iter()
        .map(|event| {
            (
                event.span_id.clone(),
                format!(
                    "span_{}",
                    &sha256_bytes(format!("post-restart:{}", event.span_id).as_bytes())[..24]
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for mut event in post_restart_events {
        event.timestamp_ms = event.timestamp_ms.saturating_add(timestamp_offset);
        event.span_id = renamed_spans
            .get(&event.span_id)
            .cloned()
            .unwrap_or(event.span_id);
        if let Some(parent) = renamed_spans.get(&event.parent_span_id) {
            event.parent_span_id.clone_from(parent);
        }
        events.push(event);
    }
    for (index, event) in events.iter_mut().enumerate() {
        event.seq = index as u64 + 1;
    }
    after.events = events;
    after.trace.span_count = 1u64.saturating_add(
        after
            .events
            .iter()
            .map(|event| event.span_id.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len() as u64,
    );
    let mut observed = before.observed_signals;
    observed.extend(after.observed_signals.clone());
    observed.sort();
    observed.dedup();
    after.observed_signals = observed;
    after.warnings.extend(before.warnings);
    after
        .warnings
        .push("telemetry merged across an authenticated Hope process restart".to_string());
    if interrupted_work || before.attribution != AttributionCompleteness::Complete {
        // SIGKILL cannot run a Provider guard's Drop. Completed calls remain
        // inspectable in the merged trace, but an interrupted child/request
        // can have unreported upstream usage; never certify a zero remainder.
        after.attribution = AttributionCompleteness::Partial;
        after.tokens = TokenMetrics::default();
        after.cost.total_usd = None;
        after.cost.agent_usd = None;
        after.warnings.push("process restart interrupted active work; unobserved Provider usage may exist in addition to known completed trace calls".to_string());
    }
}

fn close_restart_interrupted_spans(events: &mut Vec<ModelCampaignEvent>, timestamp_ms: u64) {
    let terminal_spans = events
        .iter()
        .filter(|event| event.status != "started")
        .map(|event| event.span_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let interrupted = events
        .iter()
        .filter(|event| {
            event.status == "started" && !terminal_spans.contains(event.span_id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    for mut event in interrupted {
        event.status = "cancelled".to_string();
        event.duration_ms = timestamp_ms.saturating_sub(event.timestamp_ms);
        event.timestamp_ms = timestamp_ms;
        event.attributes.clear();
        events.push(event);
    }
}

fn parse_scripted_user_flow(value: Value, max_turns: u16) -> Result<Vec<ScriptedUserStep>> {
    let schema_version = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("scripted user flow requires schemaVersion"))?;
    let steps = match schema_version {
        "scripted-user-flow.v1" => {
            let flow: ScriptedUserFlowV1 =
                serde_json::from_value(value).context("parsing scripted-user-flow.v1")?;
            if flow.schema_version != "scripted-user-flow.v1" {
                bail!("scripted user flow schema is invalid");
            }
            flow.turns
                .into_iter()
                .map(|turn| ScriptedUserStep::Message {
                    message: turn.message,
                    delay_ms: turn.delay_ms,
                })
                .collect::<Vec<_>>()
        }
        "scripted-user-flow.v2" => {
            let flow: ScriptedUserFlowV2 =
                serde_json::from_value(value).context("parsing scripted-user-flow.v2")?;
            if flow.schema_version != "scripted-user-flow.v2" {
                bail!("scripted user flow schema is invalid");
            }
            flow.steps
        }
        other => bail!("unsupported scripted user flow schema {other}"),
    };

    let message_steps = steps
        .iter()
        .filter(|step| matches!(step, ScriptedUserStep::Message { .. }))
        .count();
    if message_steps + 1 > usize::from(max_turns) {
        bail!("scripted user flow turn count is invalid");
    }
    let mut compact_actions = 0usize;
    let mut owner_goal_actions = 0usize;
    for step in &steps {
        match step {
            ScriptedUserStep::Message { message, delay_ms } => {
                if message.trim().is_empty() || message.len() > 32_000 || *delay_ms > 30_000 {
                    bail!("scripted user flow contains an invalid message step");
                }
            }
            ScriptedUserStep::UpdateGoal {
                objective,
                completion_criteria,
            } => {
                owner_goal_actions += 1;
                if objective.trim().is_empty()
                    || completion_criteria.trim().is_empty()
                    || objective.len() > 32_000
                    || completion_criteria.len() > 32_000
                {
                    bail!("scripted Goal revision requires bounded nonempty objective and completionCriteria");
                }
            }
            ScriptedUserStep::CompactContext { expected_tier } => {
                compact_actions += 1;
                if *expected_tier != 3 {
                    bail!("compact_context currently requires expectedTier=3");
                }
            }
        }
    }
    let owner_update_has_later_message = steps
        .iter()
        .position(|step| matches!(step, ScriptedUserStep::UpdateGoal { .. }))
        .is_none_or(|index| {
            steps[index + 1..]
                .iter()
                .any(|step| matches!(step, ScriptedUserStep::Message { .. }))
        });
    if owner_goal_actions > 1
        || (owner_goal_actions > 0 && message_steps == 0)
        || !owner_update_has_later_message
    {
        bail!(
            "a scripted Goal revision occurs at most once and must be followed by a user message"
        );
    }
    if compact_actions > 1 {
        bail!("scripted user flow may compact context at most once");
    }
    Ok(steps)
}

pub(crate) fn validate_scripted_user_flow(
    scenario_dir: &Path,
    scenario: &LiveAgentScenario,
) -> Result<()> {
    if !matches!(
        scenario.user_simulator.kind,
        UserSimulatorKind::ScriptedFsm | UserSimulatorKind::Replay
    ) {
        return Ok(());
    }
    let steps = load_scripted_user_steps(scenario_dir, scenario)?;
    if steps
        .iter()
        .any(|step| matches!(step, ScriptedUserStep::UpdateGoal { .. }))
        && (scenario.subject.config_profile != "eval-control-plane-ready-v9"
            || !scenario
                .capabilities
                .iter()
                .any(|value| value.eq_ignore_ascii_case("goal"))
            || !scenario.faults.iter().any(|fault| {
                fault.kind == FaultKind::UserEvent
                    && matches!(
                        fault.trigger.as_str(),
                        "after_first_draft" | "narrow_time_range_after_draft"
                    )
            }))
    {
        bail!("owner Goal revision requires a registered v9 Goal user-revision scenario");
    }
    Ok(())
}

fn load_scripted_user_steps(
    scenario_dir: &Path,
    scenario: &LiveAgentScenario,
) -> Result<Vec<ScriptedUserStep>> {
    match scenario.user_simulator.kind {
        UserSimulatorKind::None => Ok(Vec::new()),
        UserSimulatorKind::ScriptedFsm | UserSimulatorKind::Replay => {
            let relative = scenario
                .user_simulator
                .script_path
                .as_deref()
                .ok_or_else(|| anyhow!("scripted/replay user simulator requires scriptPath"))?;
            let path = resolve_contained(scenario_dir, relative)?;
            let value = ha_eval_spec::model::read_json_or_yaml(&path)?;
            parse_scripted_user_flow(value, scenario.user_simulator.max_turns)
                .with_context(|| format!("parsing user simulator {}", path.display()))
        }
        UserSimulatorKind::Llm => {
            bail!("LLM user simulator is not installed in the registered v1 Hope Harness")
        }
    }
}

async fn finish_trial_for_scoring(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    product_trial_id: &str,
) -> Result<()> {
    let mut request = client.post(format!(
        "{server_url}/api/eval/model/trials/{product_trial_id}/finish"
    ));
    if let Some(token) = token.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .context("finalizing Hope model-eval trial root")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("reading Hope model-eval finalization response")?;
    if !status.is_success() {
        bail!(
            "Hope model-eval finalization returned HTTP {status}; body sha256:{}",
            sha256_bytes(&bytes)
        );
    }
    Ok(())
}

async fn cleanup_after_failed_chat(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    trial: &PlannedModelTrial,
    product_trial_id: &str,
) -> (TelemetrySnapshot, Option<String>) {
    let before_cleanup = fetch_telemetry(client, server_url, token, product_trial_id).await;
    let Some(before_cleanup) = before_cleanup else {
        return (fallback_telemetry(trial, 0), None);
    };
    match cleanup_and_wait_for_trial(client, server_url, token, product_trial_id).await {
        Ok(telemetry) => (telemetry, None),
        Err(error) => {
            let detail = format!("{error:#}");
            let mut telemetry = before_cleanup;
            telemetry
                .warnings
                .push(format!("trial cleanup remained incomplete: {detail}"));
            (telemetry, Some(detail))
        }
    }
}

async fn cleanup_and_wait_for_trial(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    product_trial_id: &str,
) -> Result<TelemetrySnapshot> {
    let mut request = client.post(format!(
        "{server_url}/api/eval/model/trials/{product_trial_id}/cleanup"
    ));
    if let Some(token) = token.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .context("cleaning up Hope model-eval trial")?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("reading Hope model-eval cleanup response")?;
    if !status.is_success() {
        bail!(
            "Hope model-eval cleanup returned HTTP {status}; body sha256:{}",
            sha256_bytes(&bytes)
        );
    }

    let deadline =
        Instant::now() + std::time::Duration::from_secs(TRIAL_CLEANUP_SAFETY_FUSE_SECONDS);
    let mut last_snapshot = None;
    loop {
        if let Some(snapshot) = fetch_telemetry(client, server_url, token, product_trial_id).await {
            if snapshot.trace.closed && snapshot.trace.orphan_span_count == 0 {
                return Ok(snapshot);
            }
            last_snapshot = Some(snapshot);
        }
        if Instant::now() >= deadline {
            if let Some(snapshot) = last_snapshot {
                bail!(
                    "Hope model-eval cleanup did not close all trial work within {} seconds (traceClosed={}, orphanSpans={}, activeChildren={}, backgroundModelWork={})",
                    TRIAL_CLEANUP_SAFETY_FUSE_SECONDS,
                    snapshot.trace.closed,
                    snapshot.trace.orphan_span_count,
                    snapshot.active_children,
                    snapshot.background_model_work,
                );
            }
            bail!(
                "Hope model-eval cleanup did not close all trial work within {} seconds and no final telemetry snapshot was available",
                TRIAL_CLEANUP_SAFETY_FUSE_SECONDS,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn format_failure_with_cleanup(primary: String, cleanup_error: Option<String>) -> String {
    match cleanup_error {
        Some(cleanup_error) => format!("{primary}; cleanup also failed: {cleanup_error}"),
        None => primary,
    }
}

fn tool_result_digest_sequence_check(
    config: &Value,
    telemetry: &TelemetrySnapshot,
) -> Result<(bool, String)> {
    let tool_name = config
        .get("toolName")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        })
        .ok_or_else(|| anyhow!("tool_result_digest_sequence requires a safe toolName"))?;
    let expected = config
        .get("resultDigests")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("tool_result_digest_sequence requires resultDigests[]"))?;
    if expected.is_empty() || expected.len() > 128 {
        bail!("tool_result_digest_sequence requires 1..=128 result digests");
    }
    let expected = expected
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|digest| {
                    digest.len() == 64
                        && digest
                            .chars()
                            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
                })
                .ok_or_else(|| {
                    anyhow!("tool_result_digest_sequence values must be lowercase SHA-256")
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let event_name = format!("tool.{tool_name}");
    let only_target_tool = match config.get("onlyTargetTool") {
        Some(Value::Bool(value)) => *value,
        Some(_) => bail!("tool_result_digest_sequence onlyTargetTool must be boolean"),
        None => false,
    };
    let observed = telemetry
        .events
        .iter()
        .filter(|event| event.event == event_name)
        .collect::<Vec<_>>();
    let unexpected_tool_events = if only_target_tool {
        telemetry
            .events
            .iter()
            .filter(|event| event.event.starts_with("tool.") && event.event != event_name)
            .count()
    } else {
        0
    };
    let first_mismatch = (0..expected.len().max(observed.len())).find(|index| {
        let Some(expected) = expected.get(*index) else {
            return true;
        };
        let Some(actual) = observed.get(*index) else {
            return true;
        };
        actual.status != "succeeded"
            || actual
                .attributes
                .get("resultDigest")
                .and_then(Value::as_str)
                != Some(*expected)
    });
    let passed = first_mismatch.is_none() && unexpected_tool_events == 0;
    Ok((
        passed,
        if unexpected_tool_events > 0 {
            format!(
                "tool {tool_name} was not exclusive; observed {unexpected_tool_events} other tool event(s)"
            )
        } else if let Some(index) = first_mismatch {
            format!(
                "tool {tool_name} result digest sequence mismatch at index {index}; observed {}, expected {}",
                observed.len(),
                expected.len()
            )
        } else {
            format!(
                "tool {tool_name} observed the exact {}-result successful digest sequence",
                expected.len()
            )
        },
    ))
}

#[allow(clippy::too_many_arguments)]
async fn evaluate_registered_verifier(
    verifier: &ha_eval_spec::model::VerifierSpec,
    scenario_dir: &Path,
    workspace: &Path,
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    chat: &ChatResponse,
    telemetry: &TelemetrySnapshot,
    response_hash: &str,
    trial: &PlannedModelTrial,
) -> Result<ModelCampaignCheck> {
    let (passed, detail) = match (verifier.kind, verifier.handler.as_str()) {
        (VerifierKind::HttpAssertion, "hope_state_subset") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let path = config
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("hope_state_subset requires path"))?
                .replace("{sessionId}", &chat.session_id);
            ensure_allowed_owner_path(&path, &chat.session_id)?;
            let mut request = client.get(format!("{server_url}{path}"));
            if let Some(token) = token.filter(|value| !value.is_empty()) {
                request = request.bearer_auth(token);
            }
            let response = request
                .send()
                .await
                .context("calling Hope state verifier")?;
            let status = response.status();
            let bytes = response
                .bytes()
                .await
                .context("reading Hope state verifier response")?;
            if !status.is_success() {
                (
                    false,
                    format!(
                        "owner state returned HTTP {status}; body sha256:{}",
                        sha256_bytes(&bytes)
                    ),
                )
            } else {
                let value: Value = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing Hope state verifier HTTP {status}"))?;
                let config = orchestration_state_expectation(
                    &config,
                    trial.orchestration_profile.as_deref(),
                );
                let passed = owner_state_matches(config, &value, trial.fault_profile)?;
                (
                    passed,
                    if passed {
                        format!(
                            "owner state response sha256:{}",
                            sha256_bytes(&serde_json::to_vec(&value)?)
                        )
                    } else {
                        format!(
                            "owner state subset mismatch ({}); response sha256:{}",
                            terminal_state_summary(&value),
                            sha256_bytes(&serde_json::to_vec(&value)?)
                        )
                    },
                )
            }
        }
        (VerifierKind::FileAssertion, "file_exists") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let path = workspace_file(workspace, &config)?;
            match safe_workspace_regular_file(workspace, &path) {
                Ok(Some(_)) => (true, "required workspace file exists".to_string()),
                Ok(None) => (false, "required workspace file is missing".to_string()),
                Err(error) => (
                    false,
                    format!(
                        "workspace file rejected: {}",
                        sanitize_error(&error.to_string())
                    ),
                ),
            }
        }
        (VerifierKind::FileAssertion, "file_contains_all") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let path = workspace_file(workspace, &config)?;
            let required = config
                .get("contains")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("file_contains_all requires contains[]"))?;
            let required = required
                .iter()
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| anyhow!("contains values must be strings"))
                })
                .collect::<Result<Vec<_>>>()?;
            match safe_workspace_regular_file(workspace, &path) {
                Ok(Some(path)) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let missing = required
                            .into_iter()
                            .filter(|needle| !content.contains(needle))
                            .count();
                        (
                            missing == 0,
                            format!("{missing} required file marker(s) missing"),
                        )
                    }
                    Err(error) => (
                        false,
                        format!(
                            "workspace file unreadable: {}",
                            sanitize_error(&error.to_string())
                        ),
                    ),
                },
                Ok(None) => (false, "verifier target is missing".to_string()),
                Err(error) => (
                    false,
                    format!(
                        "workspace file rejected: {}",
                        sanitize_error(&error.to_string())
                    ),
                ),
            }
        }
        (VerifierKind::FileAssertion, "file_json_subset") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let path = workspace_file(workspace, &config)?;
            file_json_expectation(workspace, &path, &config, trial.fault_profile)?
        }
        (VerifierKind::GitAssertion, "git_changed_paths") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let output = Command::new("git")
                .args([
                    "-C",
                    workspace.to_string_lossy().as_ref(),
                    "diff",
                    "--name-only",
                    "--",
                ])
                .output()
                .context("running registered git_changed_paths verifier")?;
            if !output.status.success() {
                bail!("registered git verifier failed");
            }
            let changed = String::from_utf8(output.stdout)
                .context("git verifier returned non-UTF-8 paths")?
                .lines()
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<_>>();
            let required = string_set(&config, "required")?;
            let allowed = string_set(&config, "allowed")?;
            (
                required.is_subset(&changed) && changed.is_subset(&allowed),
                format!(
                    "{} changed path(s), {} required",
                    changed.len(),
                    required.len()
                ),
            )
        }
        (VerifierKind::TraceAssertion | VerifierKind::EventAssertion, "signal_observed") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let mut expected = string_set(&config, "signals")?;
            if trial
                .orchestration_profile
                .as_deref()
                .is_some_and(|profile| profile.starts_with("single_agent"))
            {
                expected.remove("subagent");
                expected.remove("team");
            }
            let observed = telemetry
                .observed_signals
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let missing = expected.difference(&observed).cloned().collect::<Vec<_>>();
            (
                missing.is_empty(),
                if missing.is_empty() {
                    "all required telemetry signals observed".to_string()
                } else {
                    format!("missing required telemetry signals: {}", missing.join(", "))
                },
            )
        }
        (
            VerifierKind::TraceAssertion | VerifierKind::EventAssertion,
            "tool_result_digest_sequence",
        ) => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            tool_result_digest_sequence_check(&config, telemetry)?
        }
        (VerifierKind::TraceAssertion, "trace_closed") => {
            let task_orphans = telemetry
                .trace
                .orphan_span_count
                .saturating_sub(u64::from(telemetry.background_model_work));
            (
                task_orphans == 0,
                format!(
                    "task orphan spans={task_orphans}, product background model work={}",
                    telemetry.background_model_work
                ),
            )
        }
        (VerifierKind::HttpAssertion, "response_non_empty") => (
            !chat.response.trim().is_empty(),
            "assistant response must be non-empty".to_string(),
        ),
        (VerifierKind::HttpAssertion, "response_contains_all") => {
            let config = read_verifier_config(verifier, scenario_dir)?;
            let contains = config
                .get("contains")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("response_contains_all requires contains[]"))?;
            let missing = contains
                .iter()
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| anyhow!("contains values must be strings"))
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|needle| !chat.response.contains(needle))
                .count();
            (
                missing == 0,
                format!("{missing} required response marker(s) missing"),
            )
        }
        (VerifierKind::HttpAssertion, "response_json_subset") => {
            let expected = read_verifier_config(verifier, scenario_dir)?;
            match serde_json::from_str::<Value>(&chat.response) {
                Ok(actual) => (
                    json_subset(&expected, &actual),
                    "assistant JSON must contain the configured subset".to_string(),
                ),
                Err(_) => (false, "assistant response is not valid JSON".to_string()),
            }
        }
        (VerifierKind::HttpAssertion, "response_json_exact") => {
            let expected = read_verifier_config(verifier, scenario_dir)?;
            match parse_semantically_exact_json_response(&chat.response) {
                Ok(actual) => (
                    json_exact(&expected, &actual),
                    "assistant JSON must exactly match the configured value".to_string(),
                ),
                Err(_) => (
                    false,
                    "assistant response is neither raw JSON nor a single JSON code fence"
                        .to_string(),
                ),
            }
        }
        (_, other) => bail!(
            "unregistered verifier handler {other} for {:?}",
            verifier.kind
        ),
    };
    let artifact_hashes = if matches!(verifier.kind, VerifierKind::HttpAssertion) {
        vec![response_hash.to_string()]
    } else {
        Vec::new()
    };
    Ok(ModelCampaignCheck {
        id: verifier.id.clone(),
        passed,
        blocking: verifier.blocking,
        detail,
        metric: None,
        artifact_hashes,
    })
}

fn terminal_state_summary(value: &Value) -> String {
    fn collect(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    if matches!(key.as_str(), "state" | "status") {
                        if let Some(state) = child.as_str().filter(|state| {
                            !state.is_empty()
                                && state.len() <= 64
                                && state
                                    .chars()
                                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
                        }) {
                            out.push(format!("{child_path}={state}"));
                        }
                    }
                    collect(child, &child_path, out);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().take(8).enumerate() {
                    collect(child, &format!("{path}[{index}]"), out);
                }
            }
            _ => {}
        }
    }
    let mut states = Vec::new();
    collect(value, "", &mut states);
    states.sort();
    states.dedup();
    if states.is_empty() {
        "no public state/status fields".to_string()
    } else {
        states.into_iter().take(16).collect::<Vec<_>>().join(", ")
    }
}

fn provision_workspace(
    scenario_dir: &Path,
    scenario: &LiveAgentScenario,
) -> Result<std::path::PathBuf> {
    let trial_home = std::env::var("HA_MODEL_EVAL_TRIAL_HOME")
        .context("HA_MODEL_EVAL_TRIAL_HOME is required in the isolated trial worker")?;
    let trial_home = std::path::PathBuf::from(trial_home);
    let workspace = trial_home.join("workspace");
    std::fs::create_dir_all(&workspace)?;
    for relative in &scenario.environment.assets {
        let source = resolve_contained(scenario_dir, relative)?;
        let metadata = std::fs::symlink_metadata(&source)
            .with_context(|| format!("reading scenario asset {}", source.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("scenario workspace assets must be regular files");
        }
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            bail!("scenario workspace asset path is unsafe");
        }
        let destination = workspace.join(relative_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &destination).with_context(|| {
            format!(
                "copying scenario asset {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    if scenario_has_ready_control_plane(scenario)
        && scenario.capabilities.iter().any(|capability| {
            matches!(
                capability.to_ascii_lowercase().as_str(),
                "subagent" | "team"
            )
        })
    {
        provision_synthetic_git_workspace(&workspace)?;
    }
    Ok(workspace)
}

fn provision_synthetic_git_workspace(workspace: &Path) -> Result<()> {
    // Only synthetic fixtures enter this repository. Suppress owner Git config,
    // templates and hooks so child worktrees have no access to owner state.
    for args in [
        vec!["init", "--quiet", "--template=", "--initial-branch=eval"],
        vec!["add", "--", "fixtures"],
        vec!["commit", "--quiet", "-m", "🦭 synthetic evaluation fixture"],
    ] {
        let mut command = std::process::Command::new("git");
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
        ] {
            command.env_remove(name);
        }
        let output = command
            .args([
                "-c",
                "core.hooksPath=",
                "-c",
                "user.name=Hope Evaluation",
                "-c",
                "user.email=eval@hope.invalid",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .current_dir(workspace)
            .output()
            .context("provisioning isolated synthetic Git workspace")?;
        if !output.status.success() {
            bail!(
                "synthetic Git workspace initialization failed; stderr sha256:{}",
                sha256_bytes(&output.stderr)
            );
        }
    }
    Ok(())
}

fn workspace_file(workspace: &Path, config: &Value) -> Result<std::path::PathBuf> {
    let relative = config
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("file verifier requires path"))?;
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("file verifier path may not escape the trial workspace");
    }
    Ok(workspace.join(relative))
}

fn safe_workspace_regular_file(
    workspace: &Path,
    candidate: &Path,
) -> Result<Option<std::path::PathBuf>> {
    let relative = candidate
        .strip_prefix(workspace)
        .map_err(|_| anyhow!("workspace artifact escapes the isolated trial workspace"))?;
    let mut ancestor = workspace.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            bail!("workspace artifact path is not contained");
        }
        ancestor.push(component);
        match std::fs::symlink_metadata(&ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("workspace artifact path contains a symlink")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    let metadata = match std::fs::symlink_metadata(candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("workspace artifact must be a regular non-symlink file");
    }
    let canonical_workspace = workspace
        .canonicalize()
        .context("canonicalizing trial workspace")?;
    let canonical_candidate = candidate
        .canonicalize()
        .with_context(|| format!("canonicalizing workspace file {}", candidate.display()))?;
    if !canonical_candidate.starts_with(&canonical_workspace) {
        bail!("workspace artifact escapes the isolated trial workspace");
    }
    Ok(Some(canonical_candidate))
}

fn collect_declared_artifacts(
    workspace: &Path,
    declared: &[String],
) -> Result<Vec<ArtifactDigest>> {
    let mut artifacts = Vec::new();
    for relative in declared {
        let config = json!({"path": relative});
        let candidate = workspace_file(workspace, &config)?;
        let Some(path) = safe_workspace_regular_file(workspace, &candidate)? else {
            continue;
        };
        artifacts.push(ArtifactDigest {
            path: relative.clone(),
            sha256: digest_file(&path)?,
        });
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn string_set(value: &Value, key: &str) -> Result<std::collections::BTreeSet<String>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("verifier requires {key}[]"))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("{key} values must be strings"))
        })
        .collect()
}

fn ensure_allowed_owner_path(path: &str, session_id: &str) -> Result<()> {
    if path.contains("..") || path.contains("://") || path.contains(['\r', '\n']) {
        bail!("state verifier path is unsafe");
    }
    let allowed = [
        format!("/api/sessions/{session_id}/goal"),
        format!("/api/sessions/{session_id}/goal/latest"),
        format!("/api/sessions/{session_id}/workflow-runs"),
        format!("/api/sessions/{session_id}/background-jobs"),
        format!("/api/subagent/runs?sessionId={session_id}"),
    ];
    if !allowed.iter().any(|allowed| allowed == path) {
        bail!("state verifier path is not in the registered owner endpoint allowlist");
    }
    Ok(())
}

fn read_verifier_config(
    verifier: &ha_eval_spec::model::VerifierSpec,
    scenario_dir: &Path,
) -> Result<Value> {
    let relative = verifier
        .config_path
        .as_deref()
        .ok_or_else(|| anyhow!("verifier {} requires configPath", verifier.id))?;
    let path = resolve_contained(scenario_dir, relative)?;
    read_json(&path)
}

fn json_subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => expected.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual| json_subset(value, actual))
        }),
        (Value::Array(expected), Value::Array(actual)) => expected
            .iter()
            .all(|value| actual.iter().any(|actual| json_subset(value, actual))),
        _ => expected == actual,
    }
}

fn experiment_arm_failure(profile: FaultProfile, fault_observed: bool) -> Option<&'static str> {
    match (profile, fault_observed) {
        (FaultProfile::Chaos, false) => Some("fault_not_activated"),
        (FaultProfile::Clean, true) => Some("unexpected_fault_activation"),
        _ => None,
    }
}

pub(crate) fn single_agent_fault_not_applicable(
    scenario: &LiveAgentScenario,
    trial: &PlannedModelTrial,
) -> bool {
    trial.fault_profile == FaultProfile::Chaos
        && trial
            .orchestration_profile
            .as_deref()
            .is_some_and(|p| p.starts_with("single_agent"))
        && scenario
            .comparison
            .as_ref()
            .is_some_and(|c| !c.single_agent_unsupported_faults.is_empty())
}

pub(crate) fn trial_prompt_path<'a>(
    scenario: &'a LiveAgentScenario,
    trial: &PlannedModelTrial,
) -> Result<&'a str> {
    if trial
        .orchestration_profile
        .as_deref()
        .is_some_and(|p| p.starts_with("single_agent"))
    {
        return scenario
            .comparison
            .as_ref()
            .and_then(|c| c.single_agent_prompt_path.as_deref())
            .ok_or_else(|| {
                anyhow!("single-Agent comparison requires a registered singleAgentPromptPath")
            });
    }
    Ok(&scenario.task.prompt_path)
}

fn orchestration_state_expectation<'a>(config: &'a Value, profile: Option<&str>) -> &'a Value {
    profile
        .and_then(|profile| config.get("expectedByOrchestrationProfile")?.get(profile))
        .unwrap_or(config)
}

fn fault_profile_expectation(config: &Value, profile: FaultProfile) -> Result<&Value> {
    let Some(profiles) = config.get("expectedByFaultProfile") else {
        return Ok(config);
    };
    if [
        "expectedSubset",
        "expectedExact",
        "anyItemSubset",
        "expectedAbsent",
        "exactFields",
    ]
    .iter()
    .any(|key| config.get(key).is_some())
    {
        bail!("verifier cannot combine shared and fault-profile expectations");
    }
    let key = match profile {
        FaultProfile::Clean => "clean",
        FaultProfile::Chaos => "chaos",
    };
    profiles
        .get(key)
        .ok_or_else(|| anyhow!("verifier has no expectation for {key}"))
}

fn file_json_expectation(
    workspace: &Path,
    path: &Path,
    config: &Value,
    profile: FaultProfile,
) -> Result<(bool, String)> {
    let expectation = fault_profile_expectation(config, profile)?;
    let absent = expectation.get("expectedAbsent").and_then(Value::as_bool) == Some(true);
    if absent
        && (expectation.get("expectedSubset").is_some() || expectation.get("exactFields").is_some())
    {
        bail!("file verifier cannot require both absence and content");
    }
    let expected = if absent {
        None
    } else {
        Some(expectation.get("expectedSubset").ok_or_else(|| {
            anyhow!("file_json_subset requires expectedSubset or expectedAbsent=true")
        })?)
    };
    match safe_workspace_regular_file(workspace, path) {
        Ok(None) => Ok((
            absent,
            if absent {
                "cancelled or purged trial has no final artifact"
            } else {
                "verifier target is missing"
            }
            .to_string(),
        )),
        Ok(Some(_)) if absent => Ok((
            false,
            "cancelled or purged trial must not publish a final artifact".to_string(),
        )),
        Ok(Some(path)) => match read_json::<Value>(&path) {
            Ok(actual) => {
                let exact = match expectation.get("exactFields") {
                    Some(fields) => fields
                        .as_object()
                        .ok_or_else(|| anyhow!("exactFields must be an object"))?
                        .iter()
                        .all(|(key, expected)| actual.get(key) == Some(expected)),
                    None => true,
                };
                Ok((
                    json_subset(
                        expected.expect("content expectation was validated"),
                        &actual,
                    ) && exact,
                    "workspace JSON must match its arm's subset and exact fields".to_string(),
                ))
            }
            Err(error) => Ok((
                false,
                format!(
                    "workspace JSON is invalid: {}",
                    sanitize_error(&error.to_string())
                ),
            )),
        },
        Err(error) => Ok((
            false,
            format!(
                "workspace file rejected: {}",
                sanitize_error(&error.to_string())
            ),
        )),
    }
}

fn expected_scripted_cancellation(
    status: StatusCode,
    bytes: &[u8],
    trigger: Option<&str>,
    precondition: bool,
) -> bool {
    precondition
        && status == StatusCode::INTERNAL_SERVER_ERROR
        && matches!(
            trigger,
            Some("cancel_parent_during_child_tool" | "purge_before_completion")
        )
        && serde_json::from_slice::<Value>(bytes)
            .ok()
            .is_some_and(|body| {
                body.get("error").and_then(Value::as_str) == Some("chat cancelled by caller")
            })
}

fn owner_state_matches(config: &Value, actual: &Value, profile: FaultProfile) -> Result<bool> {
    let expectation = fault_profile_expectation(config, profile)?;
    if let Some(expected) = expectation.get("expectedExact") {
        Ok(expected == actual)
    } else if let Some(expected) = expectation.get("expectedSubset") {
        Ok(json_subset(expected, actual))
    } else if let Some(expected) = expectation.get("anyItemSubset") {
        Ok(actual
            .as_array()
            .is_some_and(|items| items.iter().any(|item| json_subset(expected, item))))
    } else {
        bail!("hope_state_subset requires expectedSubset or anyItemSubset");
    }
}

fn json_exact(expected: &Value, actual: &Value) -> bool {
    expected == actual
}

/// Parse one semantically exact JSON document while tolerating the common
/// presentation-only Markdown wrapper produced by otherwise-correct models.
/// Prose before/after the fence, multiple fences, extra object keys and array
/// reordering remain failures; only the outer `json` fence is ignored.
fn parse_semantically_exact_json_response(response: &str) -> Result<Value> {
    let trimmed = response.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let fenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .and_then(|value| value.strip_suffix("```"))
        .ok_or_else(|| anyhow!("response is not a single JSON document"))?
        .trim();
    if fenced.contains("```") {
        bail!("response contains more than one code fence");
    }
    serde_json::from_str(fenced).context("parsing fenced assistant JSON")
}

fn fallback_telemetry(trial: &PlannedModelTrial, span_count: u64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        attribution: AttributionCompleteness::Partial,
        active_children: 0,
        background_model_work: 0,
        pending_work: 0,
        budget_exhausted: false,
        budget_exhaustion_reasons: Vec::new(),
        timings: TimingMetrics::default(),
        tokens: TokenMetrics::default(),
        cost: CostMetrics::default(),
        tools: ToolMetrics::default(),
        orchestration: OrchestrationMetrics::default(),
        trace: TraceSummary {
            trace_id: format!("trace_{}", trial.id),
            root_span_id: format!("span_{}", &trial.id[trial.id.len().saturating_sub(16)..]),
            span_count,
            orphan_span_count: 0,
            closed: true,
        },
        milestones: Vec::new(),
        invariants: Vec::new(),
        warnings: vec![
            "product telemetry endpoint unavailable; attribution is partial".to_string(),
        ],
        observed_signals: Vec::new(),
        events: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn empty_failure_result(
    trial: &PlannedModelTrial,
    attempt: u8,
    started_at: String,
    completed_at: String,
    wall_ms: u64,
    outcome: ModelCampaignOutcome,
    failure_class: &str,
    error: &str,
) -> ModelTrialResult {
    let mut telemetry = fallback_telemetry(trial, 1);
    telemetry.timings.wall_ms = wall_ms;
    telemetry.timings.critical_path_ms = wall_ms;
    ModelTrialResult {
        schema_version: TRIAL_SCHEMA_VERSION.to_string(),
        trial_id: trial.id.clone(),
        campaign_id: trial.campaign_id.clone(),
        plan_digest: trial.plan_digest.clone(),
        commit_sha: trial.commit_sha.clone(),
        app_version: trial.app_version.clone(),
        suite_id: trial.suite_id.clone(),
        suite_version: trial.suite_version.clone(),
        suite_digest: trial.suite_digest.clone(),
        case_id: trial.case_id.clone(),
        case_digest: trial.case_digest.clone(),
        trial_index: trial.trial_index,
        seed: trial.seed,
        model_role: trial.model_role.clone(),
        arm: trial.arm.clone(),
        fault_profile: trial.fault_profile,
        orchestration_profile: trial.orchestration_profile.clone(),
        model_digest: trial.model_digest.clone(),
        scenario_digest: trial.scenario_digest.clone(),
        environment_digest: trial.environment_digest.clone(),
        verifier_digest: trial.verifier_digest.clone(),
        prompt_digest: trial.prompt_digest.clone(),
        tool_schema_digest: trial.tool_schema_digest.clone(),
        runtime_config_digest: crate::model::current_runtime_config_digest(),
        attempt,
        prior_attempts: Vec::new(),
        started_at,
        completed_at,
        outcome,
        failure_class: Some(failure_class.to_string()),
        attribution: telemetry.attribution,
        timings: telemetry.timings,
        tokens: telemetry.tokens,
        cost: telemetry.cost,
        tools: telemetry.tools,
        orchestration: telemetry.orchestration,
        milestones: telemetry.milestones,
        invariants: telemetry.invariants,
        judge_checks: Vec::new(),
        trace: telemetry.trace,
        trace_events: telemetry.events,
        artifacts: Vec::new(),
        warnings: telemetry.warnings,
        error: Some(sanitize_error(error)),
    }
}

fn http_error_hint(bytes: &[u8]) -> String {
    let value = serde_json::from_slice::<Value>(bytes).ok();
    let message = value
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_str);
    message
        .map(sanitize_error)
        .unwrap_or_else(|| "non-JSON or missing error detail".to_string())
}

fn sanitize_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if [
        "bearer ",
        "sk-",
        "sk_",
        "eyj",
        "access_token",
        "refresh_token",
        "password",
        "secret",
        "api_key",
        "apikey",
        "token=",
        "authorization",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return format!(
            "[REDACTED sensitive error sha256:{}]",
            sha256_bytes(error.as_bytes())
        );
    }
    let output = ha_core::logging::redact_sensitive(&error.replace(['\r', '\n'], " "));
    ha_core::truncate_utf8(&output, 2_000).to_string()
}

fn merge_checks(
    mut left: Vec<ModelCampaignCheck>,
    right: Vec<ModelCampaignCheck>,
) -> Vec<ModelCampaignCheck> {
    for check in right {
        if let Some(existing) = left.iter_mut().find(|item| item.id == check.id) {
            *existing = check;
        } else {
            left.push(check);
        }
    }
    left.sort_by(|a, b| a.id.cmp(&b.id));
    left
}

fn dedicated_source() -> bool {
    std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
        || std::env::var("HA_MODEL_EVAL_DEDICATED_RUNNER").as_deref() == Ok("1")
}

fn scenario_workflow_mode(
    scenario: &LiveAgentScenario,
) -> Option<ha_core::workflow_mode::WorkflowMode> {
    // Owner-only autonomy must be explicitly part of the versioned fixture.
    // Preserve the historical v1 setup, including its missing-mode diagnosis.
    (scenario_has_ready_control_plane(scenario)
        && scenario.capabilities.iter().any(|capability| {
            matches!(
                capability.to_ascii_lowercase().as_str(),
                "goal" | "workflow"
            )
        }))
    .then_some(ha_core::workflow_mode::WorkflowMode::On)
}

pub(crate) fn scenario_has_ready_control_plane(scenario: &LiveAgentScenario) -> bool {
    matches!(
        scenario.subject.config_profile.as_str(),
        "eval-control-plane-ready-v2"
            | "eval-control-plane-ready-v3"
            | "eval-control-plane-ready-v4"
            | "eval-control-plane-ready-v5"
            | "eval-control-plane-ready-v6"
            | "eval-control-plane-ready-v7"
            | "eval-control-plane-ready-v8"
            | "eval-control-plane-ready-v9"
    )
}

fn scenario_initial_goal(
    scenario: &LiveAgentScenario,
    fault_profile: FaultProfile,
) -> Option<Value> {
    scenario
        .capabilities
        .iter()
        .any(|capability| capability.eq_ignore_ascii_case("goal"))
        .then(|| {
            // A fault-oriented scenario title is not the clean arm's business
            // objective. Keep historical/faulted fixtures unchanged.
            let objective = if matches!(scenario.subject.config_profile.as_str(),
                "eval-control-plane-ready-v3" | "eval-control-plane-ready-v4"
                    | "eval-control-plane-ready-v5" | "eval-control-plane-ready-v6" | "eval-control-plane-ready-v7" | "eval-control-plane-ready-v8" | "eval-control-plane-ready-v9")
                && fault_profile == FaultProfile::Clean
            {
                format!(
                    "完成 {} 的公开合成文件任务，验证原始输入 SHA-256、产物字段和条目数量，登记实际工具验证证据并据实完成 Goal；只响应实际注入的故障或用户事件。",
                    scenario.id
                )
            } else {
                scenario.task.success_summary.clone()
            };
            json!({
                "objective": objective,
                "completionCriteria": "Complete the task, record real validation evidence, evaluate the Goal, and only then request Goal completion."
            })
        })
}

async fn settle_response_snapshot<F, Fut>(
    mut telemetry: TelemetrySnapshot,
    grace: Duration,
    mut fetch: F,
) -> TelemetrySnapshot
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<TelemetrySnapshot>>,
{
    let deadline = Instant::now() + grace;
    while telemetry.trace.orphan_span_count > u64::from(telemetry.background_model_work) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(25))).await;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, fetch()).await {
            Ok(Some(snapshot)) => telemetry = snapshot,
            Ok(None) => {}
            Err(_) => break,
        }
    }
    telemetry
}

fn scripted_revision_requires_settlement(profile: &str, trigger: &str) -> bool {
    matches!(
        profile,
        "eval-control-plane-ready-v8" | "eval-control-plane-ready-v9"
    ) && matches!(
        trigger,
        "after_first_draft" | "narrow_time_range_after_draft"
    )
}

fn prior_scripted_work_settled(telemetry: &TelemetrySnapshot) -> bool {
    telemetry.trace.orphan_span_count == 0
        && telemetry.active_children == 0
        && telemetry.background_model_work == 0
        && telemetry.pending_work == 0
}

async fn observe_trial_completion<F, Fut>(
    telemetry: TelemetrySnapshot,
    remaining: Option<Duration>,
    quiet_period: Duration,
    fetch: F,
) -> TelemetrySnapshot
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<TelemetrySnapshot>>,
{
    observe_trial_completion_for_phase(telemetry, remaining, quiet_period, false, fetch).await
}

async fn observe_trial_completion_for_phase<F, Fut>(
    mut telemetry: TelemetrySnapshot,
    remaining: Option<Duration>,
    quiet_period: Duration,
    require_all_prior_work: bool,
    mut fetch: F,
) -> TelemetrySnapshot
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<TelemetrySnapshot>>,
{
    let deadline = remaining.map(|remaining| Instant::now() + remaining);
    let mut quiet_since = None;
    loop {
        if telemetry.budget_exhausted {
            return telemetry;
        }
        let now = Instant::now();
        // Unlimited evaluation must also settle product-owned model work.
        // Otherwise scoring races late review usage and the cleanup endpoint's
        // short cancellation fuse becomes an accidental model-call deadline.
        // Bounded campaigns retain their case deadline and cleanup policy.
        let background_allowance = if deadline.is_some() {
            u64::from(telemetry.background_model_work)
        } else {
            0
        };
        if telemetry.trace.orphan_span_count <= background_allowance
            && (!require_all_prior_work || prior_scripted_work_settled(&telemetry))
        {
            let start = quiet_since.get_or_insert(now);
            if now.duration_since(*start) >= quiet_period {
                return telemetry;
            }
        } else {
            quiet_since = None;
        }
        let remaining = deadline.map(|deadline| deadline.saturating_duration_since(now));
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            return telemetry;
        }
        tokio::time::sleep(remaining.map_or(Duration::from_millis(100), |remaining| {
            remaining.min(Duration::from_millis(100))
        }))
        .await;
        let observation = if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return telemetry;
            }
            match tokio::time::timeout(remaining, fetch()).await {
                Ok(snapshot) => snapshot,
                Err(_) => return telemetry,
            }
        } else {
            fetch().await
        };
        match observation {
            Some(snapshot) => telemetry = snapshot,
            // An unavailable observation cannot establish continuous quiet.
            None => quiet_since = None,
        }
    }
}

fn observes_trial_completion(config_profile: &str, unlimited_budget: bool) -> bool {
    unlimited_budget
        || matches!(
            config_profile,
            "eval-control-plane-ready-v5"
                | "eval-control-plane-ready-v6"
                | "eval-control-plane-ready-v7"
                | "eval-control-plane-ready-v8"
                | "eval-control-plane-ready-v9"
        )
}

fn evaluation_source() -> &'static str {
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        "github_actions"
    } else if std::env::var("HA_MODEL_EVAL_DEDICATED_RUNNER").as_deref() == Ok("1") {
        "dedicated_runner"
    } else {
        "local_cli"
    }
}

fn worktree_is_dirty(root: &Path) -> bool {
    std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(true)
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_or_unexpected_fault_is_a_benchmark_defect() {
        assert_eq!(
            super::experiment_arm_failure(FaultProfile::Chaos, false),
            Some("fault_not_activated")
        );
        assert_eq!(
            super::experiment_arm_failure(FaultProfile::Clean, true),
            Some("unexpected_fault_activation")
        );
        assert_eq!(
            super::experiment_arm_failure(FaultProfile::Chaos, true),
            None
        );
        assert_eq!(
            super::experiment_arm_failure(FaultProfile::Clean, false),
            None
        );
    }
    #[test]
    fn crash_does_not_certify_known_cost_when_active_work_can_lose_usage() {
        let mut before = restart_telemetry();
        before.background_model_work = 1;
        let mut after = restart_telemetry();
        super::merge_restart_telemetry(before, &mut after, 1);
        assert_eq!(after.attribution, AttributionCompleteness::Partial);
        assert_eq!(after.tokens.input, None);
        assert_eq!(after.cost.total_usd, None);
        assert!(after.events.iter().any(|e| e.event == "process.restart"
            && e.attributes.get("interruptedWorkAtRestart") == Some(&serde_json::json!(true))));
    }
    #[test]
    fn single_agent_state_requires_empty_children_without_weakening_team_state() {
        let config = serde_json::json!({
            "anyItemSubset": {"status":"completed"},
            "expectedByOrchestrationProfile": {
                "single_agent_compute_matched": {"expectedExact":[]}
            }
        });
        let solo =
            super::orchestration_state_expectation(&config, Some("single_agent_compute_matched"));
        assert!(super::owner_state_matches(
            solo,
            &serde_json::json!([]),
            ha_eval_spec::model::FaultProfile::Clean
        )
        .unwrap());
        assert!(!super::owner_state_matches(
            solo,
            &serde_json::json!([{"status":"completed"}]),
            ha_eval_spec::model::FaultProfile::Clean
        )
        .unwrap());
        assert!(!super::owner_state_matches(
            &config,
            &serde_json::json!([]),
            ha_eval_spec::model::FaultProfile::Clean
        )
        .unwrap());
        assert!(super::owner_state_matches(
            &config,
            &serde_json::json!([{"status":"completed"}]),
            ha_eval_spec::model::FaultProfile::Clean
        )
        .unwrap());
    }
    use super::*;

    #[tokio::test]
    async fn response_settling_observes_natural_teardown_and_preserves_background_usage() {
        let mut active = restart_telemetry();
        active.trace.orphan_span_count = 2;
        active.background_model_work = 1;
        let mut pending = std::collections::VecDeque::from([
            None,
            Some({
                let mut settled = restart_telemetry();
                settled.trace.orphan_span_count = 1;
                settled.background_model_work = 1;
                settled
            }),
        ]);
        let settled = settle_response_snapshot(active, Duration::from_millis(150), || {
            std::future::ready(pending.pop_front().flatten())
        })
        .await;
        assert_eq!(settled.trace.orphan_span_count, 1);
        assert_eq!(settled.background_model_work, 1);
        assert_eq!(settled.tokens.input, Some(100));
        assert!(!settled.trace.closed);
    }

    #[tokio::test]
    async fn response_settling_keeps_unfinished_children_failed_at_the_deadline() {
        let mut active = restart_telemetry();
        active.trace.orphan_span_count = 2;
        active.background_model_work = 1;
        let retained = settle_response_snapshot(active, Duration::from_millis(30), || {
            let mut pending = restart_telemetry();
            pending.trace.orphan_span_count = 2;
            pending.background_model_work = 1;
            std::future::ready(Some(pending))
        })
        .await;
        assert_eq!(retained.trace.orphan_span_count, 2);
        assert!(!retained.trace.closed);
    }

    #[tokio::test]
    async fn completion_observation_waits_for_parent_followup_and_keeps_its_usage() {
        let mut initial = restart_telemetry();
        initial.trace.orphan_span_count = 0;
        let mut snapshots = std::collections::VecDeque::from([
            {
                let mut s = restart_telemetry();
                s.trace.orphan_span_count = 1;
                s
            },
            {
                let mut s = restart_telemetry();
                s.tokens.input = Some(200);
                s
            },
        ]);
        let observed = observe_trial_completion(
            initial,
            Some(Duration::from_millis(650)),
            Duration::from_millis(150),
            || {
                let snapshot = snapshots.pop_front().unwrap_or_else(|| {
                    let mut s = restart_telemetry();
                    s.tokens.input = Some(200);
                    s
                });
                std::future::ready(Some(snapshot))
            },
        )
        .await;
        assert!(snapshots.is_empty());
        assert_eq!(observed.trace.orphan_span_count, 0);
        assert_eq!(observed.tokens.input, Some(200));
    }

    #[tokio::test]
    async fn completion_observation_preserves_busy_work_at_budget_or_deadline() {
        for exhausted in [false, true] {
            let mut initial = restart_telemetry();
            initial.trace.orphan_span_count = 1;
            initial.budget_exhausted = exhausted;
            let observed = observe_trial_completion(
                initial,
                Some(Duration::from_millis(30)),
                Duration::from_millis(10),
                || std::future::ready(None),
            )
            .await;
            assert_eq!(observed.trace.orphan_span_count, 1);
            assert_eq!(observed.budget_exhausted, exhausted);
        }
    }

    #[tokio::test]
    async fn completion_observation_unlimited_waits_for_natural_completion() {
        let mut initial = restart_telemetry();
        initial.trace.orphan_span_count = 1;
        let started = Instant::now();
        let observed = tokio::time::timeout(
            Duration::from_secs(2),
            observe_trial_completion(initial, None, Duration::from_millis(20), || {
                let mut snapshot = restart_telemetry();
                snapshot.trace.orphan_span_count =
                    u64::from(started.elapsed() < Duration::from_millis(350));
                snapshot.tokens.input = Some(900_001);
                std::future::ready(Some(snapshot))
            }),
        )
        .await
        .expect("test observation did not converge");
        assert!(started.elapsed() >= Duration::from_millis(350));
        assert_eq!(observed.trace.orphan_span_count, 0);
        assert_eq!(observed.tokens.input, Some(900_001));
        assert!(!observed.budget_exhausted);
    }

    #[tokio::test]
    async fn completion_observation_unlimited_settles_late_background_usage() {
        let mut initial = restart_telemetry();
        initial.trace.orphan_span_count = 3;
        initial.background_model_work = 3;
        let started = Instant::now();
        let observed = tokio::time::timeout(
            Duration::from_secs(2),
            observe_trial_completion(initial, None, Duration::from_millis(20), || {
                let mut snapshot = restart_telemetry();
                let pending = started.elapsed() < Duration::from_millis(350);
                snapshot.trace.orphan_span_count = if pending { 3 } else { 0 };
                snapshot.background_model_work = if pending { 3 } else { 0 };
                snapshot.tokens.input = Some(if pending { 100 } else { 6_730 });
                snapshot.tokens.output = Some(if pending { 10 } else { 4_868 });
                std::future::ready(Some(snapshot))
            }),
        )
        .await
        .expect("background settlement did not converge");
        assert!(started.elapsed() >= Duration::from_millis(350));
        assert_eq!(observed.trace.orphan_span_count, 0);
        assert_eq!(observed.background_model_work, 0);
        assert_eq!(observed.tokens.input, Some(6_730));
        assert_eq!(observed.tokens.output, Some(4_868));
    }

    #[test]
    fn completion_observation_includes_unlimited_context_profile() {
        assert!(observes_trial_completion(
            "eval-safe-context-compaction-v1",
            true
        ));
        assert!(!observes_trial_completion(
            "eval-safe-context-compaction-v1",
            false
        ));
        for profile in ["eval-control-plane-ready-v5", "eval-control-plane-ready-v6"] {
            assert!(observes_trial_completion(profile, true));
            assert!(observes_trial_completion(profile, false));
        }
    }

    #[test]
    fn queued_work_wire_contract_preserves_open_trace_and_actual_usage() {
        let mut wire = serde_json::to_value(restart_telemetry()).unwrap();
        wire["pendingWork"] = json!(2);
        wire["trace"]["orphanSpanCount"] = json!(2);
        let parsed: TelemetrySnapshot = serde_json::from_value(wire).unwrap();
        assert_eq!(parsed.pending_work, 2);
        assert_eq!(parsed.trace.orphan_span_count, 2);
        assert_eq!(parsed.tokens.input, Some(100));
    }

    #[test]
    fn clean_goal_objective_uses_public_task_instead_of_fault_title() {
        let live = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/live/scenarios");
        let mut scenario: LiveAgentScenario =
            read_json(&live.join("HA-GL-004/1.9.0/scenario.json")).unwrap();
        let historical = scenario_initial_goal(&scenario, FaultProfile::Clean).unwrap();
        assert_eq!(historical["objective"], scenario.task.success_summary);
        scenario.subject.config_profile = "eval-control-plane-ready-v3".to_string();
        let clean = scenario_initial_goal(&scenario, FaultProfile::Clean).unwrap();
        assert!(clean["objective"]
            .as_str()
            .unwrap()
            .contains("公开合成文件任务"));
        assert_ne!(clean["objective"], scenario.task.success_summary);
        assert_eq!(
            scenario_initial_goal(&scenario, FaultProfile::Chaos).unwrap()["objective"],
            historical["objective"]
        );
        assert_eq!(
            scenario_workflow_mode(&scenario),
            Some(ha_core::workflow_mode::WorkflowMode::On)
        );
    }

    #[test]
    fn synthetic_git_workspace_supports_child_worktrees_without_changing_fixture() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join("fixtures")).unwrap();
        let fixture = workspace.join("fixtures/task-state.json");
        std::fs::write(&fixture, b"{\"synthetic\":true}\n").unwrap();
        let before = std::fs::read(&fixture).unwrap();
        provision_synthetic_git_workspace(&workspace).unwrap();
        let child = root.path().join("child");
        let status = std::process::Command::new("git")
            .args(["worktree", "add", "--quiet", "--detach"])
            .arg(&child)
            .current_dir(&workspace)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read(child.join("fixtures/task-state.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read(fixture).unwrap(), before);
    }

    #[test]
    fn versioned_owner_setup_enables_workflow_for_goal_and_workflow_cases_only() {
        let live = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/live/scenarios");
        for (id, version, expected) in [
            (
                "HA-WF-002",
                "1.8.0",
                Some(ha_core::workflow_mode::WorkflowMode::On),
            ),
            (
                "HA-GL-004",
                "1.9.0",
                Some(ha_core::workflow_mode::WorkflowMode::On),
            ),
            ("HA-AJ-005", "1.9.0", None),
            ("HA-ST-002", "1.8.0", None),
        ] {
            let mut scenario: LiveAgentScenario =
                read_json(&live.join(id).join(version).join("scenario.json")).unwrap();
            assert_eq!(
                scenario_workflow_mode(&scenario),
                None,
                "historical setup: {id}"
            );
            scenario.subject.config_profile = "eval-control-plane-ready-v2".to_string();
            let mode = scenario_workflow_mode(&scenario);
            assert_eq!(mode, expected, "versioned setup: {id}");
            let request: ha_server::routes::chat::ChatRequest = serde_json::from_value(json!({
                "message": "synthetic fixture task",
                "workflowMode": mode,
            }))
            .unwrap();
            assert_eq!(
                request.workflow_mode, expected,
                "production owner wire: {id}"
            );
        }
    }

    #[test]
    fn model_binding_fixture_checks_background_and_parent_calls() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/model-call-bindings.v1.json"
        ))
        .unwrap();
        let model: ModelProfile = serde_json::from_value(fixture["expected"].clone()).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let events = case["calls"].as_array().unwrap().iter().enumerate().map(|(i, attributes)| {
                serde_json::from_value(json!({
                    "seq":i+1,"event":"model.call","timestampMs":i,"spanId":format!("span_{i}"),"parentSpanId":"root","status":"succeeded","durationMs":1,"attributes":attributes
                })).unwrap()
            }).collect::<Vec<ModelCampaignEvent>>();
            let check = model_binding_check(&model, &events);
            assert_eq!(
                check.passed,
                case["expectedMatch"].as_bool().unwrap(),
                "{}",
                case["id"]
            );
            assert!(check.blocking);
        }
    }

    fn restart_telemetry() -> TelemetrySnapshot {
        TelemetrySnapshot {
            attribution: AttributionCompleteness::Complete,
            active_children: 0,
            background_model_work: 0,
            pending_work: 0,
            budget_exhausted: false,
            budget_exhaustion_reasons: Vec::new(),
            timings: TimingMetrics {
                wall_ms: 2_000,
                ..Default::default()
            },
            tokens: TokenMetrics {
                input: Some(100),
                output: Some(25),
                ..Default::default()
            },
            cost: CostMetrics {
                total_usd: Some(0.25),
                ..Default::default()
            },
            tools: ToolMetrics {
                attempted: 4,
                logical_calls: 4,
                succeeded: 4,
                ..Default::default()
            },
            orchestration: OrchestrationMetrics {
                model_calls: 3,
                checkpoints: 1,
                spawned_agents: 1,
                ..Default::default()
            },
            trace: TraceSummary {
                trace_id: "trace_restart".to_string(),
                root_span_id: "span_restart".to_string(),
                span_count: 2,
                orphan_span_count: 0,
                closed: false,
            },
            milestones: Vec::new(),
            invariants: Vec::new(),
            warnings: Vec::new(),
            observed_signals: vec![
                "goal".to_string(),
                "workflow".to_string(),
                "subagent".to_string(),
            ],
            events: vec![ModelCampaignEvent {
                seq: 1,
                event: "goal.updated".to_string(),
                timestamp_ms: 1,
                span_id: "span_goal".to_string(),
                parent_span_id: "span_restart".to_string(),
                key: Some("goal-1".to_string()),
                status: "completed".to_string(),
                duration_ms: 1,
                attributes: std::collections::BTreeMap::new(),
            }],
        }
    }

    #[test]
    fn registered_control_expectations_are_completed_and_fault_outcomes_stay_strict() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/live/scenarios");
        for case in [
            "HA-GL-004",
            "HA-WF-004",
            "HA-WF-005",
            "HA-AJ-003",
            "HA-AJ-005",
            "HA-ST-005",
        ] {
            let config: Value =
                read_json(&root.join(case).join("1.9.0/verifiers/state.json")).unwrap();
            let field = if case == "HA-GL-004" {
                "state"
            } else if case.starts_with("HA-WF") {
                "state"
            } else {
                "status"
            };
            let completed = if case == "HA-GL-004" {
                json!({"goal":{"state":"completed"}})
            } else {
                json!([{field:"completed"}])
            };
            assert!(
                owner_state_matches(&config, &completed, FaultProfile::Clean).unwrap(),
                "{case}"
            );
            assert!(
                !owner_state_matches(&config, &completed, FaultProfile::Chaos).unwrap(),
                "{case}"
            );
            let faulted = if case == "HA-GL-004" {
                json!({"goal":{"state":"failed"}})
            } else {
                json!([config["expectedByFaultProfile"]["chaos"]["anyItemSubset"].clone()])
            };
            assert!(
                owner_state_matches(&config, &faulted, FaultProfile::Chaos).unwrap(),
                "{case}"
            );
            assert!(
                !owner_state_matches(&config, &faulted, FaultProfile::Clean).unwrap(),
                "{case}"
            );
        }
        let incomplete = json!({"expectedByFaultProfile":{"clean":{"expectedSubset":{}}}});
        assert!(owner_state_matches(&incomplete, &json!({}), FaultProfile::Chaos).is_err());
    }

    #[tokio::test]
    async fn failed_worker_recovery_reads_scoped_usage_and_cancels_existing_work() {
        use axum::{
            extract::{Path as ApiPath, State},
            http::HeaderMap,
            routing::{get, post},
            Json, Router,
        };
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        async fn snapshot(
            ApiPath(id): ApiPath<String>,
            State(done): State<Arc<AtomicBool>>,
            headers: HeaderMap,
        ) -> Result<Json<Value>, axum::http::StatusCode> {
            if id != "campaign-b-paired-trial-attempt-1"
                || headers.get("authorization").and_then(|v| v.to_str().ok())
                    != Some("Bearer synthetic-owner-token")
            {
                return Err(axum::http::StatusCode::NOT_FOUND);
            }
            let mut telemetry = restart_telemetry();
            telemetry.cost.agent_usd = Some(0.25);
            telemetry.trace.closed = done.load(Ordering::SeqCst);
            Ok(Json(serde_json::to_value(telemetry).unwrap()))
        }
        async fn cleanup(State(done): State<Arc<AtomicBool>>) -> Json<Value> {
            done.store(true, Ordering::SeqCst);
            Json(json!({"ok":true}))
        }
        let done = Arc::new(AtomicBool::new(false));
        let router = Router::new()
            .route("/api/eval/model/trials/{id}", get(snapshot))
            .route("/api/eval/model/trials/{id}/cleanup", post(cleanup))
            .with_state(done.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("http://{address}");
        assert!(recover_trial_snapshot(
            &url,
            Some("synthetic-owner-token"),
            "campaign-a-paired-trial-attempt-1"
        )
        .await
        .is_none());
        assert!(!done.load(Ordering::SeqCst));
        let (telemetry, error) = recover_trial_snapshot(
            &url,
            Some("synthetic-owner-token"),
            "campaign-b-paired-trial-attempt-1",
        )
        .await
        .unwrap();
        assert!(error.is_none());
        assert!(telemetry.trace.closed);
        assert_eq!(telemetry.orchestration.model_calls, 3);
        assert_eq!(telemetry.tokens.input, Some(100));
        assert_eq!(telemetry.tokens.output, Some(25));
        assert_eq!(telemetry.cost.agent_usd, Some(0.25));
        assert!(done.load(Ordering::SeqCst));
        server.abort();
    }

    #[test]
    fn subset_supports_nested_objects_and_unordered_arrays() {
        assert!(json_subset(
            &json!({"a": {"b": 1}, "items": [2]}),
            &json!({"items": [3, 2], "a": {"b": 1, "c": true}}),
        ));
    }

    #[test]
    fn exact_json_rejects_extra_keys_and_array_order_or_cardinality_changes() {
        let expected = json!({"items": ["a", "b"]});
        assert!(json_exact(&expected, &json!({"items": ["a", "b"]})));
        assert!(!json_exact(
            &expected,
            &json!({"items": ["a", "b"], "extra": true})
        ));
        assert!(!json_exact(&expected, &json!({"items": ["b", "a"]})));
        assert!(!json_exact(&expected, &json!({"items": ["a", "b", "b"]})));
    }

    #[test]
    fn sanitize_error_removes_bearer_and_provider_keys() {
        let value = sanitize_error("Bearer secret token=abc sk-test\nnext");
        assert!(!value.contains("secret"));
        assert!(!value.contains("abc"));
        assert!(!value.contains("sk-test"));
    }

    #[test]
    fn restart_phase_requires_the_declared_durable_trigger() {
        let telemetry = restart_telemetry();
        assert!(process_restart_trigger_observed("after_goal_checkpoint", &telemetry).0);
        assert!(process_restart_trigger_observed("before_third_operation", &telemetry).0);
        assert!(!process_restart_trigger_observed("executor_crash_after_checkpoint", &telemetry).0);
        assert!(process_restart_phase_instruction("before_third_operation").contains("checkpoint"));
    }

    #[test]
    fn restart_continuation_receives_only_the_remaining_budget() {
        let telemetry = restart_telemetry();
        let budget = CampaignBudget {
            max_wall_seconds: Some(10),
            max_model_calls: Some(5),
            max_input_tokens: Some(500),
            max_output_tokens: Some(100),
            max_cost_usd: Some(1.0),
            max_tool_calls: Some(10),
            max_agents: Some(1),
            max_concurrency: Some(4),
        };
        let mut context = json!({"budget": {}});
        apply_remaining_restart_budget(&mut context, &budget, &telemetry, 500);
        assert_eq!(context["budget"]["maxWallMs"], 7_500);
        assert_eq!(context["budget"]["maxModelCalls"], 2);
        assert_eq!(context["budget"]["maxInputTokens"], 400);
        assert_eq!(context["budget"]["maxOutputTokens"], 75);
        assert_eq!(context["budget"]["maxCostMicros"], 750_000);
        assert_eq!(context["budget"]["maxToolCalls"], 6);
        assert_eq!(context["budget"]["maxAgents"], 0);
        assert_eq!(context["budget"]["maxConcurrency"], 4);
    }

    #[test]
    fn user_event_phase_requires_unfinished_real_control_plane_state() {
        let telemetry = restart_telemetry();
        let workspace = tempfile::tempdir().unwrap();
        assert!(user_event_phase_instruction(
            "reject_delete_approval",
            "eval-control-plane-ready-v9"
        )
        .contains("不得执行删除"));
        assert!(
            user_event_precondition_observed(
                "pause_resume_cancel_sequence",
                &telemetry,
                workspace.path(),
            )
            .0
        );
        std::fs::create_dir_all(workspace.path().join("deliverables")).unwrap();
        std::fs::write(workspace.path().join("deliverables/result.json"), b"{}").unwrap();
        assert!(
            !user_event_precondition_observed(
                "pause_resume_cancel_sequence",
                &telemetry,
                workspace.path(),
            )
            .0
        );
    }

    #[test]
    fn scripted_user_flow_v1_remains_compatible_and_v2_registers_compaction() {
        let v1 = parse_scripted_user_flow(
            json!({
                "schemaVersion": "scripted-user-flow.v1",
                "turns": [{"message": "继续", "delayMs": 5}]
            }),
            2,
        )
        .unwrap();
        assert!(matches!(
            &v1[0],
            ScriptedUserStep::Message { message, delay_ms }
                if message == "继续" && *delay_ms == 5
        ));

        let v2 = parse_scripted_user_flow(
            json!({
                "schemaVersion": "scripted-user-flow.v2",
                "steps": [
                    {"action": "message", "message": "记录事实"},
                    {"action": "compact_context", "expectedTier": 3},
                    {"action": "message", "message": "复述事实"}
                ]
            }),
            3,
        )
        .unwrap();
        assert!(matches!(
            &v2[1],
            ScriptedUserStep::CompactContext { expected_tier: 3 }
        ));
        assert!(parse_scripted_user_flow(
            json!({
                "schemaVersion": "scripted-user-flow.v2",
                "steps": [{"action": "compact_context", "expectedTier": 4}]
            }),
            1,
        )
        .is_err());
    }

    #[test]
    fn compact_context_action_requires_a_real_reducing_tier_three_summary() {
        let valid = CompactContextResponse {
            tier_applied: 3,
            tokens_before: 8_000,
            tokens_after: 2_000,
            messages_affected: 8,
            description: "summarized".to_string(),
        };
        assert!(compact_context_contract_check(&valid, 3, 1, "a".repeat(64)).passed);

        let no_reduction = CompactContextResponse {
            tokens_after: valid.tokens_before,
            ..valid
        };
        assert!(!compact_context_contract_check(&no_reduction, 3, 1, "b".repeat(64)).passed);
    }

    #[test]
    fn tool_result_digest_sequence_is_exact_and_requires_success() {
        let mut telemetry = restart_telemetry();
        let expected = ["a".repeat(64), "b".repeat(64)];
        telemetry.events = expected
            .iter()
            .enumerate()
            .map(|(index, digest)| ModelCampaignEvent {
                seq: index as u64 + 1,
                event: "tool.read".to_string(),
                timestamp_ms: index as u64,
                span_id: format!("span_read_{index}"),
                parent_span_id: "span_restart".to_string(),
                key: Some(format!("call-{index}")),
                status: "succeeded".to_string(),
                duration_ms: 1,
                attributes: std::collections::BTreeMap::from([(
                    "resultDigest".to_string(),
                    Value::String(digest.clone()),
                )]),
            })
            .collect();
        let config = json!({
            "toolName": "read",
            "resultDigests": expected,
            "onlyTargetTool": true
        });
        assert!(
            tool_result_digest_sequence_check(&config, &telemetry)
                .unwrap()
                .0
        );

        telemetry.events[1].status = "failed".to_string();
        assert!(
            !tool_result_digest_sequence_check(&config, &telemetry)
                .unwrap()
                .0
        );

        telemetry.events[1].status = "succeeded".to_string();
        telemetry.events.push(ModelCampaignEvent {
            seq: 3,
            event: "tool.grep".to_string(),
            timestamp_ms: 3,
            span_id: "span_grep".to_string(),
            parent_span_id: "span_restart".to_string(),
            key: Some("call-grep".to_string()),
            status: "succeeded".to_string(),
            duration_ms: 1,
            attributes: std::collections::BTreeMap::new(),
        });
        assert!(
            !tool_result_digest_sequence_check(&config, &telemetry)
                .unwrap()
                .0
        );
    }

    #[cfg(unix)]
    #[test]
    fn workspace_artifacts_reject_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let candidate = root.path().join("result.json");
        symlink(outside.path(), &candidate).unwrap();

        assert!(safe_workspace_regular_file(root.path(), &candidate).is_err());
    }

    #[test]
    fn process_restart_merge_rewrites_the_complete_post_restart_span_graph() {
        fn snapshot(prefix: &str, wall_ms: u64) -> TelemetrySnapshot {
            let root = "span_restart_root".to_string();
            let turn = format!("span_{prefix}_turn");
            let tool = format!("span_{prefix}_tool");
            let events = vec![
                ModelCampaignEvent {
                    seq: 1,
                    event: "session.turn".to_string(),
                    timestamp_ms: 0,
                    span_id: turn.clone(),
                    parent_span_id: root.clone(),
                    key: Some(format!("session-{prefix}")),
                    status: "started".to_string(),
                    duration_ms: 0,
                    attributes: std::collections::BTreeMap::new(),
                },
                ModelCampaignEvent {
                    seq: 2,
                    event: "tool.read".to_string(),
                    timestamp_ms: 1,
                    span_id: tool,
                    parent_span_id: turn.clone(),
                    key: Some(format!("call-{prefix}")),
                    status: "succeeded".to_string(),
                    duration_ms: 1,
                    attributes: std::collections::BTreeMap::new(),
                },
                ModelCampaignEvent {
                    seq: 3,
                    event: "session.turn".to_string(),
                    timestamp_ms: 2,
                    span_id: turn,
                    parent_span_id: root.clone(),
                    key: Some(format!("session-{prefix}")),
                    status: "completed".to_string(),
                    duration_ms: 2,
                    attributes: std::collections::BTreeMap::new(),
                },
            ];
            TelemetrySnapshot {
                attribution: AttributionCompleteness::Complete,
                active_children: 0,
                background_model_work: 0,
                pending_work: 0,
                budget_exhausted: false,
                budget_exhaustion_reasons: Vec::new(),
                timings: TimingMetrics {
                    wall_ms,
                    critical_path_ms: wall_ms,
                    ..Default::default()
                },
                tokens: TokenMetrics::default(),
                cost: CostMetrics::default(),
                tools: ToolMetrics::default(),
                orchestration: OrchestrationMetrics::default(),
                trace: TraceSummary {
                    trace_id: "trace_restart".to_string(),
                    root_span_id: root,
                    span_count: 3,
                    orphan_span_count: 0,
                    closed: true,
                },
                milestones: Vec::new(),
                invariants: Vec::new(),
                warnings: Vec::new(),
                observed_signals: Vec::new(),
                events,
            }
        }
        let mut before = snapshot("before", 10);
        before.events.push(ModelCampaignEvent {
            seq: 4,
            event: "model_automation.run".to_string(),
            timestamp_ms: 5,
            span_id: "span_interrupted_automation".to_string(),
            parent_span_id: before.trace.root_span_id.clone(),
            key: Some("session-before".to_string()),
            status: "started".to_string(),
            duration_ms: 0,
            attributes: std::collections::BTreeMap::new(),
        });
        before.background_model_work = 1;
        before.trace.span_count = 4;
        before.trace.orphan_span_count = 1;
        before.trace.closed = false;
        let mut after = snapshot("after", 20);
        merge_restart_telemetry(before, &mut after, 5);

        ha_eval_spec::model::validate_model_trace(&after.trace, &after.events, "restart merge")
            .unwrap();
        let post_turn = after
            .events
            .iter()
            .find(|event| {
                event.event == "session.turn"
                    && event.key.as_deref() == Some("session-after")
                    && event.status == "started"
            })
            .unwrap();
        let post_tool = after
            .events
            .iter()
            .find(|event| event.key.as_deref() == Some("call-after"))
            .unwrap();
        assert_eq!(post_tool.parent_span_id, post_turn.span_id);
        assert!(after
            .events
            .iter()
            .any(|event| event.event == "process.restart"));
        assert!(after.events.iter().any(|event| {
            event.span_id == "span_interrupted_automation" && event.status == "cancelled"
        }));
    }
    #[test]
    fn cancelled_artifact_expectations_do_not_weaken_control() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.json");
        let config = json!({"expectedByFaultProfile": {
            "clean": {"expectedSubset": {"status":"verified"}},
            "chaos": {"expectedAbsent":true}
        }});
        assert!(
            file_json_expectation(dir.path(), &path, &config, FaultProfile::Chaos)
                .unwrap()
                .0
        );
        assert!(
            !file_json_expectation(dir.path(), &path, &config, FaultProfile::Clean)
                .unwrap()
                .0
        );
        std::fs::write(&path, r#"{"status":"verified"}"#).unwrap();
        assert!(
            file_json_expectation(dir.path(), &path, &config, FaultProfile::Clean)
                .unwrap()
                .0
        );
        assert!(
            !file_json_expectation(dir.path(), &path, &config, FaultProfile::Chaos)
                .unwrap()
                .0
        );
        let conflicting = json!({"expectedSubset": {}, "expectedByFaultProfile":{"chaos":{"expectedAbsent":true}}});
        assert!(
            file_json_expectation(dir.path(), &path, &conflicting, FaultProfile::Chaos).is_err()
        );
    }

    #[test]
    fn research_exact_citations_reject_out_of_range_or_duplicate_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.json");
        let config = json!({"expectedSubset":{"itemCount":2}, "exactFields":{"citations":["fixture:A","fixture:B"]}});
        for (citations, expected) in [
            (json!(["fixture:A", "fixture:B"]), true),
            (json!(["fixture:A", "fixture:B", "fixture:C"]), false),
            (json!(["fixture:A", "fixture:A"]), false),
            (json!(["fixture:A"]), false),
        ] {
            std::fs::write(
                &path,
                json!({"itemCount":2,"citations":citations}).to_string(),
            )
            .unwrap();
            assert_eq!(
                file_json_expectation(dir.path(), &path, &config, FaultProfile::Chaos)
                    .unwrap()
                    .0,
                expected
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn absent_artifact_rejects_dangling_and_ancestor_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let config = json!({"expectedAbsent":true});
        let path = dir.path().join("result.json");
        std::os::unix::fs::symlink(outside.path().join("missing.json"), &path).unwrap();
        assert!(
            !file_json_expectation(dir.path(), &path, &config, FaultProfile::Chaos)
                .unwrap()
                .0
        );
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escaped")).unwrap();
        assert!(
            !file_json_expectation(
                dir.path(),
                &dir.path().join("escaped/missing.json"),
                &config,
                FaultProfile::Chaos
            )
            .unwrap()
            .0
        );
    }

    #[test]
    fn expected_cancellation_never_masks_generic_provider_errors() {
        let cancelled = br#"{"error":"chat cancelled by caller"}"#;
        assert!(expected_scripted_cancellation(
            StatusCode::INTERNAL_SERVER_ERROR,
            cancelled,
            Some("cancel_parent_during_child_tool"),
            true
        ));
        for (status, body, trigger, ready) in [
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                cancelled.as_slice(),
                Some("after_first_draft"),
                true,
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                cancelled.as_slice(),
                Some("cancel_parent_during_child_tool"),
                false,
            ),
            (
                StatusCode::TOO_MANY_REQUESTS,
                cancelled.as_slice(),
                Some("cancel_parent_during_child_tool"),
                true,
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                br#"{"error":"Provider failed"}"#.as_slice(),
                Some("cancel_parent_during_child_tool"),
                true,
            ),
        ] {
            assert!(!expected_scripted_cancellation(
                status, body, trigger, ready
            ));
        }
    }
    #[test]
    fn http_error_hints_preserve_diagnostics_without_credentials_or_utf8_panics() {
        assert_eq!(
            http_error_hint(br#"{"error":"synthetic admission conflict"}"#),
            "synthetic admission conflict"
        );
        for message in [
            "Bearer synthetic-credential",
            "sk_synthetic_credential",
            "eyJsynthetic.jwt.value",
            "access_token=sensitive",
        ] {
            let value = serde_json::to_vec(&json!({"error":message})).unwrap();
            let hint = http_error_hint(&value);
            assert!(hint.starts_with("[REDACTED"));
            assert!(!hint.contains(message));
        }
        let value = serde_json::to_vec(&json!({"error":"评测错误".repeat(1000)})).unwrap();
        let hint = http_error_hint(&value);
        assert!(hint.len() <= 2000);
        assert_eq!(
            http_error_hint(b"not JSON"),
            "non-JSON or missing error detail"
        );
    }
    #[tokio::test]
    async fn draft_revision_settles_prior_work_without_waiting_for_cancel_targets() {
        assert!(scripted_revision_requires_settlement(
            "eval-control-plane-ready-v8",
            "narrow_time_range_after_draft"
        ));
        for trigger in [
            "cancel_parent_during_child_tool",
            "purge_before_completion",
            "pause_resume_cancel_sequence",
        ] {
            assert!(!scripted_revision_requires_settlement(
                "eval-control-plane-ready-v8",
                trigger
            ));
        }
        assert!(!scripted_revision_requires_settlement(
            "eval-control-plane-ready-v7",
            "narrow_time_range_after_draft"
        ));
        let mut initial = restart_telemetry();
        initial.trace.orphan_span_count = 0;
        initial.active_children = 1;
        initial.background_model_work = 1;
        initial.pending_work = 1;
        let mut released = restart_telemetry();
        released.trace.orphan_span_count = 0;
        let mut repeated = restart_telemetry();
        repeated.trace.orphan_span_count = 0;
        let mut snapshots = std::collections::VecDeque::from([released, repeated]);
        let observed = observe_trial_completion_for_phase(
            initial,
            None,
            Duration::from_millis(10),
            true,
            || std::future::ready(snapshots.pop_front()),
        )
        .await;
        assert!(prior_scripted_work_settled(&observed));
        assert!(snapshots.is_empty());
        let mut busy = restart_telemetry();
        busy.trace.orphan_span_count = 0;
        busy.pending_work = 1;
        let timed_out = observe_trial_completion_for_phase(
            busy,
            Some(Duration::from_millis(10)),
            Duration::from_millis(10),
            true,
            || std::future::ready(None),
        )
        .await;
        assert!(!prior_scripted_work_settled(&timed_out));
        assert_eq!(timed_out.pending_work, 1);
    }
    #[test]
    fn scripted_owner_goal_actions_are_bounded_typed_and_followed_by_user_input() {
        let action = json!({"action":"update_goal","objective":"revised objective","completionCriteria":"new evidence"});
        let message = json!({"action":"message","message":"continue with updated user contract"});
        let flow =
            |steps: Vec<Value>| json!({"schemaVersion":"scripted-user-flow.v2","steps":steps});
        assert!(parse_scripted_user_flow(flow(vec![action.clone(), message.clone()]), 2).is_ok());
        assert!(parse_scripted_user_flow(flow(vec![message.clone(), action.clone()]), 2).is_err());
        assert!(parse_scripted_user_flow(flow(vec![action.clone()]), 2).is_err());
        assert!(parse_scripted_user_flow(
            flow(vec![action.clone(), action.clone(), message.clone()]),
            2
        )
        .is_err());
        for patch in [
            json!({"objective":""}),
            json!({"goalId":"foreign"}),
            json!({"headers":{"Authorization":"forbidden"}}),
        ] {
            let mut invalid = action.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(parse_scripted_user_flow(flow(vec![invalid, message.clone()]), 2).is_err());
        }
    }

    #[tokio::test]
    async fn owner_goal_revision_checks_session_scope_and_actual_revision_change() {
        use axum::{
            routing::{get, patch},
            Json, Router,
        };
        use std::sync::{Arc, Mutex};
        for (read_session, goal_id, revision, expected) in [
            ("session-fixture", "goal-fixture", 2, Some(true)),
            ("session-fixture", "goal-fixture", 1, Some(false)),
            ("session-fixture", "goal-fixture", 3, Some(false)),
            ("foreign-session", "goal-fixture", 2, None),
            ("session-fixture", "bad/id", 2, None),
        ] {
            let before = json!({"goal":{"id":goal_id,"sessionId":read_session,"revision":1,"state":"blocked"}});
            let after = json!({"goal":{"id":"goal-fixture","sessionId":"session-fixture","revision":revision,"state":"active"}});
            let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
            let capture = seen.clone();
            let app=Router::new()
                .route("/api/sessions/session-fixture/goal/latest", get(move || {let body=before.clone();async move {Json(body)}}))
                .route("/api/goals/goal-fixture", patch(move |headers:axum::http::HeaderMap, Json(body):Json<Value>| {
                    let capture=capture.clone();let after=after.clone();async move {
                        assert_eq!(headers.get("authorization").unwrap(),"Bearer test-owner");
                        assert_eq!(body,json!({"objective":"revised objective","completionCriteria":"new evidence"}));
                        capture.lock().unwrap().push(body);Json(after)
                    }
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = Client::builder().no_proxy().build().unwrap();
            let result = apply_scripted_goal_revision(
                &client,
                &url,
                Some("test-owner"),
                "session-fixture",
                "revised objective",
                "new evidence",
            )
            .await;
            match expected {
                Some(value) => assert_eq!(result.unwrap().passed, value),
                None => assert!(result.is_err()),
            }
            assert_eq!(seen.lock().unwrap().len(), usize::from(expected.is_some()));
            server.abort();
            let _ = server.await;
        }
    }
}
