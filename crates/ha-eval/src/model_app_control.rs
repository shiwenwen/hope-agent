//! JSONL control process used by the desktop Evaluation Center.
//!
//! Stdout is protocol-only. Secrets arrive on anonymous stdin, are passed only
//! to the supervised Hope process environment, and are never included in the
//! immutable plan, files, events, or command line.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use ha_eval_spec::app::{
    validate_app_control_envelope, AppControlCommand, AppControlEnvelope, AppControlEvent,
    AppControlHello, EvalAppPlan, APP_CONTROL_PROTOCOL_VERSION,
};
use ha_eval_spec::model::{reject_embedded_secrets, CampaignBudget, ModelShardResult};
use ha_eval_spec::{digest_file, read_json, stable_shard, write_json};
use rand::RngCore;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch};

const PROVIDER_SECRETS_ENV: &str = "HA_MODEL_EVAL_PROVIDER_SECRETS_B64";
const LOCAL_CODEX_OAUTH_ENV: &str = "HA_MODEL_EVAL_LOCAL_CODEX_OAUTH";
const SERVER_TOKEN_ENV: &str = "HA_MODEL_EVAL_SERVER_TOKEN";
const SUPERVISOR_TOKEN_ENV: &str = "HA_MODEL_EVAL_SUPERVISOR_TOKEN";
pub const PARENT_PID_ENV: &str = "HA_EVAL_PARENT_PID";

