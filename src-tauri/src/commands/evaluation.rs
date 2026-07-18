use super::CmdError;
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use ha_core::evaluation::ModelCampaignTier;
use ha_core::evaluation::{
    validate_app_control_envelope, AppControlCommand, AppControlEnvelope, AppControlEvent,
    AppControlHello, CodingHistorySource, DomainHistorySource, EvalAnnotationRecord, EvalAppPlan,
    EvalAppProfile, EvalAppRunRequest, EvalArtifactStore, EvalBaselineRecord, EvalCatalog,
    EvalCompareQuery, EvalCompareResult, EvalExperimentDetail, EvalExperimentRecord,
    EvalHistoryQuery, EvalHistorySource, EvalImportResult, EvalLocalExportResult, EvalOrchestrator,
    EvalPreview, EvalQueryService, EvalReadiness, EvalRepository, EvalResolvedLaunch,
    EvalTrendPoint, EvalTrendQuery, EvalTrialDetail, EvalWorkerEvent, EvalWorkerRuntime,
    APP_CONTROL_PROTOCOL_VERSION,
};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, Mutex};

type DesktopOrchestrator = EvalOrchestrator<DesktopEvalRuntime>;

const SIDECAR_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
const SIDECAR_CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub struct EvaluationState {
    orchestrator: Option<Arc<DesktopOrchestrator>>,
    init_error: Option<String>,
}

impl EvaluationState {
    pub fn new() -> Self {
        match build_orchestrator() {
            Ok(orchestrator) => Self {
                orchestrator: Some(Arc::new(orchestrator)),
                init_error: None,
            },
            Err(error) => Self {
                orchestrator: None,
                init_error: Some(format!("{error:#}")),
            },
        }
    }

    fn orchestrator(&self) -> Result<Arc<DesktopOrchestrator>> {
        self.orchestrator.clone().ok_or_else(|| {
            anyhow!(
                "Evaluation Center initialization failed: {}",
                self.init_error.as_deref().unwrap_or("unknown error")
            )
        })
    }
}

fn build_orchestrator() -> Result<DesktopOrchestrator> {
    let runtime = Arc::new(DesktopEvalRuntime::discover()?);
    let repository = EvalRepository::default_repository()?;
    let artifacts = EvalArtifactStore::default_store()?;
    let events = ha_core::get_event_bus()
        .cloned()
        .unwrap_or_else(|| Arc::new(ha_core::event_bus::BroadcastEventBus::new(64)));
    EvalOrchestrator::new(repository, artifacts, runtime, events)
}

fn refresh_import_trust(orchestrator: &DesktopOrchestrator, require_registry: bool) -> Result<()> {
    let trust_path = orchestrator
        .runtime()
        .paths
        .asset_root
        .as_ref()
        .map(|root| root.join("evals/live/trust/evidence-keys.json"));
    let trust = trust_path
        .as_deref()
        .ok_or_else(|| anyhow!("packaged evaluation assets are unavailable"))
        .and_then(ha_core::evaluation::load_evidence_trust_registry_file);
    match trust {
        Ok(trust) => {
            orchestrator
                .repository()
                .refresh_import_signature_status(&trust)?;
            Ok(())
        }
        Err(error) => {
            orchestrator
                .repository()
                .mark_import_signature_keys_missing()?;
            if require_registry {
                Err(error)
            } else {
                Ok(())
            }
        }
    }
}

#[derive(Clone)]
struct DesktopEvalRuntime {
    paths: DesktopEvalPaths,
    active: Arc<Mutex<Option<ActiveSidecar>>>,
}

#[derive(Clone)]
struct DesktopEvalPaths {
    sidecar: Option<PathBuf>,
    product: PathBuf,
    asset_root: Option<PathBuf>,
    output_root: PathBuf,
}

struct ActiveSidecar {
    run_id: String,
    plan_experiment_id: String,
    input: Arc<Mutex<ChildStdin>>,
    command_seq: Arc<AtomicU64>,
    hard_stop: mpsc::UnboundedSender<String>,
}

