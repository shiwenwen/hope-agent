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
struct ScriptedUserFlow {
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
struct TelemetrySnapshot {
    attribution: AttributionCompleteness,
    #[serde(default)]
    #[allow(dead_code)]
    active_children: u32,
    #[serde(default)]
    background_model_work: u32,
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
) -> ModelTrialResult {
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let started = Instant::now();
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
            run_hope_core_scenario(root, scenario_path, scenario, planned_case, trial, model).await
        }
        other => Err(anyhow!(
            "registered adapter {other:?} has no installed harness implementation"
        )),
    };
    let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let wall_ms = elapsed_ms(started);

    match execution {
        Ok(mut observed) => {
            if observed.telemetry.timings.wall_ms == 0 {
                observed.telemetry.timings.wall_ms = wall_ms;
            }
            if observed.telemetry.timings.critical_path_ms == 0 {
                observed.telemetry.timings.critical_path_ms = wall_ms;
            }
            let exceeded = exceeded_budgets(planned_case, &observed.telemetry);
            if observed.telemetry.budget_exhausted
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
        Err(error) => empty_failure_result(
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
        ),
    }
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

async fn timed_out_chat_result(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    trial: &PlannedModelTrial,
    stage: &str,
) -> ObservedTrial {
    let telemetry = match tokio::time::timeout(
        Duration::from_secs(TRIAL_CLEANUP_GRACE_SECONDS.saturating_sub(2).max(1)),
        cleanup_after_failed_chat(client, server_url, token, trial),
    )
    .await
    {
        Ok(Ok(telemetry)) => telemetry,
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
) -> Result<ObservedTrial> {
    let server_url = std::env::var(SERVER_URL_ENV)
        .with_context(|| format!("{SERVER_URL_ENV} is required for hope_core_scenario"))?;
    let server_url = server_url.trim_end_matches('/');
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
    let setup_started = Instant::now();
    let workspace = provision_workspace(scenario_dir, scenario)?;
    let environment_setup_ms = elapsed_ms(setup_started);
    let prompt_path = resolve_contained(scenario_dir, &scenario.task.prompt_path)?;
    let mut prompt = std::fs::read_to_string(&prompt_path)
        .with_context(|| format!("reading scenario prompt {}", prompt_path.display()))?;
    if prompt.trim().is_empty() {
        bail!("scenario prompt is empty");
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

    let request_timeout_seconds = planned_case
        .timeout_seconds
        .saturating_sub(TRIAL_CLEANUP_GRACE_SECONDS)
        .max(1);
    let client = Client::builder()
        .timeout(Duration::from_secs(request_timeout_seconds))
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
        prompt.push_str(user_event_phase_instruction(trigger));
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
        "trialId": trial.id,
        "trialIndex": trial.trial_index,
        "traceId": format!("trace_{}", trial.id),
        "rootSpanId": format!("span_{}", &trial.id[trial.id.len().saturating_sub(16)..]),
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
    let initial_goal = scenario
        .capabilities
        .iter()
        .any(|capability| capability.eq_ignore_ascii_case("goal"))
        .then(|| {
            json!({
                "objective": scenario.task.success_summary,
                "completionCriteria": "Complete the task, record real validation evidence, evaluate the Goal, and only then request Goal completion."
            })
        });
    let body = json!({
        "message": prompt,
        "incognito": scenario.id == "HA-AJ-005",
        "modelOverride": format!("{}::{}", model.provider_id, model.model_id),
        "temperatureOverride": model.temperature,
        "reasoningEffort": model.reasoning_effort,
        "workingDir": workspace,
        "initialGoal": initial_goal,
        "evalContext": initial_eval_context,
    });
    let mut request = client
        .post(format!("{server_url}/api/chat"))
        .header("x-hope-eval-trial", &trial.id)
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
        return Ok(ObservedTrial {
            outcome,
            failure_class: Some(classification.to_string()),
            telemetry: cleanup_after_failed_chat(&client, server_url, token.as_deref(), trial)
                .await?,
            artifacts: Vec::new(),
            error: Some(format!(
                "Hope server returned HTTP {}; body sha256:{}",
                status,
                sha256_bytes(&bytes)
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
    if !request_blocked {
        let user_event_is_fault = scenario
            .faults
            .iter()
            .any(|fault| fault.kind == FaultKind::UserEvent);
        let mut scripted_turns =
            if trial.fault_profile == FaultProfile::Chaos || !user_event_is_fault {
                load_scripted_user_turns(scenario_dir, scenario)?
            } else {
                Vec::new()
            };
        if let Some(trigger) = user_event_trigger {
            let before = fetch_telemetry(&client, server_url, token.as_deref(), trial)
                .await
                .ok_or_else(|| anyhow!("pre-user-event Hope telemetry is unavailable"))?;
            user_event_precondition_check = Some(user_event_precondition_observed(
                trigger, &before, &workspace,
            ));
        }
        if process_restart_fault {
            let before = fetch_telemetry(&client, server_url, token.as_deref(), trial)
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
            scripted_turns.insert(
                0,
                ScriptedUserTurn {
                    message: "Hope 已按评测计划重启。请从持久化状态恢复，不要重复已经提交的副作用；检查 Goal/Workflow checkpoint，只继续尚未完成的工作并重新验证终态。".to_string(),
                    delay_ms: 0,
                },
            );
        }
        for turn in scripted_turns {
            if turn.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(turn.delay_ms)).await;
                scripted_wait_ms = scripted_wait_ms.saturating_add(turn.delay_ms);
            }
            let body = json!({
                "message": turn.message,
                "sessionId": chat.session_id.clone(),
                "modelOverride": format!("{}::{}", model.provider_id, model.model_id),
                "temperatureOverride": model.temperature,
                "reasoningEffort": model.reasoning_effort,
                "evalContext": continuation_eval_context.clone(),
            });
            let mut request = client
                .post(format!("{server_url}/api/chat"))
                .header("x-hope-eval-trial", &trial.id)
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
            if !status.is_success() {
                let telemetry =
                    cleanup_after_failed_chat(&client, server_url, token.as_deref(), trial).await?;
                return Ok(ObservedTrial {
                    outcome: if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
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
                    error: Some(format!(
                        "Hope scripted turn returned HTTP {status}; body sha256:{}",
                        sha256_bytes(&bytes)
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
    }
    finish_trial_for_scoring(&client, server_url, token.as_deref(), trial).await?;
    let agent_wall_ms = elapsed_ms(agent_started);
    let response_hash = response_hashes
        .last()
        .cloned()
        .unwrap_or_else(|| sha256_bytes(b""));
    let mut telemetry = fetch_telemetry(&client, server_url, token.as_deref(), trial)
        .await
        .ok_or_else(|| anyhow!("Hope model-eval telemetry endpoint is unavailable"))?;
    let active_work_at_response =
        telemetry.trace.orphan_span_count > u64::from(telemetry.background_model_work);
    let mut checks = Vec::new();
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
    let final_telemetry =
        cleanup_and_wait_for_trial(&client, server_url, token.as_deref(), trial).await?;
    let environment_cleanup_ms = elapsed_ms(cleanup_started);
    // Use the post-cleanup snapshot for complete usage/terminal metrics, while
    // retaining checks observed at the assistant response boundary. Active
    // descendants at that boundary are a false-completion signal even if the
    // Harness subsequently cancels them successfully during cleanup.
    telemetry = final_telemetry;
    telemetry.timings.environment_setup_ms = environment_setup_ms;
    telemetry.timings.environment_cleanup_ms = environment_cleanup_ms;
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
    let (mut outcome, mut failure_class) = if request_blocked {
        (
            ModelCampaignOutcome::PolicyFailed,
            Some("request_blocked".to_string()),
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
        error: None,
    })
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

fn user_event_phase_instruction(trigger: &str) -> &'static str {
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
    trial: &PlannedModelTrial,
) -> Option<TelemetrySnapshot> {
    let mut request = client.get(format!("{server_url}/api/eval/model/trials/{}", trial.id));
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
        attributes: std::collections::BTreeMap::new(),
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

fn load_scripted_user_turns(
    scenario_dir: &Path,
    scenario: &LiveAgentScenario,
) -> Result<Vec<ScriptedUserTurn>> {
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
            let flow: ScriptedUserFlow = serde_json::from_value(value)
                .with_context(|| format!("parsing user simulator {}", path.display()))?;
            if flow.schema_version != "scripted-user-flow.v1"
                || flow.turns.len() + 1 > usize::from(scenario.user_simulator.max_turns)
            {
                bail!("scripted user flow schema or turn count is invalid");
            }
            for turn in &flow.turns {
                if turn.message.trim().is_empty()
                    || turn.message.len() > 32_000
                    || turn.delay_ms > 30_000
                {
                    bail!("scripted user flow contains an invalid turn");
                }
            }
            Ok(flow.turns)
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
    trial: &PlannedModelTrial,
) -> Result<()> {
    let mut request = client.post(format!(
        "{server_url}/api/eval/model/trials/{}/finish",
        trial.id
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
) -> Result<TelemetrySnapshot> {
    if fetch_telemetry(client, server_url, token, trial)
        .await
        .is_none()
    {
        return Ok(fallback_telemetry(trial, 0));
    }
    cleanup_and_wait_for_trial(client, server_url, token, trial).await
}

async fn cleanup_and_wait_for_trial(
    client: &Client,
    server_url: &str,
    token: Option<&str>,
    trial: &PlannedModelTrial,
) -> Result<TelemetrySnapshot> {
    let mut request = client.post(format!(
        "{server_url}/api/eval/model/trials/{}/cleanup",
        trial.id
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

    let deadline = Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(snapshot) = fetch_telemetry(client, server_url, token, trial).await {
            if snapshot.trace.closed && snapshot.trace.orphan_span_count == 0 {
                return Ok(snapshot);
            }
        }
        if Instant::now() >= deadline {
            bail!("Hope model-eval cleanup did not close all trial work within 15 seconds");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
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
                let passed = if let Some(expected) = config.get("expectedSubset") {
                    json_subset(expected, &value)
                } else if let Some(expected) = config.get("anyItemSubset") {
                    value
                        .as_array()
                        .is_some_and(|items| items.iter().any(|item| json_subset(expected, item)))
                } else {
                    bail!("hope_state_subset requires expectedSubset or anyItemSubset");
                };
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
            let expected = config
                .get("expectedSubset")
                .ok_or_else(|| anyhow!("file_json_subset requires expectedSubset"))?;
            match safe_workspace_regular_file(workspace, &path) {
                Ok(Some(path)) => match read_json::<Value>(&path) {
                    Ok(actual) => (
                        json_subset(expected, &actual),
                        "workspace JSON must contain expected subset".to_string(),
                    ),
                    Err(error) => (
                        false,
                        format!(
                            "workspace JSON is invalid: {}",
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
            let missing = expected.difference(&observed).count();
            (
                missing == 0,
                format!("{missing} required telemetry signal(s) missing"),
            )
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
    Ok(workspace)
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

fn fallback_telemetry(trial: &PlannedModelTrial, span_count: u64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        attribution: AttributionCompleteness::Partial,
        active_children: 0,
        background_model_work: 0,
        budget_exhausted: false,
        budget_exhaustion_reasons: Vec::new(),
        timings: TimingMetrics::default(),
        tokens: TokenMetrics::default(),
        cost: CostMetrics::default(),
        tools: ToolMetrics::default(),
        orchestration: OrchestrationMetrics {
            model_calls: 1,
            ..Default::default()
        },
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

fn sanitize_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if [
        "bearer ",
        "sk-",
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
    let mut output = error.replace(['\r', '\n'], " ");
    output.truncate(2_000);
    output
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
    use super::*;

    fn restart_telemetry() -> TelemetrySnapshot {
        TelemetrySnapshot {
            attribution: AttributionCompleteness::Complete,
            active_children: 0,
            background_model_work: 0,
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
    fn subset_supports_nested_objects_and_unordered_arrays() {
        assert!(json_subset(
            &json!({"a": {"b": 1}, "items": [2]}),
            &json!({"items": [3, 2], "a": {"b": 1, "c": true}}),
        ));
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
        assert!(user_event_phase_instruction("reject_delete_approval").contains("不得执行删除"));
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
}