/// Every Sidecar child is its own process-group leader. If its parent is
/// killed abruptly (so Rust destructors cannot run), this watchdog kills the
/// whole child group instead of leaving a Hope Server or trial worker alive.
/// Windows uses kill-on-close Job Objects for the same descendant guarantee.
pub fn install_parent_watchdog_from_env() -> Result<()> {
    let Some(value) = std::env::var_os(PARENT_PID_ENV) else {
        return Ok(());
    };
    let expected_parent = value
        .to_str()
        .ok_or_else(|| anyhow!("evaluation parent PID is not valid UTF-8"))?
        .parse::<u32>()
        .context("parsing evaluation parent PID")?;
    if expected_parent == 0 || expected_parent == std::process::id() {
        bail!("evaluation parent PID is invalid");
    }
    #[cfg(unix)]
    std::thread::Builder::new()
        .name("eval-parent-watchdog".to_string())
        .spawn(move || loop {
            let current_parent = unsafe { libc::getppid() } as u32;
            // Some macOS process launch paths can transiently report a
            // different direct parent even while the registered owner is
            // alive. Treat that as reparenting only after the exact owner PID
            // is no longer reachable; otherwise a healthy trial process can
            // kill its own process group during startup.
            let expected_parent_alive =
                unsafe { libc::kill(expected_parent as libc::pid_t, 0) } == 0;
            if current_parent != expected_parent && !expected_parent_alive {
                unsafe {
                    libc::kill(0, libc::SIGKILL);
                    libc::_exit(137);
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        })
        .context("starting evaluation parent watchdog")?;
    Ok(())
}

pub async fn run(root: &Path) -> Result<()> {
    let hello = build_hello(root)?;
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AppControlEvent>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        let mut seq = 0u64;
        while let Some(event) = events_rx.recv().await {
            seq = seq.saturating_add(1);
            let envelope = AppControlEnvelope {
                protocol_version: APP_CONTROL_PROTOCOL_VERSION.to_string(),
                campaign_id: event_campaign_id(&event),
                seq,
                timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                payload: event,
            };
            let mut line =
                serde_json::to_vec(&envelope).context("serializing App control event")?;
            line.push(b'\n');
            stdout
                .write_all(&line)
                .await
                .context("writing App control event")?;
            stdout.flush().await.context("flushing App control event")?;
        }
        Ok::<(), anyhow::Error>(())
    });
    events_tx
        .send(AppControlEvent::Hello {
            hello: hello.clone(),
        })
        .map_err(|_| anyhow!("App control output closed"))?;

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut acknowledged = false;
    let mut last_command_seq = 0u64;
    let mut active: Option<ActiveRun> = None;
    while let Some(line) = lines
        .next_line()
        .await
        .context("reading App control command")?
    {
        if line.len() > 4 * 1024 * 1024 {
            send_error(
                &events_tx,
                None,
                "command_too_large",
                "App control command exceeds 4 MiB",
                false,
            );
            continue;
        }
        let envelope = match serde_json::from_str::<AppControlEnvelope<AppControlCommand>>(&line) {
            Ok(envelope)
                if validate_app_control_envelope(&envelope).is_ok()
                    && DateTime::parse_from_rfc3339(&envelope.timestamp).is_ok()
                    && envelope.campaign_id.as_deref() == envelope.payload.correlation_id()
                    && envelope.seq == last_command_seq.saturating_add(1) =>
            {
                envelope
            }
            Err(_) => {
                send_error(
                    &events_tx,
                    None,
                    "invalid_command",
                    "Invalid App control command",
                    true,
                );
                continue;
            }
            _ => {
                send_error(
                    &events_tx,
                    None,
                    "invalid_envelope",
                    "Invalid or out-of-order App control envelope",
                    false,
                );
                continue;
            }
        };
        last_command_seq = envelope.seq;
        let command = envelope.payload;
        if active.as_ref().is_some_and(|run| run.task.is_finished()) {
            if let Some(run) = active.take() {
                let _ = run.task.await;
            }
        }
        match command {
            AppControlCommand::HelloAck {
                protocol_version,
                product_version,
                asset_root_digest,
            } => {
                if acknowledged {
                    send_error(
                        &events_tx,
                        None,
                        "duplicate_handshake",
                        "Handshake was already acknowledged",
                        false,
                    );
                    continue;
                }
                if protocol_version != hello.protocol_version
                    || product_version != hello.product_version
                    || asset_root_digest != hello.asset_root_digest
                {
                    send_error(
                        &events_tx,
                        None,
                        "version_mismatch",
                        "App, Sidecar, or evaluation assets do not match",
                        false,
                    );
                    break;
                }
                acknowledged = true;
                let _ = events_tx.send(AppControlEvent::Ready);
            }
            _ if !acknowledged => {
                send_error(
                    &events_tx,
                    None,
                    "handshake_required",
                    "hello_ack is required before other commands",
                    false,
                );
            }
            AppControlCommand::ListProfiles => match super::model::list_app_profiles(root) {
                Ok(profiles) => {
                    let _ = events_tx.send(AppControlEvent::Profiles { profiles });
                }
                Err(error) => send_error(
                    &events_tx,
                    None,
                    "profile_catalog_failed",
                    &safe_error(&error),
                    true,
                ),
            },
            AppControlCommand::ListCatalog => match super::model::list_app_catalog(root) {
                Ok(suites) => {
                    let _ = events_tx.send(AppControlEvent::Catalog { suites });
                }
                Err(error) => send_error(
                    &events_tx,
                    None,
                    "catalog_failed",
                    &safe_error(&error),
                    true,
                ),
            },
            AppControlCommand::Preview {
                request,
                resolved_models,
                reference,
                dirty,
                app_version,
                runtime_environment,
            } => match super::model::build_app_plan(
                root,
                &request,
                &resolved_models,
                &reference,
                dirty,
                &app_version,
                runtime_environment,
            ) {
                Ok(plan) => {
                    let _ = events_tx.send(AppControlEvent::Preview { plan });
                }
                Err(error) => {
                    send_error(&events_tx, None, "plan_rejected", &safe_error(&error), true)
                }
            },
            AppControlCommand::Start {
                request,
                resolved_models,
                reference,
                dirty,
                app_version,
                runtime_environment,
                product_binary,
                product_binary_digest,
                output_root,
                config,
                provider_secrets_b64,
            } => {
                if active.is_some() {
                    send_error(
                        &events_tx,
                        None,
                        "experiment_active",
                        "Only one local experiment may run at a time",
                        true,
                    );
                    continue;
                }
                let plan = match super::model::build_app_plan(
                    root,
                    &request,
                    &resolved_models,
                    &reference,
                    dirty,
                    &app_version,
                    runtime_environment,
                ) {
                    Ok(plan) => plan,
                    Err(error) => {
                        send_error(&events_tx, None, "plan_rejected", &safe_error(&error), true);
                        continue;
                    }
                };
                let id = plan.experiment_id.clone();
                if let Err(error) = validate_start_payload(
                    root,
                    &hello,
                    &plan,
                    Path::new(&product_binary),
                    &product_binary_digest,
                    &config,
                    &provider_secrets_b64,
                ) {
                    send_error(
                        &events_tx,
                        Some(id),
                        "start_rejected",
                        &safe_error(&error),
                        true,
                    );
                    continue;
                }
                let (cancel_tx, cancel_rx) = watch::channel(false);
                let run_root = root.to_path_buf();
                let event_output = events_tx.clone();
                let experiment_id = plan.experiment_id.clone();
                let plan_digest = plan.plan_digest.clone();
                let max_wall_seconds = plan.campaign_budget.max_wall_seconds;
                let deadline_cancel = cancel_tx.clone();
                let _ = events_tx.send(AppControlEvent::Started {
                    experiment_id: experiment_id.clone(),
                    plan_digest,
                });
                let task = tokio::spawn(async move {
                    let run = run_experiment(
                        &run_root,
                        plan,
                        PathBuf::from(product_binary),
                        PathBuf::from(output_root),
                        config,
                        provider_secrets_b64,
                        cancel_rx,
                        &event_output,
                    );
                    let result = match max_wall_seconds {
                        Some(seconds) => {
                            tokio::pin!(run);
                            tokio::select! {
                                result = run.as_mut() => result,
                                _ = tokio::time::sleep(Duration::from_secs(seconds)) => {
                                    // Do not drop run_experiment at the deadline: doing so skips
                                    // its shard and Supervisor shutdown path and can orphan the
                                    // isolated Hope Server on macOS. Ask the same cancellation
                                    // path used by the user-facing Cancel action to unwind first.
                                    let _ = deadline_cancel.send(true);
                                    match tokio::time::timeout(Duration::from_secs(20), run.as_mut()).await {
                                        Ok(Ok(_)) => Err(anyhow!("experiment wall-clock budget exhausted")),
                                        Ok(Err(error)) => Err(anyhow!(
                                            "experiment wall-clock budget exhausted; cleanup failed: {error:#}"
                                        )),
                                        Err(_) => Err(anyhow!(
                                            "experiment wall-clock budget exhausted; cleanup timed out"
                                        )),
                                    }
                                }
                            }
                        }
                        None => run.await,
                    };
                    match result {
                        Ok(RunCompletion::Completed(paths)) => {
                            let _ = event_output.send(AppControlEvent::Completed {
                                experiment_id,
                                evidence_paths: paths,
                            });
                        }
                        Ok(RunCompletion::Cancelled) => {
                            let _ = event_output.send(AppControlEvent::Cancelled { experiment_id });
                        }
                        Err(error) => send_error(
                            &event_output,
                            Some(experiment_id),
                            "experiment_failed",
                            &safe_error(&error),
                            true,
                        ),
                    }
                });
                active = Some(ActiveRun {
                    experiment_id: plan_id_for_active(&id),
                    cancel: cancel_tx,
                    task,
                });
            }
            AppControlCommand::Cancel { experiment_id } => {
                let Some(run) = &active else {
                    send_error(
                        &events_tx,
                        Some(experiment_id),
                        "not_running",
                        "No local experiment is running",
                        true,
                    );
                    continue;
                };
                if run.experiment_id != experiment_id {
                    send_error(
                        &events_tx,
                        Some(experiment_id),
                        "experiment_mismatch",
                        "A different experiment is active",
                        true,
                    );
                    continue;
                }
                let _ = run.cancel.send(true);
            }
            AppControlCommand::Shutdown => {
                if let Some(run) = active.take() {
                    let _ = run.cancel.send(true);
                    let mut task = run.task;
                    if tokio::time::timeout(Duration::from_secs(12), &mut task)
                        .await
                        .is_err()
                    {
                        task.abort();
                    }
                }
                let _ = events_tx.send(AppControlEvent::Bye);
                break;
            }
        }
    }
    if let Some(run) = active {
        let _ = run.cancel.send(true);
        let mut task = run.task;
        if tokio::time::timeout(Duration::from_secs(12), &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
    }
    drop(events_tx);
    writer.await.context("joining App control writer")??;
    Ok(())
}

struct ActiveRun {
    experiment_id: String,
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

fn plan_id_for_active(id: &str) -> String {
    id.to_string()
}

fn build_hello(root: &Path) -> Result<AppControlHello> {
    let executable = std::env::current_exe().context("resolving evaluation Sidecar")?;
    Ok(AppControlHello {
        protocol_version: APP_CONTROL_PROTOCOL_VERSION.to_string(),
        product_version: env!("CARGO_PKG_VERSION").to_string(),
        runner_digest: digest_file(&executable)?,
        asset_root_digest: super::model::live_asset_root_digest(root)?,
        version_lock_digest: digest_file(&root.join("evals/live/version-lock.json"))?,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        adapters: vec![
            "hope_core_server".to_string(),
            "coding_live".to_string(),
            "domain_live".to_string(),
        ],
    })
}

fn validate_start_payload(
    root: &Path,
    hello: &AppControlHello,
    plan: &EvalAppPlan,
    product_binary: &Path,
    product_binary_digest: &str,
    config: &serde_json::Value,
    provider_secrets_b64: &str,
) -> Result<()> {
    reject_embedded_secrets(config, "$.config")?;
    if provider_secrets_b64.len() > 1_000_000 {
        bail!("Provider credential bundle has an invalid size");
    }
    if !provider_secrets_b64.is_empty() {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(provider_secrets_b64)
            .context("Provider credential bundle is not valid base64")?;
        let secrets: std::collections::BTreeMap<String, String> =
            serde_json::from_slice(&decoded).context("Provider credential bundle is not a map")?;
        let allowed = plan
            .campaigns
            .iter()
            .map(|campaign| campaign.model.provider_id.as_str())
            .collect::<BTreeSet<_>>();
        if secrets.len() > allowed.len()
            || secrets.iter().any(|(provider_id, secret)| {
                !allowed.contains(provider_id.as_str())
                    || secret.trim().is_empty()
                    || secret.len() > 64 * 1024
                    || secret.contains(['\0', '\r', '\n'])
            })
        {
            bail!("Provider credential bundle does not match the selected model Providers");
        }
    }
    if plan.app_version != hello.product_version
        || plan.runtime_environment.runner_binary_digest != hello.runner_digest
        || plan.runtime_environment.product_binary_digest != product_binary_digest
    {
        bail!("App plan binary identity does not match the selected product/Sidecar");
    }
    validate_product_binary(
        root,
        product_binary,
        product_binary_digest,
        &plan.app_version,
    )
}

fn validate_product_binary(
    root: &Path,
    product_binary: &Path,
    expected_digest: &str,
    expected_version: &str,
) -> Result<()> {
    let binary = product_binary
        .canonicalize()
        .with_context(|| format!("canonicalizing product binary {}", product_binary.display()))?;
    let metadata = fs::symlink_metadata(&binary)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("Product binary must be a regular non-symlink file");
    }
    let name = binary
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "hope-agent"
            | "hope-agent.exe"
            | "hope-agent-server"
            | "hope-agent-server.exe"
            | "hope agent"
    ) {
        bail!("Product binary is not a registered Hope executable");
    }
    let sidecar = std::env::current_exe()?.canonicalize()?;
    if !binary.starts_with(root)
        && binary.parent() != sidecar.parent()
        && !same_macos_app_bundle(&binary, &sidecar)
    {
        bail!("Product binary is outside the checkout/installed Hope bundle");
    }
    if digest_file(&binary)? != expected_digest {
        bail!("Product binary digest does not match the App-resolved digest");
    }
    let output = Command::new(&binary)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .context("querying Hope product version")?;
    let version_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success()
        || !version_text
            .split_whitespace()
            .any(|part| part == expected_version)
    {
        bail!("Hope product and Sidecar versions differ");
    }
    validate_macos_signing_identity(&binary, &sidecar)
}