impl DesktopEvalRuntime {
    fn discover() -> Result<Self> {
        let product = std::env::current_exe()?.canonicalize()?;
        let sidecar_name = if cfg!(windows) {
            "hope-agent-eval.exe"
        } else {
            "hope-agent-eval"
        };
        let sidecar = product
            .parent()
            .map(|parent| parent.join(sidecar_name))
            .filter(|path| path.is_file())
            .or_else(|| {
                std::env::current_dir().ok().and_then(|cwd| {
                    find_upward(&cwd, |root| root.join("target/debug").join(sidecar_name))
                        .map(|root| root.join("target/debug").join(sidecar_name))
                })
            })
            .and_then(|path| path.canonicalize().ok());
        let asset_root = locate_asset_root(&product).ok();
        let output_root = ha_core::paths::evals_dir()?.join("runs");
        std::fs::create_dir_all(&output_root)?;
        Ok(Self {
            paths: DesktopEvalPaths {
                sidecar,
                product,
                asset_root,
                output_root,
            },
            active: Arc::new(Mutex::new(None)),
        })
    }

    async fn spawn_handshaken(&self) -> Result<SidecarSession> {
        let sidecar = self
            .paths
            .sidecar
            .as_ref()
            .ok_or_else(|| anyhow!("packaged evaluation Sidecar is missing"))?;
        let asset_root = self
            .paths
            .asset_root
            .as_ref()
            .ok_or_else(|| anyhow!("packaged evals/live assets are missing"))?;
        let mut command = Command::new(sidecar);
        command
            .arg("--root")
            .arg(asset_root)
            .arg("model")
            .arg("app-control")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("HA_EVAL_PARENT_PID", std::process::id().to_string())
            .kill_on_drop(false);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().context("starting evaluation Sidecar")?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("evaluation Sidecar stdin is unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("evaluation Sidecar stdout is unavailable"))?;
        let mut session = SidecarSession {
            child,
            input,
            output: BufReader::new(output),
            hello: None,
            command_seq: Arc::new(AtomicU64::new(1)),
            last_event_seq: 0,
        };
        let handshake = async {
            let event = tokio::time::timeout(SIDECAR_HANDSHAKE_TIMEOUT, session.read_event())
                .await
                .map_err(|_| anyhow!("evaluation Sidecar hello timed out"))??;
            let AppControlEvent::Hello { hello } = event else {
                bail!("evaluation Sidecar did not begin with a hello event");
            };
            if hello.product_version != env!("CARGO_PKG_VERSION")
                || hello.runner_digest
                    != ha_core::evaluation::artifact_sha256(&std::fs::read(sidecar)?)
            {
                bail!("evaluation Sidecar version or binary digest mismatch");
            }
            session
                .send(&AppControlCommand::HelloAck {
                    protocol_version: hello.protocol_version.clone(),
                    product_version: hello.product_version.clone(),
                    asset_root_digest: hello.asset_root_digest.clone(),
                })
                .await?;
            let ready = tokio::time::timeout(SIDECAR_HANDSHAKE_TIMEOUT, session.read_event())
                .await
                .map_err(|_| anyhow!("evaluation Sidecar ready acknowledgement timed out"))??;
            if !matches!(ready, AppControlEvent::Ready) {
                bail!("evaluation Sidecar rejected the version handshake");
            }
            Ok::<_, anyhow::Error>(hello)
        }
        .await;
        let hello = match handshake {
            Ok(hello) => hello,
            Err(error) => {
                wait_or_kill_sidecar(&mut session.child, std::time::Duration::ZERO).await;
                return Err(error);
            }
        };
        session.hello = Some(hello);
        Ok(session)
    }

    async fn with_ephemeral<T>(
        &self,
        command: AppControlCommand,
        parse: impl FnOnce(AppControlEvent) -> Result<T>,
    ) -> Result<T> {
        let mut session = self.spawn_handshaken().await?;
        let result = async {
            session.send(&command).await?;
            let event = tokio::time::timeout(SIDECAR_CONTROL_TIMEOUT, session.read_event())
                .await
                .map_err(|_| anyhow!("evaluation Sidecar control response timed out"))??;
            parse(event)
        }
        .await;
        let _ = session.send(&AppControlCommand::Shutdown).await;
        wait_or_kill_sidecar(&mut session.child, std::time::Duration::from_secs(3)).await;
        result
    }
}

struct SidecarSession {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    hello: Option<AppControlHello>,
    command_seq: Arc<AtomicU64>,
    last_event_seq: u64,
}

impl SidecarSession {
    async fn send(&mut self, command: &AppControlCommand) -> Result<()> {
        send_command_line(&mut self.input, &self.command_seq, command).await
    }

