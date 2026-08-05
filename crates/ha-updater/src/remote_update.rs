//! Owner-plane update lifecycle for a remotely connected Web / desktop UI.
//!
//! The browser never supplies a command, URL, artifact, or filesystem path.
//! It may only confirm a short-lived, process-bound plan produced from a
//! freshly verified release manifest. Durable job state lives under the
//! updater data directory so the new process can reconcile an interrupted
//! connection after a service restart.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::source_detector::InstallSource;
use crate::{CheckOutcome, RecommendedPath};

const STATE_FILE: &str = "remote-update-state.json";
const PLAN_TTL_MINUTES: i64 = 5;
const MAX_JOBS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteUpdateSnapshot {
    pub server_instance_id: String,
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub install_source: String,
    pub recommended_path: RecommendedPath,
    pub capability: RemoteUpdateCapability,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub bare_binary_available: bool,
    pub staged_version: Option<String>,
    pub checked_at: String,
    pub manual_instructions: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteUpdateCapability {
    Automatic,
    DockerRedeploy,
    Manual,
    DesktopOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteUpdateJob {
    pub job_id: String,
    pub from_version: String,
    pub target_version: String,
    pub path: RecommendedPath,
    pub status: RemoteJobStatus,
    pub phase: String,
    pub percent: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteJobStatus {
    Running,
    AwaitingRestart,
    Succeeded,
    Failed,
}

impl RemoteJobStatus {
    fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteUpdateStatus {
    pub server_instance_id: String,
    pub current_version: String,
    pub snapshot: Option<RemoteUpdateSnapshot>,
    pub active_job: Option<RemoteUpdateJob>,
    pub recent_jobs: Vec<RemoteUpdateJob>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInstallPlan {
    pub plan_id: Option<String>,
    pub server_instance_id: String,
    pub current_version: String,
    pub target_version: String,
    pub path: RecommendedPath,
    pub capability: RemoteUpdateCapability,
    pub expires_at: Option<String>,
    pub confirmation: String,
    pub manual_instructions: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingPlan {
    public: RemoteInstallPlan,
    source: InstallSource,
    manifest: Manifest,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedState {
    snapshot: Option<RemoteUpdateSnapshot>,
    jobs: Vec<RemoteUpdateJob>,
}

#[derive(Default)]
struct RuntimeState {
    persisted: PersistedState,
    plans: HashMap<String, PendingPlan>,
}

fn runtime() -> &'static Mutex<RuntimeState> {
    static RUNTIME: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(RuntimeState::default()))
}

fn server_instance_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn state_path() -> Result<std::path::PathBuf> {
    Ok(ha_core::paths::updater_dir()?.join(STATE_FILE))
}

async fn ensure_loaded() -> Result<()> {
    static LOADED: AtomicBool = AtomicBool::new(false);
    static LOAD_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    if LOADED.load(Ordering::Acquire) {
        return Ok(());
    }
    let _guard = LOAD_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    if LOADED.load(Ordering::Acquire) {
        return Ok(());
    }
    let loaded = ha_core::blocking::run_blocking(|| -> Result<PersistedState> {
        let path = state_path()?;
        if !path.exists() {
            return Ok(PersistedState::default());
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read remote update state {}", path.display()))?;
        serde_json::from_slice(&bytes).context("parse remote update state")
    })
    .await?;
    {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        g.persisted = loaded;
        reconcile_after_restart(&mut g.persisted);
    }
    LOADED.store(true, Ordering::Release);
    persist().await
}

fn reconcile_after_restart(state: &mut PersistedState) {
    let current = ha_core::app_init::app_version();
    let stamp = now();
    if let Some(snapshot) = state.snapshot.as_mut() {
        snapshot.server_instance_id = server_instance_id().into();
        snapshot.current_version = current.into();
        if snapshot.latest_version.trim_start_matches('v') == current.trim_start_matches('v') {
            snapshot.has_update = false;
        }
    }
    for job in &mut state.jobs {
        if job.target_version.trim_start_matches('v') == current.trim_start_matches('v') {
            if job.status != RemoteJobStatus::Succeeded {
                job.updated_at = stamp.clone();
                job.status = RemoteJobStatus::Succeeded;
                job.phase = "done".into();
                job.percent = Some(100);
                job.error = None;
            }
            continue;
        }
        if job.status.terminal() {
            continue;
        }
        job.updated_at = stamp.clone();
        job.status = RemoteJobStatus::Failed;
        job.phase = "restart_failed".into();
        job.error = Some(format!(
            "服务已重新启动，但运行版本仍为 {current}；请检查服务管理器或手动恢复"
        ));
    }
}

async fn persist() -> Result<()> {
    let bytes = {
        let g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        serde_json::to_vec_pretty(&g.persisted).context("serialize remote update state")?
    };
    ha_core::blocking::run_blocking(move || -> Result<()> {
        let path = state_path()?;
        ha_core::platform::write_atomic(&path, &bytes)
            .with_context(|| format!("write remote update state {}", path.display()))
    })
    .await
}

fn capability(outcome: &CheckOutcome) -> (RemoteUpdateCapability, Option<String>) {
    match (&outcome.install_source, outcome.recommended_path) {
        (InstallSource::Docker, _) => (
            RemoteUpdateCapability::DockerRedeploy,
            Some(format!(
                "此服务运行在 Docker 中。请拉取包含 Hope Agent {} 的新镜像并重新创建容器；服务端不会在容器内替换二进制。",
                outcome.latest_version
            )),
        ),
        (_, RecommendedPath::SelfContained | RecommendedPath::PackageManager) => {
            (RemoteUpdateCapability::Automatic, None)
        }
        (_, RecommendedPath::Tauri) => (
            RemoteUpdateCapability::DesktopOnly,
            Some("该服务来自桌面应用包，请在服务器本机更新桌面应用。".into()),
        ),
        _ => (
            RemoteUpdateCapability::Manual,
            Some(format!(
                "当前安装方式不支持远程自动更新，请在服务器上手动安装 Hope Agent {}。",
                outcome.latest_version
            )),
        ),
    }
}

fn snapshot_from(outcome: &CheckOutcome, staged_version: Option<String>) -> RemoteUpdateSnapshot {
    let (capability, manual_instructions) = capability(outcome);
    RemoteUpdateSnapshot {
        server_instance_id: server_instance_id().to_string(),
        current_version: outcome.current_version.clone(),
        latest_version: outcome.latest_version.clone(),
        has_update: outcome.has_update,
        install_source: outcome.install_source.label().into(),
        recommended_path: outcome.recommended_path,
        capability,
        notes: outcome.notes.clone(),
        pub_date: outcome.pub_date.clone(),
        bare_binary_available: outcome.bare_binary_available,
        staged_version,
        checked_at: now(),
        manual_instructions,
    }
}

pub async fn status() -> Result<RemoteUpdateStatus> {
    ensure_loaded().await?;
    let g = runtime().lock().unwrap_or_else(|p| p.into_inner());
    let active_job = g
        .persisted
        .jobs
        .iter()
        .rev()
        .find(|job| !job.status.terminal())
        .cloned();
    Ok(RemoteUpdateStatus {
        server_instance_id: server_instance_id().into(),
        current_version: ha_core::app_init::app_version().into(),
        snapshot: g.persisted.snapshot.clone(),
        active_job,
        recent_jobs: g.persisted.jobs.iter().rev().cloned().collect(),
    })
}

pub async fn check_now() -> Result<RemoteUpdateStatus> {
    ensure_loaded().await?;
    let (outcome, manifest) = crate::check_update_full().await?;
    record_check(outcome, manifest).await?;
    status().await
}

pub async fn record_check(outcome: CheckOutcome, _manifest: Manifest) -> Result<()> {
    ensure_loaded().await?;
    {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        let staged = g
            .persisted
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.staged_version.clone())
            .filter(|version| version == &outcome.latest_version);
        g.persisted.snapshot = Some(snapshot_from(&outcome, staged));
    }
    persist().await
}

pub async fn record_staged(version: String) -> Result<()> {
    ensure_loaded().await?;
    {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(snapshot) = g.persisted.snapshot.as_mut() {
            snapshot.staged_version = Some(version);
        }
    }
    persist().await
}

pub async fn prepare_install(
    expected_current_version: &str,
    target_version: &str,
    expected_server_instance_id: &str,
) -> Result<RemoteInstallPlan> {
    ensure_loaded().await?;
    if expected_server_instance_id != server_instance_id() {
        anyhow::bail!("服务已重新启动，请刷新更新状态后重试");
    }
    let (outcome, manifest) = crate::check_update_full().await?;
    if outcome.current_version != expected_current_version {
        anyhow::bail!("服务版本已变化，请刷新更新状态后重试");
    }
    if !outcome.has_update || outcome.latest_version != target_version {
        anyhow::bail!("目标版本已变化或当前已是最新版本，请重新检查");
    }
    let (capability, manual_instructions) = capability(&outcome);
    let executable = capability == RemoteUpdateCapability::Automatic;
    let plan_id = executable.then(|| uuid::Uuid::new_v4().to_string());
    let expires_at =
        executable.then(|| (Utc::now() + Duration::minutes(PLAN_TTL_MINUTES)).to_rfc3339());
    let confirmation = if executable {
        format!(
            "将远程服务端从 {} 更新到 {}。服务会短暂断开，正在运行的远程任务可能被中止。",
            outcome.current_version, outcome.latest_version
        )
    } else {
        manual_instructions.clone().unwrap_or_default()
    };
    let public = RemoteInstallPlan {
        plan_id: plan_id.clone(),
        server_instance_id: server_instance_id().into(),
        current_version: outcome.current_version.clone(),
        target_version: outcome.latest_version.clone(),
        path: outcome.recommended_path,
        capability,
        expires_at,
        confirmation,
        manual_instructions,
    };
    {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        let staged = g
            .persisted
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.staged_version.clone())
            .filter(|version| version == &outcome.latest_version);
        g.persisted.snapshot = Some(snapshot_from(&outcome, staged));
        if let Some(id) = plan_id {
            g.plans.retain(|_, plan| {
                plan.public
                    .expires_at
                    .as_deref()
                    .and_then(|stamp| stamp.parse::<chrono::DateTime<Utc>>().ok())
                    .is_some_and(|stamp| stamp > Utc::now())
            });
            g.plans.insert(
                id,
                PendingPlan {
                    public: public.clone(),
                    source: outcome.install_source,
                    manifest,
                },
            );
        }
    }
    persist().await?;
    Ok(public)
}

pub async fn confirm_install(plan_id: &str) -> Result<RemoteUpdateJob> {
    ensure_loaded().await?;
    let (job, plan, pruned_jobs) = {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        if g.persisted.jobs.iter().any(|job| !job.status.terminal()) {
            anyhow::bail!("已有服务端更新任务正在执行");
        }
        let plan = g
            .plans
            .remove(plan_id)
            .ok_or_else(|| anyhow::anyhow!("安装计划不存在、已使用或已过期"))?;
        let expires_at = plan
            .public
            .expires_at
            .as_deref()
            .and_then(|stamp| stamp.parse::<chrono::DateTime<Utc>>().ok())
            .ok_or_else(|| anyhow::anyhow!("安装计划不可执行"))?;
        if expires_at <= Utc::now() || plan.public.server_instance_id != server_instance_id() {
            anyhow::bail!("安装计划已过期，请重新检查");
        }
        if plan.public.capability != RemoteUpdateCapability::Automatic {
            anyhow::bail!("当前安装方式不支持远程自动更新");
        }
        // Execution-layer Docker guard. A stale/malformed plan must never turn
        // a container deployment into an in-container binary replacement.
        if matches!(
            crate::source_detector::detect_install_source(),
            InstallSource::Docker
        ) {
            anyhow::bail!("Docker 部署禁止在容器内替换二进制；请重新拉取镜像");
        }
        let stamp = now();
        let job = RemoteUpdateJob {
            job_id: format!("remote_{}", uuid::Uuid::new_v4().simple()),
            from_version: plan.public.current_version.clone(),
            target_version: plan.public.target_version.clone(),
            path: plan.public.path,
            status: RemoteJobStatus::Running,
            phase: "queued".into(),
            percent: Some(0),
            created_at: stamp.clone(),
            updated_at: stamp,
            error: None,
        };
        g.persisted.jobs.push(job.clone());
        let pruned_jobs = if g.persisted.jobs.len() > MAX_JOBS {
            let drain = g.persisted.jobs.len() - MAX_JOBS;
            g.persisted.jobs.drain(0..drain).collect()
        } else {
            Vec::new()
        };
        (job, plan, pruned_jobs)
    };
    if let Err(error) = persist().await {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        g.persisted.jobs.retain(|item| item.job_id != job.job_id);
        if !pruned_jobs.is_empty() {
            let mut restored = pruned_jobs;
            restored.append(&mut g.persisted.jobs);
            g.persisted.jobs = restored;
        }
        g.plans.insert(plan_id.to_string(), plan.clone());
        return Err(error);
    }
    let returned = job.clone();
    tokio::spawn(async move {
        execute(job, plan).await;
    });
    Ok(returned)
}

async fn execute(job: RemoteUpdateJob, plan: PendingPlan) {
    let result: Result<()> = async {
        match plan.public.path {
            RecommendedPath::SelfContained => {
                crate::self_contained::install(
                    &job.job_id,
                    Some(&job.target_version),
                    Some(plan.manifest),
                )
                .await?;
                // The service manager normally terminates us before this line.
                // If it returns first (or no managed service exists), keep the
                // durable state at awaiting_restart; only the new process may
                // claim success after its version matches the target.
                return Ok(());
            }
            RecommendedPath::PackageManager => {
                crate::self_contained::emit_phase(
                    &job.job_id,
                    crate::self_contained::Phase::Downloading,
                );
                let source = plan.source;
                let outcome = ha_core::blocking::run_blocking(move || {
                    crate::package_manager::upgrade(&source)
                })
                .await?;
                if !outcome.success {
                    anyhow::bail!(
                        "package manager upgrade failed: {}\n{}",
                        outcome.command,
                        ha_core::truncate_utf8(&outcome.stderr, 1024)
                    );
                }
                mark_restart_pending(&job.job_id).await?;
                crate::self_contained::emit_phase(
                    &job.job_id,
                    crate::self_contained::Phase::Restarting,
                );
                crate::service_control::restart_service()?;
                return Ok(());
            }
            _ => anyhow::bail!("安装计划的更新路径不可远程执行"),
        }
    }
    .await;

    match result {
        Ok(()) => {}
        Err(error) => {
            app_warn!(
                "self_update",
                "remote_install",
                "remote update job {} failed: {error:#}",
                job.job_id
            );
            finish_job(
                &job.job_id,
                RemoteJobStatus::Failed,
                Some(error.to_string()),
            )
            .await;
        }
    }
}

pub async fn mark_restart_pending(job_id: &str) -> Result<()> {
    {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(job) = g.persisted.jobs.iter_mut().find(|job| job.job_id == job_id) {
            job.status = RemoteJobStatus::AwaitingRestart;
            job.phase = "restarting".into();
            job.updated_at = now();
        } else {
            return Ok(());
        }
    }
    persist().await
}

async fn finish_job(job_id: &str, status: RemoteJobStatus, error: Option<String>) {
    let snapshot = {
        let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
        let Some(job) = g.persisted.jobs.iter_mut().find(|job| job.job_id == job_id) else {
            return;
        };
        job.status = status;
        job.phase = if status == RemoteJobStatus::Succeeded {
            "done".into()
        } else {
            "failed".into()
        };
        job.percent = (status == RemoteJobStatus::Succeeded).then_some(100);
        job.error = error.clone();
        job.updated_at = now();
        job.clone()
    };
    if let Err(error) = persist().await {
        app_error!(
            "self_update",
            "remote_install",
            "failed to persist terminal state for {}: {error:#}",
            job_id
        );
    }
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "app_update:completed",
            serde_json::json!({
                "job_id": job_id,
                "status": status,
                "error": error,
                "targetVersion": snapshot.target_version,
                "remote": true,
            }),
        );
    }
}

pub fn record_progress_sync(job_id: &str, phase: &str, percent: Option<u32>) {
    let mut g = runtime().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(job) = g.persisted.jobs.iter_mut().find(|job| job.job_id == job_id) {
        job.phase = phase.into();
        job.percent = percent.or(job.percent);
        job.updated_at = now();
    }
}

pub async fn job_status(job_id: &str) -> Result<Option<RemoteUpdateJob>> {
    ensure_loaded().await?;
    let g = runtime().lock().unwrap_or_else(|p| p.into_inner());
    Ok(g.persisted
        .jobs
        .iter()
        .find(|job| job.job_id == job_id)
        .cloned())
}