#[cfg(target_os = "macos")]
fn same_macos_app_bundle(left: &Path, right: &Path) -> bool {
    fn bundle(path: &Path) -> Option<PathBuf> {
        path.ancestors()
            .find(|part| part.extension().and_then(|value| value.to_str()) == Some("app"))
            .map(Path::to_path_buf)
    }
    bundle(left).is_some_and(|left_bundle| Some(left_bundle) == bundle(right))
}

#[cfg(not(target_os = "macos"))]
fn same_macos_app_bundle(_left: &Path, _right: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn validate_macos_signing_identity(product: &Path, sidecar: &Path) -> Result<()> {
    fn team(path: &Path) -> Result<Option<String>> {
        let output = Command::new("/usr/bin/codesign")
            .args(["-dv", "--verbose=4"])
            .arg(path)
            .output()
            .context("inspecting code-signing identity")?;
        if !output.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&output.stderr);
        Ok(text
            .lines()
            .find_map(|line| line.strip_prefix("TeamIdentifier=").map(str::to_string)))
    }
    match (team(product)?, team(sidecar)?) {
        (Some(product), Some(sidecar)) if product == sidecar => Ok(()),
        (None, None) => Ok(()),
        _ => bail!("Hope product and evaluation Sidecar signing identities differ"),
    }
}