    async fn read_event(&mut self) -> Result<AppControlEvent> {
        read_event(&mut self.output, &mut self.last_event_seq).await
    }
}

#[async_trait]
impl EvalWorkerRuntime for DesktopEvalRuntime {
    async fn readiness(&self) -> Result<EvalReadiness> {
        let (signed_import_available, signed_import_issues) = self
            .paths
            .asset_root
            .as_deref()
            .map(|root| root.join("evals/live/trust/evidence-keys.json"))
            .map(
                |path| match ha_core::evaluation::validate_evidence_trust_registry_file(&path) {
                    Ok(()) => (true, Vec::new()),
                    Err(error) => (false, vec![error.to_string()]),
                },
            )
            .unwrap_or_else(|| {
                (
                    false,
                    vec!["packaged evaluation assets are unavailable".to_string()],
                )
            });
        match self.spawn_handshaken().await {
            Ok(mut session) => {
                let hello = session.hello.clone();
                let _ = session.send(&AppControlCommand::Shutdown).await;
                wait_or_kill_sidecar(&mut session.child, std::time::Duration::from_secs(3)).await;
                Ok(EvalReadiness {
                    available: true,
                    can_run: true,
                    remote_run_enabled: false,
                    signed_import_available,
                    hello,
                    issues: Vec::new(),
                    signed_import_issues,
                })
            }
            Err(error) => Ok(EvalReadiness {
                available: false,
                can_run: false,
                remote_run_enabled: false,
                signed_import_available,
                hello: None,
                issues: vec![error.to_string()],
                signed_import_issues,
            }),
        }
    }

    async fn list_profiles(&self) -> Result<Vec<EvalAppProfile>> {
        self.with_ephemeral(AppControlCommand::ListProfiles, |event| match event {
            AppControlEvent::Profiles { profiles } => Ok(profiles),
            AppControlEvent::Error { message, .. } => bail!(message),
            _ => bail!("evaluation Sidecar returned an unexpected profile event"),
        })
        .await
    }

    async fn list_suites(&self) -> Result<Vec<ha_core::evaluation::AppEvalSuiteCatalog>> {
        self.with_ephemeral(AppControlCommand::ListCatalog, |event| match event {
            AppControlEvent::Catalog { suites } => Ok(suites),
            AppControlEvent::Error { message, .. } => bail!(message),
            _ => bail!("evaluation Sidecar returned an unexpected catalog event"),
        })
        .await
    }

    async fn preview(&self, launch: &EvalResolvedLaunch) -> Result<EvalAppPlan> {
        self.with_ephemeral(
            AppControlCommand::Preview {
                request: launch.request.clone(),
                resolved_models: launch.models.clone(),
                reference: launch.reference.clone(),
                dirty: launch.dirty,
                app_version: launch.app_version.clone(),
                runtime_environment: launch.runtime_environment.clone(),
            },
            |event| match event {
                AppControlEvent::Preview { plan } => Ok(plan),
                AppControlEvent::Error { message, .. } => bail!(message),
                _ => bail!("evaluation Sidecar returned an unexpected preview event"),
            },
        )
        .await
    }

    async fn start(
        &self,
        run_id: &str,
        plan: &EvalAppPlan,
        launch: EvalResolvedLaunch,
    ) -> Result<mpsc::UnboundedReceiver<EvalWorkerEvent>> {
        let mut active = self.active.lock().await;
        if active.is_some() {
            bail!("desktop evaluation Sidecar is already active");
        }
        let mut session = self.spawn_handshaken().await?;
        let output_root = self.paths.output_root.join(run_id);
        std::fs::create_dir_all(&output_root)?;
        let start_ack = async {
            session
                .send(&AppControlCommand::Start {
                    request: launch.request,
                    resolved_models: launch.models,
                    reference: launch.reference,
                    dirty: launch.dirty,
                    app_version: launch.app_version,
                    runtime_environment: launch.runtime_environment,
                    product_binary: self.paths.product.to_string_lossy().to_string(),
                    product_binary_digest: ha_core::evaluation::artifact_sha256(&std::fs::read(
                        &self.paths.product,
                    )?),
                    output_root: output_root.to_string_lossy().to_string(),
                    config: launch.credential_free_config,
                    provider_secrets_b64: launch.provider_secrets_b64,
                })
                .await?;
            let event = tokio::time::timeout(SIDECAR_CONTROL_TIMEOUT, session.read_event())
                .await
                .map_err(|_| anyhow!("evaluation Sidecar start acknowledgement timed out"))??;
            match event {
                AppControlEvent::Started {
                    experiment_id,
                    plan_digest,
                } if experiment_id == plan.experiment_id && plan_digest == plan.plan_digest => {
                    Ok::<_, anyhow::Error>(())
                }
                AppControlEvent::Error { message, .. } => bail!(message),
                _ => bail!("evaluation Sidecar returned an invalid start acknowledgement"),
            }
        }
        .await;
        if let Err(error) = start_ack {
            wait_or_kill_sidecar(&mut session.child, std::time::Duration::ZERO).await;
            return Err(error);
        }
        let input = Arc::new(Mutex::new(session.input));
        let command_seq = Arc::clone(&session.command_seq);
        let mut last_event_seq = session.last_event_seq;
        let (hard_stop_tx, mut hard_stop_rx) = mpsc::unbounded_channel::<String>();
        *active = Some(ActiveSidecar {
            run_id: run_id.to_string(),
            plan_experiment_id: plan.experiment_id.clone(),
            input: Arc::clone(&input),
            command_seq: Arc::clone(&command_seq),
            hard_stop: hard_stop_tx.clone(),
        });
        drop(active);
        if let Some(max_wall_seconds) = plan.campaign_budget.max_wall_seconds {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(
                    max_wall_seconds.saturating_add(30),
                ))
                .await;
                let _ = hard_stop_tx.send("sidecar_wall_timeout".to_string());
            });
        }
        let active_state = Arc::clone(&self.active);
        let run_id_owned = run_id.to_string();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut reader = session.output;
            let mut child = session.child;
            let mut terminal = false;
            loop {
                let event_result = tokio::select! {
                    event = read_event(&mut reader, &mut last_event_seq) => event,
                    reason = hard_stop_rx.recv() => {
                        let reason = reason.unwrap_or_else(|| "sidecar_hard_stop".to_string());
                        wait_or_kill_sidecar(&mut child, std::time::Duration::ZERO).await;
                        let _ = events_tx.send(EvalWorkerEvent::Failed {
                            experiment_id: run_id_owned.clone(),
                            code: reason,
                            message: "Evaluation Sidecar did not terminate within its hard deadline".to_string(),
                        });
                        break;
                    }
                };
                let event = match event_result {
                    Ok(event) => event,
                    Err(error) => {
                        if !terminal {
                            let _ = events_tx.send(EvalWorkerEvent::Failed {
                                experiment_id: run_id_owned.clone(),
                                code: "sidecar_stream_closed".to_string(),
                                message: error.to_string(),
                            });
                        }
                        break;
                    }
                };
                match event {
                    AppControlEvent::Phase {
                        phase,
                        completed,
                        total,
                        ..
                    } => {
                        let _ = events_tx.send(EvalWorkerEvent::Phase {
                            experiment_id: run_id_owned.clone(),
                            phase,
                            completed,
                            total,
                        });
                    }
                    AppControlEvent::CampaignCompleted {
                        campaign_id,
                        evidence_path,
                        ..
                    } => {
                        let _ = events_tx.send(EvalWorkerEvent::Evidence {
                            experiment_id: run_id_owned.clone(),
                            campaign_id,
                            evidence_path,
                        });
                    }
                    AppControlEvent::TrialStarted {
                        campaign_id,
                        trial_id,
                        completed,
                        total,
                        ..
                    } => {
                        let _ = events_tx.send(EvalWorkerEvent::TrialStarted {
                            experiment_id: run_id_owned.clone(),
                            campaign_id,
                            trial_id,
                            completed,
                            total,
                        });
                    }
                    AppControlEvent::TrialCompleted {
                        campaign_id,
                        trial_id,
                        completed,
                        total,
                        outcome,
                        wall_ms,
                        input_tokens,
                        output_tokens,
                        cost_usd,
                        ..
                    } => {
                        let _ = events_tx.send(EvalWorkerEvent::TrialCompleted {
                            experiment_id: run_id_owned.clone(),
                            campaign_id,
                            trial_id,
                            completed,
                            total,
                            outcome,
                            wall_ms,
                            input_tokens,
                            output_tokens,
                            cost_usd,
                        });
                    }
                    AppControlEvent::BudgetWarning {
                        dimension,
                        observed,
                        limit,
                        ratio,
                        ..
                    } => {
                        let _ = events_tx.send(EvalWorkerEvent::BudgetWarning {
                            experiment_id: run_id_owned.clone(),
                            dimension,
                            observed,
                            limit,
                            ratio,
                        });
                    }
                    AppControlEvent::ArtifactWritten {
                        campaign_id,
                        path,
                        sha256,
                        ..
                    } => {
                        let _ = events_tx.send(EvalWorkerEvent::ArtifactWritten {
                            experiment_id: run_id_owned.clone(),
                            campaign_id,
                            path,
                            sha256,
                        });
                    }
                    AppControlEvent::Completed { evidence_paths, .. } => {
                        terminal = true;
                        let _ = events_tx.send(EvalWorkerEvent::Completed {
                            experiment_id: run_id_owned.clone(),
                            evidence_paths,
                        });
                        let _ = send_command_line(
                            &mut *input.lock().await,
                            &command_seq,
                            &AppControlCommand::Shutdown,
                        )
                        .await;
                    }
                    AppControlEvent::Cancelled { .. } => {
                        terminal = true;
                        let _ = events_tx.send(EvalWorkerEvent::Cancelled {
                            experiment_id: run_id_owned.clone(),
                        });
                        let _ = send_command_line(
                            &mut *input.lock().await,
                            &command_seq,
                            &AppControlCommand::Shutdown,
                        )
                        .await;
                    }
                    AppControlEvent::Error { code, message, .. } => {
                        terminal = true;
                        let _ = events_tx.send(EvalWorkerEvent::Failed {
                            experiment_id: run_id_owned.clone(),
                            code,
                            message,
                        });
                        let _ = send_command_line(
                            &mut *input.lock().await,
                            &command_seq,
                            &AppControlCommand::Shutdown,
                        )
                        .await;
                    }
                    AppControlEvent::Bye => break,
                    AppControlEvent::Hello { .. }
                    | AppControlEvent::Ready
                    | AppControlEvent::Profiles { .. }
                    | AppControlEvent::Catalog { .. }
                    | AppControlEvent::Preview { .. }
                    | AppControlEvent::Started { .. } => {}
                }
            }
            wait_or_kill_sidecar(&mut child, std::time::Duration::from_secs(12)).await;
            let mut active = active_state.lock().await;
            if active
                .as_ref()
                .is_some_and(|value| value.run_id == run_id_owned)
            {
                *active = None;
            }
        });
        Ok(events_rx)
    }

    async fn cancel(&self, run_id: &str) -> Result<()> {
        let active = self.active.lock().await;
        let sidecar = active
            .as_ref()
            .filter(|active| active.run_id == run_id)
            .ok_or_else(|| anyhow!("desktop evaluation Sidecar is not active"))?;
        let input = Arc::clone(&sidecar.input);
        let command_seq = Arc::clone(&sidecar.command_seq);
        let plan_experiment_id = sidecar.plan_experiment_id.clone();
        let hard_stop = sidecar.hard_stop.clone();
        drop(active);
        let mut input = input.lock().await;
        send_command_line(
            &mut input,
            &command_seq,
            &AppControlCommand::Cancel {
                experiment_id: plan_experiment_id,
            },
        )
        .await?;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;
            let _ = hard_stop.send("cancel_timeout".to_string());
        });
        Ok(())
    }

    async fn cleanup(&self, run_id: &str) -> Result<()> {
        if !run_id.starts_with("evalrun-")
            || !run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("invalid evaluation run id for cleanup");
        }
        let root = self.paths.output_root.canonicalize()?;
        let target = root.join(run_id);
        let metadata = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let canonical = target.canonicalize()?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || canonical.parent() != Some(root.as_path())
        {
            bail!("evaluation run cleanup target escaped its output root");
        }
        std::fs::remove_dir_all(&canonical)
            .with_context(|| format!("removing evaluation output {}", canonical.display()))
    }
}