#[cfg(not(target_os = "macos"))]
fn validate_macos_signing_identity(_product: &Path, _sidecar: &Path) -> Result<()> {
    Ok(())
}

enum RunCompletion {
    Completed(Vec<String>),
    Cancelled,
}

struct AppShardJob {
    suite_id: String,
    network_policy: ha_eval_spec::model::NetworkPolicy,
    shard_index: u16,
    shard_total: u16,
    output: PathBuf,
    trial_ids: Vec<String>,
}

struct AppShardCompletion {
    output: PathBuf,
    status: Option<ExitStatus>,
}

#[derive(Clone, Copy)]
struct BudgetObservation {
    model_calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    tool_calls: u64,
    cost_usd: f64,
}

fn emit_budget_warnings(
    events: &mpsc::UnboundedSender<AppControlEvent>,
    experiment_id: &str,
    budget: &CampaignBudget,
    observed: BudgetObservation,
    emitted: &mut BTreeSet<&'static str>,
) {
    for (dimension, value, limit) in [
        (
            "model_calls",
            observed.model_calls as f64,
            budget.max_model_calls.map(|value| value as f64),
        ),
        (
            "input_tokens",
            observed.input_tokens as f64,
            budget.max_input_tokens.map(|value| value as f64),
        ),
        (
            "output_tokens",
            observed.output_tokens as f64,
            budget.max_output_tokens.map(|value| value as f64),
        ),
        (
            "tool_calls",
            observed.tool_calls as f64,
            budget.max_tool_calls.map(|value| value as f64),
        ),
        ("cost_usd", observed.cost_usd, budget.max_cost_usd),
    ] {
        let Some(limit) = limit.filter(|limit| *limit > 0.0) else {
            continue;
        };
        let ratio = value / limit;
        if ratio >= 0.8 && emitted.insert(dimension) {
            let _ = events.send(AppControlEvent::BudgetWarning {
                experiment_id: experiment_id.to_string(),
                dimension: dimension.to_string(),
                observed: value,
                limit,
                ratio,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_experiment(
    root: &Path,
    plan: EvalAppPlan,
    product_binary: PathBuf,
    output_root: PathBuf,
    config: serde_json::Value,
    provider_secrets_b64: String,
    mut cancel: watch::Receiver<bool>,
    events: &mpsc::UnboundedSender<AppControlEvent>,
) -> Result<RunCompletion> {
    let experiment_root = prepare_experiment_root(&output_root, &plan.experiment_id)?;
    let app_plan_path = experiment_root.join("eval-app-plan.v1.json");
    write_json(&app_plan_path, &plan)?;
    let data_dir = experiment_root.join("runtime/data");
    let home_dir = experiment_root.join("runtime/home");
    let workspace = experiment_root.join("runtime/workspace");
    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&home_dir)?;
    fs::create_dir_all(&workspace)?;
    let _runtime_cleanup = RuntimeCleanup(experiment_root.join("runtime"));
    write_json(&data_dir.join("config.json"), &config)?;
    let runtime_config_digest = digest_file(&data_dir.join("config.json"))?;
    let server_bind = reserve_loopback_address()?;
    let supervisor_bind = reserve_loopback_address()?;
    let server_token = random_token();
    let supervisor_token = random_token();
    let executable = std::env::current_exe()?;
    let mut supervisor_command = Command::new(&executable);
    supervisor_command
        .arg("--root")
        .arg(root)
        .arg("model")
        .arg("_supervise-server")
        .arg("--server-bin")
        .arg(&product_binary)
        .arg("--bind")
        .arg(&server_bind)
        .arg("--control-bind")
        .arg(&supervisor_bind)
        .env(SERVER_TOKEN_ENV, &server_token)
        .env(SUPERVISOR_TOKEN_ENV, &supervisor_token)
        .env("HA_DATA_DIR", &data_dir)
        .env("HOME", &home_dir)
        .env("USERPROFILE", &home_dir)
        .env("HA_MODEL_EVAL_WORKSPACE", &workspace)
        // App-control evidence is permanently local_diagnostic. It may carry a
        // freshly resolved Codex access token, but never OAuth refresh state.
        // Protected CLI/GitHub supervisors do not receive this opt-in.
        .env(LOCAL_CODEX_OAUTH_ENV, "1")
        // Registered scenarios run in a disposable synthetic workspace with
        // explicit user consent. There is no interactive approval surface in
        // the supervised headless process, so registered tool calls must use
        // the same eval-only auto-approval mode as the CLI smoke harness.
        .env("HA_SERVER_AUTO_APPROVE_TOOLS", "1")
        .env(PARENT_PID_ENV, std::process::id().to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null());
    if !provider_secrets_b64.is_empty() {
        supervisor_command.env(PROVIDER_SECRETS_ENV, provider_secrets_b64);
    }
    let mut supervisor = ChildTree::spawn(supervisor_command)?;
    let server_url = format!("http://{server_bind}");
    if wait_server_ready(&server_url, &mut supervisor, &mut cancel).await? {
        supervisor.terminate(Duration::from_secs(10)).await;
        return Ok(RunCompletion::Cancelled);
    }

    let total = plan
        .campaigns
        .iter()
        .flat_map(|campaign| &campaign.resolved_plan.suites)
        .map(|suite| u32::from(suite.shards) + 1)
        .sum::<u32>();
    let mut completed = 0u32;
    let total_trials = u32::try_from(
        plan.campaigns
            .iter()
            .map(|campaign| campaign.resolved_plan.trials.len())
            .sum::<usize>(),
    )?;
    let mut completed_trials = 0u32;
    let mut observed_model_calls = 0u64;
    let mut observed_input_tokens = 0u64;
    let mut observed_output_tokens = 0u64;
    let mut observed_tool_calls = 0u64;
    let mut observed_cost_usd = 0.0f64;
    let mut budget_warnings = BTreeSet::new();
    let mut evidence_paths = Vec::new();
    let max_parallel_trials = plan
        .campaign_budget
        .max_concurrency
        .unwrap_or(1)
        .clamp(1, ha_eval_spec::app::APP_MAX_CONCURRENCY) as usize;
    for campaign in &plan.campaigns {
        let campaign_root = experiment_root
            .join("campaigns")
            .join(&campaign.campaign_id);
        fs::create_dir_all(campaign_root.join("shards"))?;
        let child_plan_path = campaign_root.join("model-campaign-plan.v1.json");
        write_json(&child_plan_path, &campaign.resolved_plan)?;
        let mut pending_shards = VecDeque::new();
        for suite in &campaign.resolved_plan.suites {
            for shard in 1..=suite.shards {
                let shard_path = campaign_root
                    .join("shards")
                    .join(format!("{}-{shard}.json", suite.id));
                let trial_ids = campaign
                    .resolved_plan
                    .trials
                    .iter()
                    .filter(|trial| {
                        trial.suite_id == suite.id
                            && stable_shard(&trial.id, suite.shards) == shard - 1
                    })
                    .map(|trial| trial.id.clone())
                    .collect::<Vec<_>>();
                pending_shards.push_back(AppShardJob {
                    suite_id: suite.id.clone(),
                    network_policy: suite.network_policy,
                    shard_index: shard,
                    shard_total: suite.shards,
                    output: shard_path,
                    trial_ids,
                });
            }
        }
        let mut running = tokio::task::JoinSet::new();
        let mut shard_paths = Vec::new();
        while !pending_shards.is_empty() || !running.is_empty() {
            while running.len() < max_parallel_trials {
                let Some(job) = pending_shards.pop_front() else {
                    break;
                };
                if *cancel.borrow() {
                    running.abort_all();
                    while running.join_next().await.is_some() {}
                    supervisor.terminate(Duration::from_secs(10)).await;
                    return Ok(RunCompletion::Cancelled);
                }
                for trial_id in &job.trial_ids {
                    let _ = events.send(AppControlEvent::TrialStarted {
                        experiment_id: plan.experiment_id.clone(),
                        campaign_id: campaign.campaign_id.clone(),
                        trial_id: trial_id.clone(),
                        completed: completed_trials,
                        total: total_trials,
                    });
                }
                let mut command = Command::new(&executable);
                command
                    .arg("--root")
                    .arg(root)
                    .arg("model")
                    .arg("run")
                    .arg("--plan")
                    .arg(&child_plan_path)
                    .arg("--suite")
                    .arg(&job.suite_id)
                    .arg("--shard")
                    .arg(format!("{}/{}", job.shard_index, job.shard_total))
                    .arg("--output")
                    .arg(&job.output)
                    .arg("--confirm-model-costs")
                    .env("HA_MODEL_EVAL_APP_CONTROL", "1")
                    .env("HA_MODEL_EVAL_APP_PLAN", &app_plan_path)
                    .env("HA_MODEL_EVAL_SOURCE", "local_app")
                    .env("HA_MODEL_EVAL_SERVER_URL", &server_url)
                    .env("HA_MODEL_EVAL_SERVER_TOKEN", &server_token)
                    .env(
                        "HA_MODEL_EVAL_SUPERVISOR_URL",
                        format!("http://{supervisor_bind}"),
                    )
                    .env("HA_MODEL_EVAL_SUPERVISOR_TOKEN", &supervisor_token)
                    .env(
                        "HA_MODEL_EVAL_NETWORK_POLICY",
                        network_policy_name(job.network_policy),
                    )
                    .env(
                        "HA_MODEL_EVAL_RUNTIME_CONFIG_DIGEST",
                        &runtime_config_digest,
                    )
                    .env("HA_DATA_DIR", &data_dir)
                    .env("HOME", &home_dir)
                    .env("USERPROFILE", &home_dir)
                    .env("HA_MODEL_EVAL_WORKSPACE", &workspace)
                    .env(PARENT_PID_ENV, std::process::id().to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .stdin(Stdio::null());
                let mut job_cancel = cancel.clone();
                running.spawn(async move {
                    let status = run_cancellable(command, &mut job_cancel).await?;
                    Ok::<_, anyhow::Error>(AppShardCompletion {
                        output: job.output,
                        status,
                    })
                });
            }

            let completion = match running.join_next().await {
                Some(Ok(Ok(completion))) => completion,
                Some(Ok(Err(error))) => {
                    running.abort_all();
                    while running.join_next().await.is_some() {}
                    supervisor.terminate(Duration::from_secs(10)).await;
                    return Err(error);
                }
                Some(Err(error)) => {
                    running.abort_all();
                    while running.join_next().await.is_some() {}
                    supervisor.terminate(Duration::from_secs(10)).await;
                    return Err(anyhow!("model suite shard task failed: {error}"));
                }
                None => break,
            };
            let Some(status) = completion.status else {
                running.abort_all();
                while running.join_next().await.is_some() {}
                supervisor.terminate(Duration::from_secs(10)).await;
                return Ok(RunCompletion::Cancelled);
            };
            if !status.success() {
                running.abort_all();
                while running.join_next().await.is_some() {}
                supervisor.terminate(Duration::from_secs(10)).await;
                bail!("model suite shard failed before producing evidence (status={status})");
            }
            let shard_result: ModelShardResult = read_json(&completion.output)?;
            for trial in &shard_result.trials {
                completed_trials = completed_trials.saturating_add(1);
                observed_model_calls =
                    observed_model_calls.saturating_add(trial.orchestration.model_calls);
                observed_input_tokens =
                    observed_input_tokens.saturating_add(trial.tokens.input.unwrap_or(0));
                observed_output_tokens =
                    observed_output_tokens.saturating_add(trial.tokens.output.unwrap_or(0));
                observed_tool_calls = observed_tool_calls.saturating_add(trial.tools.attempted);
                observed_cost_usd += trial.cost.total_usd.unwrap_or(0.0);
                let _ = events.send(AppControlEvent::TrialCompleted {
                    experiment_id: plan.experiment_id.clone(),
                    campaign_id: campaign.campaign_id.clone(),
                    trial_id: trial.trial_id.clone(),
                    completed: completed_trials,
                    total: total_trials,
                    outcome: trial.outcome,
                    wall_ms: trial.timings.wall_ms,
                    input_tokens: trial.tokens.input,
                    output_tokens: trial.tokens.output,
                    cost_usd: trial.cost.total_usd,
                    model_calls: trial.orchestration.model_calls,
                    tool_calls: trial.tools.attempted,
                    suite_id: trial.suite_id.clone(),
                    case_id: trial.case_id.clone(),
                    arm: trial.arm.clone(),
                    attempt: trial.attempt,
                    failure_class: trial.failure_class.clone(),
                });
            }
            emit_budget_warnings(
                events,
                &plan.experiment_id,
                &plan.campaign_budget,
                BudgetObservation {
                    model_calls: observed_model_calls,
                    input_tokens: observed_input_tokens,
                    output_tokens: observed_output_tokens,
                    tool_calls: observed_tool_calls,
                    cost_usd: observed_cost_usd,
                },
                &mut budget_warnings,
            );
            let _ = events.send(AppControlEvent::ArtifactWritten {
                experiment_id: plan.experiment_id.clone(),
                campaign_id: campaign.campaign_id.clone(),
                path: completion.output.to_string_lossy().to_string(),
                sha256: digest_file(&completion.output)?,
            });
            shard_paths.push(completion.output);
            completed += 1;
            let _ = events.send(AppControlEvent::Phase {
                experiment_id: plan.experiment_id.clone(),
                phase: "running".to_string(),
                completed,
                total,
            });
        }
        shard_paths.sort();
        let evidence_path = campaign_root.join("eval-model-campaign.v1.json");
        let summary_path = campaign_root.join("summary.md");
        let mut aggregate = Command::new(&executable);
        aggregate
            .arg("--root")
            .arg(root)
            .arg("model")
            .arg("aggregate")
            .arg("--plan")
            .arg(&child_plan_path);
        for path in &shard_paths {
            aggregate.arg("--inputs").arg(path);
        }
        aggregate
            .arg("--output")
            .arg(&evidence_path)
            .arg("--summary")
            .arg(&summary_path)
            .env("HA_MODEL_EVAL_APP_CONTROL", "1")
            .env("HA_MODEL_EVAL_APP_PLAN", &app_plan_path)
            .env("HA_MODEL_EVAL_SOURCE", "local_app")
            .env(
                "HA_MODEL_EVAL_RUNTIME_CONFIG_DIGEST",
                &runtime_config_digest,
            )
            .env(PARENT_PID_ENV, std::process::id().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null());
        let status = run_cancellable(aggregate, &mut cancel).await?;
        if status.is_none() {
            supervisor.terminate(Duration::from_secs(10)).await;
            return Ok(RunCompletion::Cancelled);
        }
        if !status.is_some_and(|status| status.success()) {
            supervisor.terminate(Duration::from_secs(10)).await;
            bail!("local campaign aggregation failed");
        }
        let _: ha_eval_spec::model::ModelCampaignEvidence = read_json(&evidence_path)?;
        completed += 1;
        let evidence = evidence_path.to_string_lossy().to_string();
        evidence_paths.push(evidence.clone());
        let _ = events.send(AppControlEvent::ArtifactWritten {
            experiment_id: plan.experiment_id.clone(),
            campaign_id: campaign.campaign_id.clone(),
            path: evidence.clone(),
            sha256: digest_file(&evidence_path)?,
        });
        let _ = events.send(AppControlEvent::CampaignCompleted {
            experiment_id: plan.experiment_id.clone(),
            campaign_id: campaign.campaign_id.clone(),
            evidence_path: evidence,
        });
        let _ = events.send(AppControlEvent::Phase {
            experiment_id: plan.experiment_id.clone(),
            phase: "aggregating".to_string(),
            completed,
            total,
        });
    }
    shutdown_supervisor(&supervisor_bind, &supervisor_token).await;
    supervisor.terminate(Duration::from_secs(10)).await;
    Ok(RunCompletion::Completed(evidence_paths))
}

fn prepare_experiment_root(output_root: &Path, experiment_id: &str) -> Result<PathBuf> {
    if !experiment_id.starts_with("exp-")
        || !experiment_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("Invalid experiment id");
    }
    fs::create_dir_all(output_root)?;
    let root = output_root.canonicalize()?;
    let experiment = root.join(experiment_id);
    if experiment.exists() {
        bail!("Experiment output already exists; retries must create a new plan");
    }
    fs::create_dir_all(&experiment)?;
    Ok(experiment)
}

struct RuntimeCleanup(PathBuf);

impl Drop for RuntimeCleanup {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn reserve_loopback_address() -> Result<String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.to_string())
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn network_policy_name(policy: ha_eval_spec::model::NetworkPolicy) -> &'static str {
    match policy {
        ha_eval_spec::model::NetworkPolicy::ProviderOnly => "provider_only",
        ha_eval_spec::model::NetworkPolicy::Allowlist => "allowlist",
        ha_eval_spec::model::NetworkPolicy::SandboxLocal => "sandbox_local",
        ha_eval_spec::model::NetworkPolicy::LiveWeb => "live_web",
    }
}

async fn wait_server_ready(
    server_url: &str,
    supervisor: &mut ChildTree,
    cancel: &mut watch::Receiver<bool>,
) -> Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    for _ in 0..120 {
        if *cancel.borrow() {
            return Ok(true);
        }
        if let Some(status) = supervisor.try_wait()? {
            bail!("Hope supervisor exited during startup with {status}");
        }
        if client
            .get(format!("{server_url}/api/health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(false);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(250)) => {},
            changed = cancel.changed() => {
                if changed.is_ok() && *cancel.borrow() { return Ok(true); }
            }
        }
    }
    bail!("Isolated Hope server did not become healthy within 30 seconds")
}

async fn shutdown_supervisor(address: &str, token: &str) {
    if let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        let _ = client
            .post(format!("http://{address}/shutdown"))
            .bearer_auth(token)
            .send()
            .await;
    }
}

async fn run_cancellable(
    command: Command,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Option<ExitStatus>> {
    let mut child = ChildTree::spawn(command)?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if *cancel.borrow() {
            child.terminate(Duration::from_secs(10)).await;
            return Ok(None);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            changed = cancel.changed() => {
                if changed.is_ok() && *cancel.borrow() {
                    child.terminate(Duration::from_secs(10)).await;
                    return Ok(None);
                }
            }
        }
    }
}

struct ChildTree {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl ChildTree {
    fn spawn(mut command: Command) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command
            .spawn()
            .context("spawning evaluation child process")?;
        #[cfg(windows)]
        let job = create_kill_on_close_job(&child)?;
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
        })
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.child.try_wait().context("polling evaluation child")
    }

    async fn terminate(&mut self, grace: Duration) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
        let started = Instant::now();
        while started.elapsed() < grace {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ChildTree {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(windows)]
fn create_kill_on_close_job(child: &Child) -> Result<windows_sys::Win32::Foundation::HANDLE> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == 0 {
            bail!("creating evaluation Job Object failed");
        }
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || AssignProcessToJobObject(job, child.as_raw_handle() as isize) == 0
        {
            windows_sys::Win32::Foundation::CloseHandle(job);
            bail!("configuring evaluation Job Object failed");
        }
        Ok(job)
    }
}

fn send_error(
    events: &mpsc::UnboundedSender<AppControlEvent>,
    experiment_id: Option<String>,
    code: &str,
    message: &str,
    recoverable: bool,
) {
    let _ = events.send(AppControlEvent::Error {
        experiment_id,
        code: code.to_string(),
        message: message.to_string(),
        recoverable,
    });
}

fn event_campaign_id(event: &AppControlEvent) -> Option<String> {
    event.correlation_id().map(str::to_string)
}

fn safe_error(error: &anyhow::Error) -> String {
    let value = error.to_string();
    let value = value
        .replace("Authorization: Bearer ", "Authorization: [redacted] ")
        .replace("x-api-key", "[redacted-header]");
    value.chars().take(2_000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_high_entropy_and_header_safe() {
        let left = random_token();
        let right = random_token();
        assert_ne!(left, right);
        assert!(left.len() >= 40);
        assert!(!left.contains(['\r', '\n', ' ']));
    }

    #[test]
    fn experiment_ids_cannot_escape_output_root() {
        let temp = tempfile::tempdir().unwrap();
        assert!(prepare_experiment_root(temp.path(), "../escape").is_err());
        assert!(prepare_experiment_root(temp.path(), "exp-abcdef012345").is_ok());
    }
}