async fn send_command_line(
    input: &mut ChildStdin,
    sequence: &AtomicU64,
    command: &AppControlCommand,
) -> Result<()> {
    let envelope = AppControlEnvelope {
        protocol_version: APP_CONTROL_PROTOCOL_VERSION.to_string(),
        campaign_id: command.correlation_id().map(str::to_string),
        seq: sequence.fetch_add(1, Ordering::Relaxed),
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        payload: command,
    };
    let mut bytes = serde_json::to_vec(&envelope)?;
    bytes.push(b'\n');
    input.write_all(&bytes).await?;
    input.flush().await?;
    Ok(())
}

async fn wait_or_kill_sidecar(child: &mut Child, grace: std::time::Duration) {
    if tokio::time::timeout(grace, child.wait()).await.is_ok() {
        return;
    }
    #[cfg(unix)]
    if let Some(id) = child.id() {
        unsafe {
            libc::kill(-(id as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn read_event(
    reader: &mut BufReader<ChildStdout>,
    last_seq: &mut u64,
) -> Result<AppControlEvent> {
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            bail!("evaluation Sidecar closed its protocol stream");
        }
        if line.len() > 16 * 1024 * 1024 {
            bail!("evaluation Sidecar event exceeds 16 MiB");
        }
        let envelope: AppControlEnvelope<AppControlEvent> =
            serde_json::from_str(&line).context("decoding evaluation Sidecar event")?;
        validate_app_control_envelope(&envelope)?;
        if envelope.campaign_id.as_deref() != envelope.payload.correlation_id() {
            bail!("evaluation Sidecar event correlation identity does not match its payload");
        }
        DateTime::parse_from_rfc3339(&envelope.timestamp)
            .context("invalid evaluation Sidecar event timestamp")?;
        if envelope.seq <= *last_seq {
            continue;
        }
        if envelope.seq != last_seq.saturating_add(1) {
            bail!("evaluation Sidecar event sequence contains a gap");
        }
        *last_seq = envelope.seq;
        return Ok(envelope.payload);
    }
}

fn locate_asset_root(product: &Path) -> Result<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = find_upward(&cwd, |root| root.join("evals/live")) {
            return root
                .canonicalize()
                .context("canonicalizing evaluation checkout root");
        }
    }
    for ancestor in product.ancestors() {
        for candidate in [ancestor.to_path_buf(), ancestor.join("Resources")] {
            if candidate.join("evals/live").is_dir() {
                return candidate
                    .canonicalize()
                    .context("canonicalizing packaged evaluation asset root");
            }
        }
    }
    bail!("packaged evals/live assets are missing")
}

fn find_upward(start: &Path, resolve: impl Fn(&Path) -> PathBuf) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|root| resolve(root).exists())
        .map(Path::to_path_buf)
}

async fn resolve_launch(
    runtime: &DesktopEvalRuntime,
    request: EvalAppRunRequest,
) -> Result<EvalResolvedLaunch> {
    let readiness = runtime.readiness().await?;
    let hello = readiness.hello.ok_or_else(|| {
        anyhow!(
            "evaluation Sidecar is not ready: {}",
            readiness.issues.join(", ")
        )
    })?;
    let (reference, dirty) = local_build_identity(&runtime.paths.product);
    ha_core::evaluation::resolve_local_launch(
        request,
        reference,
        dirty,
        env!("CARGO_PKG_VERSION").to_string(),
        ha_core::evaluation::artifact_sha256(&std::fs::read(&runtime.paths.product)?),
        hello.runner_digest,
        hello.asset_root_digest,
    )
}

fn local_build_identity(product: &Path) -> (String, bool) {
    let fallback = (
        env!("HA_BUILD_COMMIT_SHA").to_string(),
        env!("HA_BUILD_GIT_DIRTY") == "1",
    );
    let Some(start) = product.parent() else {
        return fallback;
    };
    let Some(root) = find_upward(start, |candidate| candidate.join(".git")) else {
        return fallback;
    };
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| {
            matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
    let dirty = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude)src-tauri/binaries/hope-agent-eval-*",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty());
    head.zip(dirty).unwrap_or(fallback)
}

#[tauri::command]
pub async fn eval_readiness(
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalReadiness, CmdError> {
    Ok(state.orchestrator()?.readiness().await?)
}

#[tauri::command]
pub async fn eval_catalog(
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalCatalog, CmdError> {
    let orchestrator = state.orchestrator()?;
    let readiness = orchestrator.readiness().await?;
    let profiles = if readiness.available {
        orchestrator.list_profiles().await?
    } else {
        Vec::new()
    };
    let suites = if readiness.available {
        orchestrator.runtime().list_suites().await?
    } else {
        Vec::new()
    };
    Ok(EvalCatalog {
        readiness,
        profiles,
        suites,
        models: ha_core::evaluation::list_model_options()?,
    })
}

#[tauri::command]
pub fn eval_list_model_options() -> Result<Vec<ha_core::evaluation::EvalModelOption>, CmdError> {
    Ok(ha_core::evaluation::list_model_options()?)
}

#[tauri::command]
pub async fn eval_preview(
    request: EvalAppRunRequest,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalPreview, CmdError> {
    let orchestrator = state.orchestrator()?;
    let runtime = orchestrator.runtime();
    let launch = resolve_launch(&runtime, request).await?;
    Ok(orchestrator.preview(&launch).await?)
}

#[tauri::command]
pub async fn eval_start(
    request: EvalAppRunRequest,
    parent_experiment_id: Option<String>,
    expected_plan_digest: Option<String>,
    state: tauri::State<'_, EvaluationState>,
) -> Result<String, CmdError> {
    let orchestrator = state.orchestrator()?;
    let runtime = orchestrator.runtime();
    let launch = resolve_launch(&runtime, request).await?;
    Ok(orchestrator
        .start(
            launch,
            parent_experiment_id.as_deref(),
            expected_plan_digest.as_deref(),
        )
        .await?)
}

#[tauri::command]
pub async fn eval_cancel(
    experiment_id: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<(), CmdError> {
    state.orchestrator()?.cancel(&experiment_id).await?;
    Ok(())
}

#[tauri::command]
pub async fn eval_retry(
    experiment_id: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<String, CmdError> {
    let orchestrator = state.orchestrator()?;
    let request = orchestrator
        .repository()
        .experiment_request(&experiment_id)?;
    let runtime = orchestrator.runtime();
    let launch = resolve_launch(&runtime, request).await?;
    Ok(orchestrator
        .start(launch, Some(&experiment_id), None)
        .await?)
}

#[tauri::command]
pub async fn eval_list_history(
    query: EvalHistoryQuery,
    state: tauri::State<'_, EvaluationState>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<EvalExperimentRecord>, CmdError> {
    let orchestrator = state.orchestrator()?;
    refresh_import_trust(&orchestrator, false)?;
    let limit = query.limit.clamp(1, 200);
    let mut source_query = query.clone();
    source_query.limit = limit.saturating_add(query.offset).min(200);
    source_query.offset = 0;
    let mut records = orchestrator.repository().list_experiments(&source_query)?;
    let legacy_query = source_query;
    let db = app_state.session_db.clone();
    let mut legacy = db
        .run(move |db| {
            let mut records = CodingHistorySource::new(db).list(&legacy_query)?;
            records.extend(DomainHistorySource::new(db).list(&legacy_query)?);
            Ok::<_, anyhow::Error>(records)
        })
        .await?;
    records.append(&mut legacy);
    records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(records
        .into_iter()
        .skip(query.offset as usize)
        .take(limit as usize)
        .collect())
}

#[tauri::command]
pub async fn eval_get_experiment(
    experiment_id: String,
    state: tauri::State<'_, EvaluationState>,
    app_state: tauri::State<'_, crate::AppState>,
) -> Result<Option<EvalExperimentDetail>, CmdError> {
    let orchestrator = state.orchestrator()?;
    refresh_import_trust(&orchestrator, false)?;
    if let Some(detail) = orchestrator.repository().detail(&experiment_id)? {
        return Ok(Some(detail));
    }
    let db = app_state.session_db.clone();
    if let Some(id) = experiment_id.strip_prefix("coding:").map(str::to_string) {
        return Ok(db
            .run(move |db| {
                Ok::<_, anyhow::Error>(
                    db.get_coding_benchmark_campaign(&id)?
                        .map(|value| ha_core::evaluation::coding_detail(&value)),
                )
            })
            .await?);
    }
    if let Some(id) = experiment_id.strip_prefix("domain:").map(str::to_string) {
        return Ok(db
            .run(move |db| {
                Ok::<_, anyhow::Error>(
                    db.get_domain_eval_campaign(&id)?
                        .map(|value| ha_core::evaluation::domain_detail(&value)),
                )
            })
            .await?);
    }
    Ok(None)
}

#[tauri::command]
pub fn eval_compare(
    query: EvalCompareQuery,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalCompareResult, CmdError> {
    let orchestrator = state.orchestrator()?;
    refresh_import_trust(&orchestrator, false)?;
    let repository = orchestrator.repository().clone();
    Ok(EvalQueryService::new(repository, EvalArtifactStore::default_store()?).compare(&query)?)
}

#[tauri::command]
pub fn eval_trends(
    query: EvalTrendQuery,
    state: tauri::State<'_, EvaluationState>,
) -> Result<Vec<EvalTrendPoint>, CmdError> {
    let orchestrator = state.orchestrator()?;
    refresh_import_trust(&orchestrator, false)?;
    let repository = orchestrator.repository().clone();
    Ok(EvalQueryService::new(repository, EvalArtifactStore::default_store()?).trends(&query)?)
}

#[tauri::command]
pub fn eval_get_trial(
    experiment_id: String,
    campaign_id: String,
    trial_id: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalTrialDetail, CmdError> {
    let repository = state.orchestrator()?.repository().clone();
    Ok(
        EvalQueryService::new(repository, EvalArtifactStore::default_store()?).trial(
            &experiment_id,
            &campaign_id,
            &trial_id,
        )?,
    )
}

#[tauri::command]
pub fn eval_set_pinned(
    experiment_id: String,
    pinned: bool,
    state: tauri::State<'_, EvaluationState>,
) -> Result<(), CmdError> {
    state
        .orchestrator()?
        .repository()
        .set_experiment_pinned(&experiment_id, pinned)?;
    Ok(())
}

#[tauri::command]
pub fn eval_import_bundle(
    bundle_path: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalImportResult, CmdError> {
    let orchestrator = state.orchestrator()?;
    let runtime = orchestrator.runtime();
    let asset_root = runtime
        .paths
        .asset_root
        .as_ref()
        .ok_or_else(|| anyhow!("packaged evaluation assets are unavailable"))?;
    let trust = asset_root.join("evals/live/trust/evidence-keys.json");
    let result = ha_core::evaluation::import_evidence_bundle(
        Path::new(&bundle_path),
        &trust,
        orchestrator.repository(),
        &EvalArtifactStore::default_store()?,
    )?;
    refresh_import_trust(&orchestrator, true)?;
    if let Some(events) = ha_core::get_event_bus() {
        events.emit(
            ha_core::evaluation::EVALUATION_EVENT,
            serde_json::json!({"experimentId": result.experiment_id, "change": "imported"}),
        );
    }
    Ok(result)
}

#[tauri::command]
pub fn eval_import_unverified(
    evidence_path: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalImportResult, CmdError> {
    let orchestrator = state.orchestrator()?;
    let result = ha_core::evaluation::import_unverified_evidence_file(
        Path::new(&evidence_path),
        orchestrator.repository(),
        &EvalArtifactStore::default_store()?,
    )?;
    if let Some(events) = ha_core::get_event_bus() {
        events.emit(
            ha_core::evaluation::EVALUATION_EVENT,
            serde_json::json!({"experimentId": result.experiment_id, "change": "imported"}),
        );
    }
    Ok(result)
}

#[tauri::command]
pub fn eval_export_local_bundle(
    experiment_id: String,
    output_path: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalLocalExportResult, CmdError> {
    let orchestrator = state.orchestrator()?;
    Ok(ha_core::evaluation::export_local_evidence_bundle(
        &experiment_id,
        Path::new(&output_path),
        orchestrator.repository(),
        &EvalArtifactStore::default_store()?,
    )?)
}

#[tauri::command]
pub fn eval_create_baseline(
    experiment_id: String,
    tier: ModelCampaignTier,
    note: Option<String>,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalBaselineRecord, CmdError> {
    let orchestrator = state.orchestrator()?;
    refresh_import_trust(&orchestrator, true)?;
    Ok(orchestrator.repository().create_baseline(
        &experiment_id,
        tier,
        "desktop_owner",
        note.as_deref(),
    )?)
}

#[tauri::command]
pub fn eval_delete_baseline(
    baseline_id: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<bool, CmdError> {
    Ok(state
        .orchestrator()?
        .repository()
        .delete_baseline(&baseline_id)?)
}

#[tauri::command]
pub fn eval_list_baselines(
    tier: Option<ModelCampaignTier>,
    state: tauri::State<'_, EvaluationState>,
) -> Result<Vec<EvalBaselineRecord>, CmdError> {
    Ok(state.orchestrator()?.repository().list_baselines(tier)?)
}

#[tauri::command]
pub fn eval_create_annotation(
    experiment_id: String,
    campaign_id: Option<String>,
    trial_id: Option<String>,
    text: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<EvalAnnotationRecord, CmdError> {
    Ok(state.orchestrator()?.repository().create_annotation(
        &experiment_id,
        campaign_id.as_deref(),
        trial_id.as_deref(),
        &text,
    )?)
}

#[tauri::command]
pub fn eval_list_annotations(
    experiment_id: String,
    state: tauri::State<'_, EvaluationState>,
) -> Result<Vec<EvalAnnotationRecord>, CmdError> {
    Ok(state
        .orchestrator()?
        .repository()
        .list_annotations(&experiment_id)?)
}
